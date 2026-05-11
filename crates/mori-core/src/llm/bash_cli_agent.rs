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

/// 各 AI CLI 的呼叫協定差異。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliProtocol {
    /// `claude --print --no-session-persistence --allowedTools ... --system-prompt ...`
    Claude,
    /// `gemini -p "" --yolo --output-format text`，system prompt 嵌 stdin 頂部
    Gemini,
    /// `codex exec --dangerously-bypass-approvals-and-sandbox`，system prompt 嵌 stdin 頂部
    Codex,
    /// `gemini -p "" --output-format text`(省略 `--yolo`)— chat-only。
    /// non-TTY 下無法核准 tool 執行 → 實質只輸出文字,不 dispatch shell tool。
    GeminiChat,
    /// `codex exec`(省略 `--dangerously-bypass-approvals-and-sandbox`)— chat-only。
    /// 純文字任務 codex 不會嘗試執行 shell 命令,且 non-TTY 下也無法取得核准。
    CodexChat,
}

impl CliProtocol {
    /// 從 binary 檔名自動偵測協定。
    fn detect(binary: &str) -> Self {
        let name = std::path::Path::new(binary)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(binary);
        match name {
            "gemini" => Self::Gemini,
            "codex" => Self::Codex,
            _ => Self::Claude,
        }
    }
}

pub struct BashCliAgentProvider {
    /// agent CLI binary("claude" / "gemini" / "codex" / 自訂)
    binary: String,
    /// mori CLI binary 路徑(絕對 path 比較穩)
    mori_cli_path: PathBuf,
    /// `--model` 可選 override
    model: Option<String>,
    /// mori binary 的檔名(claude allowedTools 白名單用)
    mori_basename: String,
    /// 從 binary 名稱自動偵測的呼叫協定
    protocol: CliProtocol,
}

impl BashCliAgentProvider {
    pub const DEFAULT_BINARY: &'static str = "claude";
    pub const DEFAULT_MORI_CLI: &'static str = "mori";

    pub fn new(
        binary: impl Into<String>,
        mori_cli_path: PathBuf,
        model: Option<String>,
    ) -> Self {
        let binary = binary.into();
        let protocol = CliProtocol::detect(&binary);
        let mori_basename = mori_cli_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("mori")
            .to_string();
        Self {
            binary,
            mori_cli_path,
            model,
            mori_basename,
            protocol,
        }
    }

    /// Chat-only 變體(gemini-cli / codex-cli)用 — 由呼叫端顯式指定 protocol,
    /// 不靠 binary 名稱自動偵測。mori_cli_path 不使用(PATH 不注入,system prompt
    /// 也不帶 mori CLI 說明),傳 `PathBuf::from("mori")` 做 dummy 即可。
    pub fn new_with_protocol(
        binary: impl Into<String>,
        mori_cli_path: PathBuf,
        model: Option<String>,
        protocol: CliProtocol,
    ) -> Self {
        let binary = binary.into();
        let mori_basename = mori_cli_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("mori")
            .to_string();
        Self {
            binary,
            mori_cli_path,
            model,
            mori_basename,
            protocol,
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
        // chat-only 變體不透過 mori CLI dispatch,system prompt 簡化成純對話規則。
        if matches!(self.protocol, CliProtocol::GeminiChat | CliProtocol::CodexChat) {
            return "你是 Mori — 使用者的個人 AI 管家精靈,繁體中文為主、不客套。\n\
                    直接輸出結果,禁止前言(「我來幫你」「以下是」「好的」等),\
                    禁止尾綴補充說明,禁止執行任何 shell 命令。"
                .into();
        }
        format!(
            "你是 Mori — 使用者的個人 AI 管家精靈,繁體中文為主、不客套、不用 Markdown 標題。\n\
             \n\
             ## 你有一個 `{cli}` CLI 可以透過 Bash 工具呼叫\n\
             \n\
             用它來 dispatch Mori 的技能（包含內建 LLM 技能、動作技能、使用者自訂的 shell 技能）。\n\
             技能會根據使用者當前選的 Agent profile 動態變化,不要假設只有特定幾個。\n\
             \n\
             ## 第一步：先看有哪些技能\n\
             ```\n\
             {cli} skill list\n\
             ```\n\
             這會回傳 JSON,含每個 skill 的 name / description / parameters schema。\n\
             根據 parameters 構造正確 JSON args 再呼叫。\n\
             \n\
             ## 兩種呼叫方式（任選）\n\
             \n\
             【A】內建 LLM skill 有 typed args（人類也方便用）:\n\
             ```\n\
             {cli} skill translate   --text \"你好\" --target en\n\
             {cli} skill polish      --text \"...\" --tone formal\n\
             {cli} skill summarize   --text \"...\" --style bullet_points\n\
             {cli} skill compose     --kind email --topic \"...\" --audience \"...\"\n\
             {cli} skill remember    --title \"...\" --content \"...\" --category preference\n\
             {cli} skill recall-memory --id \"<memory-id>\"\n\
             ```\n\
             \n\
             【B】通用 dispatch（**動作技能 / shell 技能必須用這個**）:\n\
             ```\n\
             {cli} skill call open_url --args '{{\"url\":\"https://example.com\"}}'\n\
             {cli} skill call open_app --args '{{\"app\":\"Firefox\"}}'\n\
             {cli} skill call gh_pr_list                              # 沒參數時 --args 可省\n\
             {cli} skill call ssh_to --args '{{\"host\":\"dev01\"}}'\n\
             ```\n\
             不確定 args schema 時先 `{cli} skill list` 看完整定義。\n\
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
        match self.protocol {
            CliProtocol::Claude     => "bash-cli-agent",
            CliProtocol::Gemini     => "gemini-bash",
            CliProtocol::Codex      => "codex-bash",
            CliProtocol::GeminiChat => "gemini-cli",
            CliProtocol::CodexChat  => "codex-cli",
        }
    }

    fn model(&self) -> &str {
        self.model.as_deref().unwrap_or("(agent CLI default)")
    }

    fn supports_tool_calling(&self) -> bool {
        // Agent 變體(Claude/Gemini/Codex):內部有 Bash tool loop → 可當主 agent。
        // Chat-only 變體(GeminiChat/CodexChat):純文字 in/out → 只能當 skill 內部 LLM。
        !matches!(self.protocol, CliProtocol::GeminiChat | CliProtocol::CodexChat)
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        _tools: Vec<ToolDefinition>,
    ) -> Result<ChatResponse> {
        // Tools 列表故意忽略 — 我們把 dispatch 的決策外包給 CLI,Mori 內部
        // 看到的是 single-shot chat。CLI 收 system prompt 知道有 mori CLI 可用。
        //
        let cli_instructions = self.system_prompt();
        let system_prompt = merge_upstream_system(&messages, &cli_instructions);
        let transcript = format_transcript(&messages);

        // PATH 注入:讓子程序能找到 mori CLI binary。
        let extra_path = self
            .mori_cli_path
            .parent()
            .map(|p| p.to_path_buf())
            .filter(|p| !p.as_os_str().is_empty());
        let patched_path = extra_path.map(|extra| {
            let cur = std::env::var("PATH").unwrap_or_default();
            if cur.is_empty() {
                extra.to_string_lossy().into_owned()
            } else {
                format!("{}:{}", extra.display(), cur)
            }
        });

        let (mut cmd, stdin_bytes, suppress_stderr) = match self.protocol {
            CliProtocol::Claude => {
                let allowed_tools = format!("Bash({} *)", self.mori_basename);
                let mut c = Command::new(&self.binary);
                c.arg("--print")
                    .arg("--no-session-persistence")
                    .arg("--allowedTools").arg(&allowed_tools)
                    .arg("--system-prompt").arg(&system_prompt);
                if let Some(model) = &self.model {
                    c.arg("--model").arg(model);
                }
                (c, transcript.into_bytes(), false)
            }
            CliProtocol::Gemini => {
                // system prompt 嵌進 stdin 頂部;YOLO 警告走 stderr → 丟掉。
                let stdin_content = format_stdin_with_system(&system_prompt, &transcript);
                let mut c = Command::new(&self.binary);
                c.arg("-p").arg("")
                    .arg("--yolo")
                    .arg("--output-format").arg("text");
                if let Some(model) = &self.model {
                    c.arg("--model").arg(model);
                }
                (c, stdin_content.into_bytes(), true)
            }
            CliProtocol::Codex => {
                // codex 走 `codex exec` subcommand；system prompt 嵌進 stdin 頂部。
                let stdin_content = format_stdin_with_system(&system_prompt, &transcript);
                let mut c = Command::new(&self.binary);
                c.arg("exec")
                    .arg("--dangerously-bypass-approvals-and-sandbox");
                if let Some(model) = &self.model {
                    c.arg("--model").arg(model);
                }
                (c, stdin_content.into_bytes(), false)
            }
            CliProtocol::GeminiChat => {
                // chat-only:省略 --yolo → gemini 不自動執行 tool;
                // non-TTY 下無法取得使用者核准 → 實質只輸出文字。
                let stdin_content = format_stdin_with_system(&system_prompt, &transcript);
                let mut c = Command::new(&self.binary);
                c.arg("-p").arg("")
                    .arg("--output-format").arg("text");
                if let Some(model) = &self.model {
                    c.arg("--model").arg(model);
                }
                (c, stdin_content.into_bytes(), true)
            }
            CliProtocol::CodexChat => {
                // chat-only:省略 --dangerously-bypass-approvals-and-sandbox →
                // tool 執行需手動核准;non-TTY 下核准不可得 → 純文字任務實質 chat-only。
                let stdin_content = format_stdin_with_system(&system_prompt, &transcript);
                let mut c = Command::new(&self.binary);
                c.arg("exec");
                if let Some(model) = &self.model {
                    c.arg("--model").arg(model);
                }
                (c, stdin_content.into_bytes(), false)
            }
        };

        if let Some(path) = patched_path {
            cmd.env("PATH", path);
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(if suppress_stderr {
                std::process::Stdio::null()
            } else {
                std::process::Stdio::piped()
            });

        tracing::debug!(
            binary = %self.binary,
            protocol = ?self.protocol,
            mori_cli = %self.mori_cli_path.display(),
            stdin_chars = stdin_bytes.len(),
            "bash-cli-agent chat request",
        );

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn `{}`", self.binary))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(&stdin_bytes)
                .await
                .context("write to agent CLI stdin")?;
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
                if stderr.is_empty() { "(stderr suppressed)" } else { stderr.trim() }
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

/// Gemini / Codex 沒有 `--system-prompt` flag，把 system prompt 嵌進 stdin 頂部。
fn format_stdin_with_system(system: &str, transcript: &str) -> String {
    format!("## Instructions\n{system}\n\n{transcript}")
}

/// 5J fix: 上游 (`run_agent_pipeline`) 把 profile body + Rust 注入的 context
/// section（時間 / 視窗 / 剪貼簿 / 反白 / 記憶索引）放在 `messages` 內 role=system
/// 的訊息。`format_transcript` 會 skip 掉 role=system,如果這裡再用 `self.system_prompt()`
/// 直接覆寫 → 上游 context 整個丟失,Mori 不知道現在幾點。
///
/// 解法：拼上游 system + bash-cli 自己的 CLI 使用說明,用 `---` 分隔。
fn merge_upstream_system(messages: &[ChatMessage], cli_instructions: &str) -> String {
    let upstream: String = messages
        .iter()
        .filter(|m| m.role == "system")
        .filter_map(|m| m.content.as_deref())
        .collect::<Vec<_>>()
        .join("\n\n");
    if upstream.trim().is_empty() {
        cli_instructions.to_string()
    } else {
        format!("{upstream}\n\n---\n\n{cli_instructions}")
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
    fn protocol_detected_from_binary() {
        let claude = BashCliAgentProvider::new("claude", PathBuf::from("/tmp/mori"), None);
        assert_eq!(claude.protocol, CliProtocol::Claude);
        assert_eq!(claude.name(), "bash-cli-agent");
        assert!(claude.supports_tool_calling());

        let gemini = BashCliAgentProvider::new("gemini", PathBuf::from("/tmp/mori"), None);
        assert_eq!(gemini.protocol, CliProtocol::Gemini);
        assert_eq!(gemini.name(), "gemini-bash");
        assert!(gemini.supports_tool_calling());

        let codex = BashCliAgentProvider::new("codex", PathBuf::from("/tmp/mori"), None);
        assert_eq!(codex.protocol, CliProtocol::Codex);
        assert_eq!(codex.name(), "codex-bash");
        assert!(codex.supports_tool_calling());
    }

    #[test]
    fn chat_only_variants_via_new_with_protocol() {
        let g = BashCliAgentProvider::new_with_protocol(
            "gemini",
            PathBuf::from("mori"),
            None,
            CliProtocol::GeminiChat,
        );
        assert_eq!(g.name(), "gemini-cli");
        assert!(!g.supports_tool_calling(), "gemini-cli must be chat-only");

        let c = BashCliAgentProvider::new_with_protocol(
            "codex",
            PathBuf::from("mori"),
            None,
            CliProtocol::CodexChat,
        );
        assert_eq!(c.name(), "codex-cli");
        assert!(!c.supports_tool_calling(), "codex-cli must be chat-only");
    }

    #[test]
    fn chat_only_system_prompt_has_no_mori_cli_instructions() {
        let g = BashCliAgentProvider::new_with_protocol(
            "gemini",
            PathBuf::from("mori"),
            None,
            CliProtocol::GeminiChat,
        );
        let sys = g.system_prompt();
        assert!(!sys.contains("mori skill"), "chat-only prompt must not reference mori CLI");
        assert!(sys.contains("Mori"), "should still identify as Mori");

        let c = BashCliAgentProvider::new_with_protocol(
            "codex",
            PathBuf::from("mori"),
            None,
            CliProtocol::CodexChat,
        );
        let sys = c.system_prompt();
        assert!(!sys.contains("mori skill"), "chat-only prompt must not reference mori CLI");
    }

    #[test]
    fn unknown_binary_defaults_to_claude_protocol() {
        let p = BashCliAgentProvider::new("my-custom-ai", PathBuf::from("/tmp/mori"), None);
        assert_eq!(p.protocol, CliProtocol::Claude);
    }

    #[test]
    fn explicit_model_shows_through() {
        let p = BashCliAgentProvider::new("claude", PathBuf::from("/tmp/mori"), Some("opus".into()));
        assert_eq!(p.model(), "opus");
        assert!(p.supports_tool_calling());
    }

    #[test]
    fn mori_basename_extracted() {
        let p = BashCliAgentProvider::new("claude", PathBuf::from("/usr/local/bin/mori-tool"), None);
        assert_eq!(p.mori_basename, "mori-tool");
    }

    #[test]
    fn format_stdin_with_system_prepends_instructions() {
        let result = format_stdin_with_system("你是 Mori", "User: 你好");
        assert!(result.starts_with("## Instructions\n你是 Mori\n\n"));
        assert!(result.ends_with("User: 你好"));
    }

    #[test]
    fn system_prompt_includes_cli_usage() {
        let p = BashCliAgentProvider::new(
            "claude",
            PathBuf::from("/tmp/mori"),
            None,
        );
        let sys = p.system_prompt();
        // 5I 起 system_prompt 同時提兩種呼叫方式
        assert!(sys.contains("mori skill list"));
        assert!(sys.contains("mori skill translate"));
        assert!(sys.contains("mori skill remember"));
        assert!(sys.contains("mori skill recall-memory"));
        assert!(sys.contains("mori skill call"), "5I: generic dispatch must be mentioned for action_skills / shell_skills");
        assert!(sys.contains("禁止在 CLI 結果後面加任何括號說明"));
    }

    #[test]
    fn merge_upstream_system_empty_falls_back_to_cli() {
        // 沒上游 system → 用原 CLI 指令
        let msgs = vec![ChatMessage::user("hi")];
        let merged = merge_upstream_system(&msgs, "CLI_BOILERPLATE");
        assert_eq!(merged, "CLI_BOILERPLATE");
    }

    #[test]
    fn merge_upstream_system_includes_both() {
        // 5J 關鍵保證:上游 system(時間 / context section / profile body)
        // 跟 CLI boilerplate 都要進去
        let msgs = vec![
            ChatMessage::system("時間: 2026-05-12 03:00\nprofile: 你是 Mori"),
            ChatMessage::user("現在幾點?"),
        ];
        let merged = merge_upstream_system(&msgs, "CLI_BOILERPLATE");
        assert!(merged.contains("時間: 2026-05-12 03:00"), "missing upstream context: {merged}");
        assert!(merged.contains("profile: 你是 Mori"), "missing profile body: {merged}");
        assert!(merged.contains("CLI_BOILERPLATE"), "missing CLI instructions: {merged}");
        // 用 --- 分隔
        assert!(merged.contains("\n\n---\n\n"), "missing separator: {merged}");
    }

    #[test]
    fn merge_upstream_system_concatenates_multiple_system_messages() {
        let msgs = vec![
            ChatMessage::system("first"),
            ChatMessage::user("hi"),
            ChatMessage::system("second"),
        ];
        let merged = merge_upstream_system(&msgs, "CLI");
        assert!(merged.contains("first"));
        assert!(merged.contains("second"));
        assert!(merged.contains("CLI"));
    }

    #[test]
    fn merge_upstream_system_ignores_whitespace_only_system() {
        // 空白 system 不該觸發 merge,直接用 CLI
        let msgs = vec![
            ChatMessage::system("   \n  "),
            ChatMessage::user("hi"),
        ];
        let merged = merge_upstream_system(&msgs, "CLI");
        assert_eq!(merged, "CLI");
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

    /// gemini-cli chat-only 真實呼叫。需要 `gemini` 在 PATH 且已登入。
    /// `cargo test -p mori-core --lib -- --ignored integration_gemini_cli` 觸發。
    #[tokio::test]
    #[ignore]
    async fn integration_gemini_cli_real_binary() {
        let p = BashCliAgentProvider::new_with_protocol(
            "gemini",
            PathBuf::from("mori"),
            None,
            CliProtocol::GeminiChat,
        );
        let resp = p
            .chat(
                vec![
                    ChatMessage::system("Answer in one short English word, no punctuation."),
                    ChatMessage::user("What color is grass?"),
                ],
                vec![],
            )
            .await
            .expect("gemini-cli chat should succeed");
        let answer = resp.content.expect("content");
        // system prompt 要求英文,但 chat-only 的繁中 persona 可能仍回中文
        assert!(
            answer.to_lowercase().contains("green") || answer.contains("綠"),
            "expected color answer containing 'green' or '綠', got: {answer:?}"
        );
        assert!(resp.tool_calls.is_empty(), "gemini-cli must not return tool_calls");
    }

    /// codex-cli chat-only 真實呼叫。需要 `codex` 在 PATH 且已登入。
    /// `cargo test -p mori-core --lib -- --ignored integration_codex_cli` 觸發。
    #[tokio::test]
    #[ignore]
    async fn integration_codex_cli_real_binary() {
        let p = BashCliAgentProvider::new_with_protocol(
            "codex",
            PathBuf::from("mori"),
            None,
            CliProtocol::CodexChat,
        );
        let resp = p
            .chat(
                vec![
                    ChatMessage::system("Answer in one short English word, no punctuation."),
                    ChatMessage::user("What color is grass?"),
                ],
                vec![],
            )
            .await
            .expect("codex-cli chat should succeed");
        let answer = resp.content.expect("content");
        assert!(
            answer.to_lowercase().contains("green") || answer.contains("綠"),
            "expected color answer containing 'green' or '綠', got: {answer:?}"
        );
        assert!(resp.tool_calls.is_empty(), "codex-cli must not return tool_calls");
    }
}
