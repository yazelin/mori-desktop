//! Phase 5D — Bash CLI proxy agent provider。
//!
//! 把 `claude` / `codex` / `gemini` 等 AI CLI 當主 agent loop 用,但**不**
//! 透過 MCP(token 重)也**不**透過各家自家 tool channel(每家不一樣),而是
//! 把 Mori 的能力透過一個本機 `mori` CLI binary 暴露出去 —— LLM 用它們的
//! Bash tool 直接執行 `mori skill translate ...` 即可 dispatch。
//!
//! ## Token 帳
//! - MCP:每輪 prompt 載入全部 tools 的 schema,Mori 10 個 skill 估計 1-2K
//!   tokens 預載
//! - Bash CLI:system prompt 提一句「你有個 `mori` CLI,跑 `mori skill list`
//!   看能用什麼」 ~150 tokens,實際用到才 `mori skill X --help` 或直接執行
//!
//! ## 為什麼能跨 CLI
//! claude / codex / gemini 都有 Bash(或 shell)tool。所以「LLM 透過 shell
//! 跑外部 CLI」是它們的共同最大公因數,不必為每家寫不同的 binding。
//!
//! ## supports_tool_calling = true
//! 表面上這個 provider 收到 `tools` 參數會忽略(Mori 的 agent loop 從外部
//! 看是 single-turn — chat() 一次 round-trip),但**實質上** tool dispatch
//! 在 CLI 子程序內部發生(claude/codex/gemini 自己 reason → call Bash → 拿
//! 結果 → 繼續推理)。所以宣告 supports_tool_calling = true 才能當主 agent
//! provider 用。

use std::path::PathBuf;

use anyhow::{bail, Context as _, Result};
use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::{ChatMessage, ChatResponse, LlmProvider, ToolCall, ToolDefinition};

pub struct BashCliAgentProvider {
    /// agent CLI binary("claude" / "codex" / "gemini" / 自訂)
    binary: String,
    /// mori CLI binary 路徑(絕對 path 比較穩),會在 system prompt + allowedTools
    /// 裡 reference。
    mori_cli_path: PathBuf,
    /// `--model` 可選 override(claude 才有意義)
    model: Option<String>,
    /// `--allowedTools` 用的 binary 名稱(從 mori_cli_path 取 file_name)。
    /// claude 把這個塞進 `Bash(mori_basename *)` 做白名單。
    mori_basename: String,
}

impl BashCliAgentProvider {
    pub const DEFAULT_BINARY: &'static str = "claude";
    pub const DEFAULT_MORI_CLI: &'static str = "mori";

    pub fn new(
        binary: impl Into<String>,
        mori_cli_path: PathBuf,
        model: Option<String>,
    ) -> Self {
        let mori_basename = mori_cli_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("mori")
            .to_string();
        Self {
            binary: binary.into(),
            mori_cli_path,
            model,
            mori_basename,
        }
    }

    /// 嘗試自動找 mori CLI:先看 `current_exe()` 旁邊(dev:`target/debug/mori`),
    /// 找不到 fallback 到 PATH 上的 `mori`。
    pub fn detect_mori_cli() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let candidate = parent.join("mori");
                if candidate.exists() {
                    return candidate;
                }
            }
        }
        PathBuf::from(Self::DEFAULT_MORI_CLI)
    }

    fn system_prompt(&self) -> String {
        format!(
            "你是 Mori — 使用者的個人 AI 管家精靈,繁體中文為主、不客套、不用 Markdown 標題。\n\
             \n\
             ## 你有一個 `{cli}` CLI 可以透過 Bash 工具呼叫\n\
             \n\
             用它來 dispatch Mori 的內建技能,不要自己做 — 一律走這個 CLI,Mori 的版本會跟使用者偏好對齊。\n\
             \n\
             查詢可用技能:\n\
             ```\n\
             {cli} skill list\n\
             ```\n\
             \n\
             常用呼叫範例:\n\
             ```\n\
             {cli} skill translate   --text \"你好\" --target en\n\
             {cli} skill polish      --text \"...\" --tone formal\n\
             {cli} skill summarize   --text \"...\" --style bullet_points\n\
             {cli} skill compose     --kind email --topic \"...\" --audience \"...\"\n\
             {cli} skill remember    --title \"...\" --content \"...\" --category preference\n\
             {cli} skill recall-memory  --id \"<memory-id>\"\n\
             {cli} skill forget-memory  --id \"<memory-id>\"\n\
             {cli} skill edit-memory    --id \"<memory-id>\" --content \"...\"\n\
             ```\n\
             \n\
             不確定參數時跑 `{cli} skill <name> --help`。\n\
             \n\
             ## 回應規則(嚴格遵守)\n\
             - **CLI 的 stdout 就是你給使用者的完整回應。原樣輸出,一字不改。**\n\
             - 禁止在 CLI 結果後面加任何括號說明、補充、解釋或評語。\n\
             - 禁止前言(「我來幫你翻譯」「以下是」「好的」等)。\n\
             - 禁止把 CLI 指令本身貼出來。\n\
             - 一般閒聊不呼叫 CLI,直接回。\n\
             - 對話歷史在後面附上。",
            cli = self.mori_basename,
        )
    }
}

#[async_trait]
impl LlmProvider for BashCliAgentProvider {
    fn name(&self) -> &'static str {
        // 對外回的 name 是固定的(LlmProvider trait 要 &'static),實際 binary 在 log 用 binary 欄位露出。
        "bash-cli-agent"
    }

    fn model(&self) -> &str {
        self.model.as_deref().unwrap_or("(agent CLI default)")
    }

    fn supports_tool_calling(&self) -> bool {
        // 假性 true:Mori agent loop 一輪 round-trip 結束;但 CLI 內部會做
        // 真正的 reasoning + tool dispatch,所以可以當主 agent provider。
        true
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        _tools: Vec<ToolDefinition>,
    ) -> Result<ChatResponse> {
        // Tools 列表故意忽略 — 我們把 dispatch 的決策外包給 CLI,Mori 內部
        // 看到的是 single-shot chat。CLI 收 system prompt 知道有 mori CLI 可用。
        let transcript = format_transcript(&messages);
        let system_prompt = self.system_prompt();

        let allowed_tools = format!("Bash({} *)", self.mori_basename);
        let mut cmd = Command::new(&self.binary);
        cmd.arg("--print")
            .arg("--no-session-persistence")
            .arg("--allowedTools")
            .arg(&allowed_tools)
            .arg("--system-prompt")
            .arg(&system_prompt);
        if let Some(model) = &self.model {
            cmd.arg("--model").arg(model);
        }
        // mori CLI 需要在 PATH 找得到,或者使用絕對路徑被 Bash 直接執行。
        // 讓 claude 子程序繼承我們的 PATH,並補上 mori-cli 所在 dir。
        let extra_path = self
            .mori_cli_path
            .parent()
            .map(|p| p.to_path_buf())
            .filter(|p| !p.as_os_str().is_empty());
        if let Some(extra) = extra_path {
            let cur = std::env::var("PATH").unwrap_or_default();
            let new_path = if cur.is_empty() {
                extra.to_string_lossy().into_owned()
            } else {
                format!("{}:{}", extra.display(), cur)
            };
            cmd.env("PATH", new_path);
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        tracing::debug!(
            binary = %self.binary,
            mori_cli = %self.mori_cli_path.display(),
            allowed_tools = %allowed_tools,
            transcript_chars = transcript.len(),
            "bash-cli-agent chat request",
        );

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn `{}`", self.binary))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(transcript.as_bytes())
                .await
                .context("write transcript to agent CLI stdin")?;
        }

        let output = child
            .wait_with_output()
            .await
            .context("wait for agent CLI")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "{} CLI failed (exit={}): {}",
                self.binary,
                output.status,
                stderr.trim()
            );
        }

        let response = String::from_utf8(output.stdout)
            .context("agent CLI stdout was not UTF-8")?
            .trim()
            .to_string();

        Ok(ChatResponse {
            content: Some(response),
            tool_calls: Vec::<ToolCall>::new(),
        })
    }
}

/// 把 messages 拍平成 user/assistant 對話 transcript。跟 ClaudeCliProvider
/// 的格式一致 — 不同 CLI 都認得這種 markdown-style turn 表示。
fn format_transcript(messages: &[ChatMessage]) -> String {
    let mut buf = String::new();
    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                // system message 透過 --system-prompt 走另一條路,這裡不重複塞。
                // (避免 LLM 把 system 訊息當作對話內容)
            }
            "user" => {
                if !buf.is_empty() {
                    buf.push_str("\n\n");
                }
                buf.push_str("User: ");
                buf.push_str(msg.content.as_deref().unwrap_or(""));
            }
            "assistant" => {
                if !buf.is_empty() {
                    buf.push_str("\n\n");
                }
                buf.push_str("Assistant: ");
                buf.push_str(msg.content.as_deref().unwrap_or(""));
            }
            "tool" => {
                if !buf.is_empty() {
                    buf.push_str("\n\n");
                }
                buf.push_str("Tool result: ");
                buf.push_str(msg.content.as_deref().unwrap_or(""));
            }
            _ => {}
        }
    }
    if buf.is_empty() {
        buf.push_str("User: ");
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_default_model() {
        let p = BashCliAgentProvider::new(
            "claude",
            PathBuf::from("/tmp/mori"),
            None,
        );
        assert_eq!(p.name(), "bash-cli-agent");
        assert_eq!(p.model(), "(agent CLI default)");
        assert!(p.supports_tool_calling());
    }

    #[test]
    fn explicit_model_shows_through() {
        let p = BashCliAgentProvider::new(
            "claude",
            PathBuf::from("/tmp/mori"),
            Some("opus".into()),
        );
        assert_eq!(p.model(), "opus");
    }

    #[test]
    fn mori_basename_extracted() {
        let p = BashCliAgentProvider::new(
            "claude",
            PathBuf::from("/usr/local/bin/mori-tool"),
            None,
        );
        assert_eq!(p.mori_basename, "mori-tool");
    }

    #[test]
    fn system_prompt_includes_cli_usage() {
        let p = BashCliAgentProvider::new(
            "claude",
            PathBuf::from("/tmp/mori"),
            None,
        );
        let sys = p.system_prompt();
        assert!(sys.contains("mori skill list"));
        assert!(sys.contains("mori skill translate"));
        assert!(sys.contains("mori skill remember"));
        assert!(sys.contains("mori skill recall-memory"));
        assert!(sys.contains("mori skill forget-memory"));
        assert!(sys.contains("mori skill edit-memory"));
        assert!(sys.contains("禁止在 CLI 結果後面加任何括號說明"));
    }

    #[test]
    fn format_transcript_drops_system() {
        // system 透過 --system-prompt 傳,transcript 不該重複
        let msgs = vec![
            ChatMessage::system("you are Mori"),
            ChatMessage::user("hi"),
            ChatMessage::assistant_with_tool_calls(Some("hello!".into()), vec![]),
            ChatMessage::user("translate this"),
        ];
        let t = format_transcript(&msgs);
        assert!(!t.contains("you are Mori"));
        assert!(t.starts_with("User: hi"));
        assert!(t.contains("Assistant: hello!"));
        assert!(t.ends_with("translate this"));
    }
}
