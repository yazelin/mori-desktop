//! Claude CLI subprocess provider — chat-only,**不支援 tool calling**。
//!
//! 形式上把 `claude --print` 當作 LLM endpoint。整個流程是:
//! 1. 把 messages 拆成 (system_prompt, transcript)
//! 2. spawn `claude --print --bare --no-session-persistence \
//!     --system-prompt <sys> [--model <m>]`
//! 3. transcript 透過 stdin 送進去(避開 argv 長度上限)
//! 4. 讀 stdout 當 chat 回應
//!
//! ## 為什麼**不**用 `--bare`
//! `--bare` 看起來是「乾淨單次呼叫」的理想開關,但它把 keychain auth
//! 關掉,只接受 ANTHROPIC_API_KEY env var 或 apiKeyHelper。一般 user
//! 是用 `claude /login` OAuth 進來的(token 在 keychain),`--bare`
//! 模式下會 `Not logged in · Please run /login` 直接 fail(實測 2026-05)。
//!
//! 折衷做法:不開 `--bare`,只開 `--no-session-persistence`,並用
//! `--system-prompt` 覆寫掉 Claude Code 預設的工具型 system prompt
//! → 避免 claude 拿 Bash/Edit/Read 工具描述污染我們的純 chat 場景。
//! 副作用是 claude 仍會掃 CLAUDE.md / 載 auto-memory / 跑 hook。對
//! Mori 的 skill 內部 chat 來說多一點 context 不致命,等 user 真的
//! 嫌它慢/吵再說。
//!
//! ## 為什麼 chat-only
//! Claude CLI 的 tool calling 是 Claude Code 內部的 Bash/Read/Edit 等等
//! 工具,跟 Mori 自己的 SkillRegistry 完全是兩個世界。把 ToolDefinition
//! 餵進 claude CLI 也不會讓它走我們的 skill。所以這個 provider 收到
//! `tools` 直接忽略,只負責純 chat。
//!
//! Mori 的 agent loop 在 round 0 沒拿到 tool_calls 時會 fall through 成
//! 文字回應 — 也就是說用 ClaudeCliProvider 當主 agent provider 會讓
//! tool dispatch 失效。**正確用法**:在 5A-3 routing 裡把 ClaudeCliProvider
//! 指給「skill 內部 chat」用,主 agent 仍走 Groq / Ollama 那種能 tool call
//! 的 provider。
//!
//! ## Config(`~/.mori/config.json`)
//! ```json
//! {
//!   "providers": {
//!     "claude_cli": {
//!       "binary": "claude",
//!       "model": "sonnet"
//!     }
//!   }
//! }
//! ```
//! 兩個欄位都選填 — `binary` 預設靠 PATH 找 `claude`,`model` 預設
//! 由 claude CLI 自己挑(目前是 Sonnet 系列)。

use anyhow::{bail, Context as _, Result};
use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::{ChatMessage, ChatResponse, LlmProvider, ToolDefinition};

pub struct ClaudeCliProvider {
    binary: String,
    /// None = 讓 claude CLI 用預設 model(目前是 Sonnet 4.x),不傳 `--model`
    model: Option<String>,
}

impl ClaudeCliProvider {
    pub const NAME: &'static str = "claude-cli";
    pub const DEFAULT_BINARY: &'static str = "claude";

    pub fn new(binary: impl Into<String>, model: Option<String>) -> Self {
        Self {
            binary: binary.into(),
            model,
        }
    }

    /// 用預設 binary 名字("claude")+ 預設 model(讓 claude CLI 自挑)。
    pub fn with_defaults() -> Self {
        Self::new(Self::DEFAULT_BINARY, None)
    }

    /// 探測 claude CLI 是否可用。跑一次 `<binary> --version`,成功就回
    /// version 字串(像 `"2.1.133 (Claude Code)"`),失敗回 Err 帶上
    /// stderr 解釋。給啟動時 / IPC `chat_provider_info` 用。
    pub async fn probe(&self) -> Result<String> {
        let output = Command::new(&self.binary)
            .arg("--version")
            .output()
            .await
            .with_context(|| format!("spawn `{}` (not in PATH?)", self.binary))?;
        if !output.status.success() {
            bail!(
                "`{} --version` failed: {}",
                self.binary,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

#[async_trait]
impl LlmProvider for ClaudeCliProvider {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn model(&self) -> &str {
        self.model.as_deref().unwrap_or("(claude-cli default)")
    }

    fn supports_tool_calling(&self) -> bool {
        false
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
    ) -> Result<ChatResponse> {
        if !tools.is_empty() {
            tracing::warn!(
                tool_count = tools.len(),
                "claude-cli does not support tool calling — tools ignored. \
                 Use this provider only for skill-internal chat."
            );
        }

        let (system_prompt, transcript) = format_messages(&messages);

        let mut cmd = Command::new(&self.binary);
        cmd.arg("--print").arg("--no-session-persistence");
        if let Some(model) = &self.model {
            cmd.arg("--model").arg(model);
        }
        if let Some(sys) = &system_prompt {
            cmd.arg("--system-prompt").arg(sys);
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        tracing::debug!(
            binary = %self.binary,
            model = ?self.model,
            sys_chars = system_prompt.as_deref().map(|s| s.len()).unwrap_or(0),
            transcript_chars = transcript.len(),
            "claude-cli chat request",
        );

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn `{}` (claude CLI installed?)", self.binary))?;

        // Pipe transcript to stdin。drop stdin → EOF,讓 claude 開始處理。
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(transcript.as_bytes())
                .await
                .context("write prompt to claude CLI stdin")?;
        }

        let output = child
            .wait_with_output()
            .await
            .context("wait for claude CLI")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "claude CLI failed (exit={}): {}",
                output.status,
                stderr.trim()
            );
        }

        let response = String::from_utf8(output.stdout)
            .context("claude CLI stdout was not UTF-8")?
            .trim()
            .to_string();

        Ok(ChatResponse {
            content: Some(response),
            tool_calls: Vec::new(),
        })
    }

    // transcribe → 預設 trait impl 回 "not supported" — Claude CLI 沒 STT。
}

/// 把 ChatMessage 列表壓成 (system_prompt?, transcript)。
///
/// - `system` messages 串成單一字串(用 `\n\n` 連起來),走 `--system-prompt`
///   旗標傳進 claude CLI(claude 也只接受一個)。
/// - `user` / `assistant` / `tool` messages 渲染成簡單的 markdown-style
///   transcript:`User: ...`、`Assistant: ...`、`Tool result: ...`。透過
///   stdin 餵進去。Claude 看到這種對話格式會自然延續。
///
/// 如果整個 transcript 為空(只有 system),最後仍給 claude 一個 user
/// turn(`User: `)當 cue,不然 claude 會卡住等 prompt。
fn format_messages(messages: &[ChatMessage]) -> (Option<String>, String) {
    let mut system: Option<String> = None;
    let mut buf = String::new();

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                let content = msg.content.as_deref().unwrap_or("");
                system = Some(match system.take() {
                    Some(prev) => format!("{}\n\n{}", prev, content),
                    None => content.to_string(),
                });
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
                // tool_calls 在 chat-only provider 沒意義,直接忽略。
            }
            "tool" => {
                // 上層理論上不該餵 tool 結果給我們,但若餵了就盡力 stringify。
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
        // 只有 system 沒 user 訊息 — 給 claude 一個空 user turn 引發回應。
        buf.push_str("User: ");
    }

    (system, buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_default_model() {
        let p = ClaudeCliProvider::with_defaults();
        assert_eq!(p.name(), "claude-cli");
        assert_eq!(p.model(), "(claude-cli default)");
    }

    #[test]
    fn explicit_model_shows_through() {
        let p = ClaudeCliProvider::new("claude", Some("opus".to_string()));
        assert_eq!(p.model(), "opus");
    }

    #[test]
    fn format_separates_system_and_transcript() {
        let msgs = vec![
            ChatMessage::system("you are helpful"),
            ChatMessage::user("hi"),
            ChatMessage::assistant_with_tool_calls(Some("hello!".into()), vec![]),
            ChatMessage::user("translate \"cat\" to japanese"),
        ];
        let (sys, body) = format_messages(&msgs);
        assert_eq!(sys.as_deref(), Some("you are helpful"));
        assert!(body.starts_with("User: hi"));
        assert!(body.contains("Assistant: hello!"));
        assert!(body.ends_with("translate \"cat\" to japanese"));
    }

    #[test]
    fn format_concatenates_multiple_systems() {
        let msgs = vec![
            ChatMessage::system("rule 1"),
            ChatMessage::system("rule 2"),
            ChatMessage::user("go"),
        ];
        let (sys, _) = format_messages(&msgs);
        assert_eq!(sys.as_deref(), Some("rule 1\n\nrule 2"));
    }

    #[test]
    fn format_no_user_emits_cue() {
        // 只有 system 也要餵個 User: 給 claude 不然會卡
        let msgs = vec![ChatMessage::system("you are helpful")];
        let (_, body) = format_messages(&msgs);
        assert_eq!(body, "User: ");
    }

    /// 跑得起來才算 — 需要真的有 `claude` 在 PATH 並完成過登入。
    /// 用 `cargo test -p mori-core --lib -- --ignored claude_cli` 觸發。
    #[tokio::test]
    #[ignore]
    async fn integration_probe_real_binary() {
        let p = ClaudeCliProvider::with_defaults();
        let v = p.probe().await.expect("claude --version should succeed");
        assert!(v.contains("Claude") || v.contains("claude"), "unexpected: {v}");
    }

    /// 真的對 claude CLI 發 chat,要花錢/額度,所以 ignored。
    /// 用 `cargo test -p mori-core --lib -- --ignored integration_chat` 觸發。
    #[tokio::test]
    #[ignore]
    async fn integration_chat_real_binary() {
        let p = ClaudeCliProvider::with_defaults();
        let resp = p
            .chat(
                vec![
                    ChatMessage::system("Answer in one short word, no punctuation."),
                    ChatMessage::user("What color is grass?"),
                ],
                vec![],
            )
            .await
            .expect("chat should succeed");
        let answer = resp.content.expect("content").to_lowercase();
        assert!(
            answer.contains("green"),
            "expected color answer to contain 'green', got: {answer:?}"
        );
        assert!(resp.tool_calls.is_empty(), "claude-cli must not return tool_calls");
    }
}
