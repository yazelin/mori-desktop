//! Groq LLM provider — chat completion + Whisper transcription。
//!
//! 接 OpenAI 相容 API:
//! - `POST /chat/completions` for chat
//! - `POST /audio/transcriptions` for Whisper

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use reqwest::{multipart, StatusCode};
use serde::{Deserialize, Serialize};

use super::transcribe::TranscriptionProvider;
use super::{ChatMessage, ChatResponse, LlmProvider, ToolCall, ToolDefinition};

/// Retry / 限流發生時推給觀察者的事件。可序列化方便給 UI emit。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryEvent {
    pub attempt: usize,
    pub max_attempts: usize,
    pub wait_secs: u64,
    /// "rate_limit" / "server_error" / "network"
    pub reason: String,
    /// 觸發 retry 的具體 endpoint(chat / transcribe)
    pub op: String,
}

pub type RetryCallback = Arc<dyn Fn(RetryEvent) + Send + Sync>;

const MAX_ATTEMPTS: usize = 5;
/// 第 N 次失敗等待秒數(N=1..=MAX_ATTEMPTS-1)— 1, 2, 4, 8, 16
const BACKOFF_SECS: [u64; 5] = [1, 2, 4, 8, 16];

/// 看 429 / 5xx 之後該幹嘛。
#[derive(Debug, PartialEq, Eq)]
enum RetryDecision {
    /// 等 wait_secs 秒後重試。
    Retry { wait_secs: u64 },
    /// 等的時間太長(> MAX_AUTOMATIC_RETRY_SECS),立刻 surface 給使用者。
    /// `wait_secs` 是 server 告訴我們的真實等待時間,給 UI 顯示用。
    Surface { wait_secs: u64 },
}

pub struct GroqProvider {
    api_key: String,
    model: String,
    transcribe_model: String,
    base_url: String,
    client: reqwest::Client,
    retry_callback: Option<RetryCallback>,
}

impl GroqProvider {
    pub const DEFAULT_BASE_URL: &'static str = "https://api.groq.com/openai/v1";
    pub const DEFAULT_CHAT_MODEL: &'static str = "openai/gpt-oss-120b";
    pub const DEFAULT_TRANSCRIBE_MODEL: &'static str = "whisper-large-v3-turbo";

    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            transcribe_model: Self::DEFAULT_TRANSCRIBE_MODEL.to_string(),
            base_url: Self::DEFAULT_BASE_URL.to_string(),
            client: reqwest::Client::new(),
            retry_callback: None,
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_transcribe_model(mut self, model: impl Into<String>) -> Self {
        self.transcribe_model = model.into();
        self
    }

    /// 設定 retry 回呼。每次因 429 / 5xx 等待重試前會呼叫一次,
    /// 讓上層(例如 mori-tauri)有機會 emit UI event 通知使用者。
    pub fn with_retry_callback(mut self, cb: RetryCallback) -> Self {
        self.retry_callback = Some(cb);
        self
    }

    /// 該不該對這個 status 重試
    fn is_retriable(status: StatusCode) -> bool {
        status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
    }

    /// 從 `Retry-After` header 抽秒數(整數秒)
    fn parse_retry_after_header(resp: &reqwest::Response) -> Option<u64> {
        resp.headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
    }

    /// 從 Groq 429 response body 的 error message 抽「try again in X」秒數。
    ///
    /// Groq 訊息格式至少有兩種:
    ///   "Please try again in 4.5s."          (TPM,純秒)
    ///   "Please try again in 12m12.24s."     (TPD,分+秒)
    ///   "Please try again in 1h30m."         (理論上,沒實測)
    ///
    /// 全部 case 都要回**正確的總秒數**。沒有單位(舊測試 "12.345" 結尾)
    /// 時當純秒數,維持向後相容。
    fn parse_retry_after_body(body: &str) -> Option<u64> {
        let lower = body.to_lowercase();
        let i = lower.find("try again in ")?;
        let rest = &body[i + "try again in ".len()..];

        let mut total = 0.0_f64;
        let mut current = String::new();
        let mut saw_unit = false;

        for ch in rest.chars() {
            if ch.is_ascii_digit() || ch == '.' {
                current.push(ch);
            } else if matches!(ch, 'h' | 'm' | 's') {
                let n: f64 = current.parse().unwrap_or(0.0);
                let factor = match ch {
                    'h' => 3600.0,
                    'm' => 60.0,
                    's' => 1.0,
                    _ => unreachable!(),
                };
                total += n * factor;
                current.clear();
                saw_unit = true;
                if ch == 's' {
                    break; // 's' 是最末單位,後面通常是「.」或空白
                }
            } else if ch.is_whitespace() {
                continue; // 「12m 12s」式留白允許
            } else {
                break;
            }
        }

        if !saw_unit {
            // 沒任何單位 → 當純秒數(向後相容舊解析行為)
            let n: f64 = current.parse().ok()?;
            if !(n.is_finite() && n >= 0.0) {
                return None;
            }
            return Some(n.ceil() as u64);
        }

        if !(total.is_finite() && total >= 0.0) {
            return None;
        }
        Some(total.ceil() as u64)
    }

    /// 上限:超過這個秒數就**不自動重試**,改成立刻 surface error 給使用者。
    /// 60s 是體驗門檻 — 等更久不如告訴使用者「等不下去這麼久」並讓他決定
    /// (例如 TPD 用完要等 12 分鐘,顯然不該卡著等)。
    const MAX_AUTOMATIC_RETRY_SECS: u64 = 60;

    /// 看 429 / 5xx 該怎辦的決策。
    fn decide_retry(
        body: &str,
        header_secs: Option<u64>,
        fallback_secs: u64,
    ) -> RetryDecision {
        let base = Self::parse_retry_after_body(body)
            .or(header_secs)
            .unwrap_or(fallback_secs);
        if base > Self::MAX_AUTOMATIC_RETRY_SECS {
            RetryDecision::Surface { wait_secs: base }
        } else {
            RetryDecision::Retry {
                wait_secs: (base + 1).clamp(1, Self::MAX_AUTOMATIC_RETRY_SECS),
            }
        }
    }

    /// 從 Groq 錯誤 body 萃取 friendly 提示給 UI 用,比原始 JSON 好讀。
    fn friendly_rate_limit_hint(body: &str, parsed_wait: Option<u64>) -> String {
        let lower = body.to_lowercase();
        let limit = if lower.contains("tokens per day") || lower.contains("(tpd)") {
            "今日 token 用完(TPD)"
        } else if lower.contains("tokens per minute") || lower.contains("(tpm)") {
            "本分鐘 token 用完(TPM)"
        } else if lower.contains("requests per minute") || lower.contains("(rpm)") {
            "本分鐘請求次數用完(RPM)"
        } else {
            "Groq 限流"
        };
        match parsed_wait {
            Some(s) if s >= 60 => format!(
                "{limit} — 需等約 {} 分鐘(超過自動重試上限)。\
                 升級 Dev Tier 或晚點再試。",
                (s + 59) / 60
            ),
            Some(s) => format!("{limit} — 需等 {s}s"),
            None => limit.to_string(),
        }
    }

    /// 通報並等待。在最後一次 attempt 之後不再呼叫。
    async fn notify_and_wait(
        &self,
        op: &str,
        attempt: usize,
        wait_secs: u64,
        reason: &str,
    ) {
        tracing::warn!(
            op,
            attempt,
            max = MAX_ATTEMPTS,
            wait_secs,
            reason,
            "groq: backing off + retrying"
        );
        if let Some(cb) = &self.retry_callback {
            cb(RetryEvent {
                attempt,
                max_attempts: MAX_ATTEMPTS,
                wait_secs,
                reason: reason.to_string(),
                op: op.to_string(),
            });
        }
        tokio::time::sleep(Duration::from_secs(wait_secs)).await;
    }

    /// 嘗試從以下來源依序取得 GROQ_API_KEY:
    /// 1. `GROQ_API_KEY` 環境變數
    /// 2. `~/.mori/config.json` 的 `providers.groq.api_key`
    pub fn discover_api_key() -> Option<String> {
        if let Ok(key) = std::env::var("GROQ_API_KEY") {
            if !key.is_empty() && !is_placeholder(&key) {
                return Some(key);
            }
        }

        let home = home_dir()?;
        read_json_pointer(
            &home.join(".mori").join("config.json"),
            "/providers/groq/api_key",
        )
    }

    /// 確保 `~/.mori/` 存在,若 `config.json` 不存在就寫一份 stub
    /// (含 placeholder + 所有可用 provider 的 default 欄位,使用者編輯
    /// 一次後就可用)。
    ///
    /// **不會覆寫**已存在的 config — 既有 user 的設定保留,新欄位若缺
    /// 由各 provider 的 default 填補(`OllamaProvider::DEFAULT_*` 等)。
    pub fn bootstrap_mori_config() -> anyhow::Result<std::path::PathBuf> {
        let home = home_dir()
            .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
        let dir = home.join(".mori");
        std::fs::create_dir_all(&dir)?;

        let config = dir.join("config.json");
        if !config.exists() {
            let stub = serde_json::json!({
                // 哪個 provider 服務 chat / 主 agent loop。
                // 接受值:"groq"(雲端 Groq Whisper + GPT-OSS 系列)
                //         "ollama"(本機 Ollama,需先 `ollama serve`)
                //         "claude-cli"(本機 claude CLI subprocess,**chat-only**,
                //                       不能當主 agent provider — 沒 tool calling)
                "default_provider": "groq",
                // 5C:STT 跟 chat 解耦。STT provider 獨立配置,可以
                // 「STT 走 Groq Whisper、chat 走 ollama」或反過來,
                // 或兩邊都本機(100% Groq-free)。
                // 接受值:"groq"(預設,Whisper API)
                //         "whisper-local"(whisper.cpp,需事先下載 ggml model)
                "default_transcribe_provider": "groq",
                // 5A-3:per-skill provider routing(可選)。沒設這塊就全部
                // 用 default_provider — 跟 5A-2 之前一樣。
                //
                // 用法:agent 指主 agent loop 走的 provider(必須能 tool calling
                // — 不要用 claude-cli);skills.<name> 指該 skill 內部 chat
                // 走的 provider,沒列到的 skill 退回 agent。
                //
                // 範例:agent 走 Groq tool dispatch、translate/polish/summarize
                // 走 user 自己的 Claude Pro/Max quota,compose 走本機 ollama:
                //   "routing": {
                //     "agent": "groq",
                //     "skills": {
                //       "translate": "claude-cli",
                //       "polish":    "claude-cli",
                //       "summarize": "claude-cli",
                //       "compose":   "ollama"
                //     }
                //   }
                "routing": {
                    "agent": null,
                    "skills": {}
                },
                "providers": {
                    "groq": {
                        "api_key": "REPLACE_ME_WITH_YOUR_GROQ_API_KEY",
                        "chat_model": super::groq::GroqProvider::DEFAULT_CHAT_MODEL,
                        "transcribe_model": super::groq::GroqProvider::DEFAULT_TRANSCRIBE_MODEL
                    },
                    "ollama": {
                        "base_url": super::ollama::OllamaProvider::DEFAULT_BASE_URL,
                        "model":    super::ollama::OllamaProvider::DEFAULT_MODEL
                    },
                    "claude-cli": {
                        // PATH 上的 binary 名稱;絕對路徑也 OK
                        "binary": super::claude_cli::ClaudeCliProvider::DEFAULT_BINARY,
                        // null = 讓 claude CLI 用預設 model;指定值如 "sonnet" / "opus" / 完整 model id
                        "model": null
                    },
                    "whisper-local": {
                        // ggml `.bin` model file。要先從 huggingface 抓:
                        //   wget -O ~/.mori/models/ggml-small.bin \
                        //     https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin
                        // 中文場景建議 small(466MB);CPU 慢可以用 base(142MB)。
                        "model_path": super::whisper_local::default_model_path()
                            .to_string_lossy()
                            .into_owned(),
                        // null / "auto" = whisper 自偵測;也可寫 "zh" / "en" 等。
                        "language": "zh"
                    }
                }
            });
            std::fs::write(&config, serde_json::to_string_pretty(&stub)?)?;
            tracing::info!(path = %config.display(), "bootstrapped ~/.mori/config.json");
        }
        Ok(config)
    }
}

fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(std::path::PathBuf::from)
}

fn is_placeholder(s: &str) -> bool {
    let upper = s.to_uppercase();
    upper.starts_with("REPLACE") || upper.contains("YOUR_GROQ") || upper == "TODO"
}

pub(crate) fn read_json_pointer(path: &std::path::Path, pointer: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let key = json.pointer(pointer)?.as_str()?;
    if key.is_empty() || is_placeholder(key) {
        return None;
    }
    Some(key.to_string())
}

// ─── chat completion wire types — shared with other OpenAI-compat
// providers(Ollama, OpenAI, OpenRouter, ...)— see super::openai_compat.
use super::openai_compat::{
    ChatRequest, ChatResponseWire, WireFunction, WireFunctionOut, WireMessage, WireTool,
    WireToolCallOut,
};

// ─── transcription wire(Groq-specific)──────────────────────────────

#[derive(Deserialize, Debug)]
struct TranscriptionResponse {
    text: String,
}

#[async_trait]
impl LlmProvider for GroqProvider {
    fn name(&self) -> &'static str {
        "groq"
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
    ) -> Result<ChatResponse> {
        // ChatMessage → WireMessage(處理 tool_calls 巢狀格式)
        let wire_messages: Vec<WireMessage> = messages
            .iter()
            .map(|m| WireMessage {
                role: &m.role,
                content: m.content.as_deref(),
                tool_calls: m
                    .tool_calls
                    .iter()
                    .map(|tc| WireToolCallOut {
                        id: &tc.id,
                        kind: "function",
                        function: WireFunctionOut {
                            name: &tc.name,
                            // 把 arguments(內部 Value)序列化成 JSON 字串
                            arguments: serde_json::to_string(&tc.arguments)
                                .unwrap_or_else(|_| "{}".to_string()),
                        },
                    })
                    .collect(),
                tool_call_id: m.tool_call_id.as_deref(),
                name: m.name.as_deref(),
            })
            .collect();

        let wire_tools: Vec<WireTool> = tools
            .iter()
            .map(|t| WireTool {
                kind: "function",
                function: WireFunction {
                    name: &t.name,
                    description: &t.description,
                    parameters: &t.parameters,
                },
            })
            .collect();

        let body = ChatRequest {
            model: &self.model,
            messages: wire_messages,
            tools: wire_tools,
        };

        let url = format!("{}/chat/completions", self.base_url);
        tracing::debug!(model = %self.model, msgs = messages.len(), "groq chat request");

        // Retry loop:429 / 5xx 自動退避重試
        let body_json = serde_json::to_value(&body)
            .context("groq chat: serialize body")?;
        let mut text: String = String::new();
        for attempt in 1..=MAX_ATTEMPTS {
            let resp = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body_json)
                .send()
                .await
                .context("groq chat: send")?;

            let status = resp.status();
            if status.is_success() {
                text = resp.text().await.context("groq chat: read body")?;
                break;
            }

            // 失敗:先把 header 留下來,然後 consume body(讓 body 解析 + 錯誤訊息可用)
            let header_wait = Self::parse_retry_after_header(&resp);
            let err_body = resp.text().await.unwrap_or_default();

            if !Self::is_retriable(status) || attempt == MAX_ATTEMPTS {
                bail!("groq chat: HTTP {}: {}", status, err_body);
            }

            // 決策:retry 還是直接 surface。Surface 時 wait_secs 是 server
            // 告訴我們的真實等待秒數(可能很長,例如 TPD 用完要等 12 分鐘)。
            let decision = Self::decide_retry(
                &err_body,
                header_wait,
                BACKOFF_SECS[attempt - 1],
            );
            let reason = if status == StatusCode::TOO_MANY_REQUESTS {
                "rate_limit"
            } else {
                "server_error"
            };
            match decision {
                RetryDecision::Retry { wait_secs } => {
                    self.notify_and_wait("chat", attempt, wait_secs, reason).await;
                }
                RetryDecision::Surface { wait_secs } => {
                    let parsed = Some(wait_secs);
                    let hint = Self::friendly_rate_limit_hint(&err_body, parsed);
                    bail!(
                        "groq chat: {} (HTTP {}). 原始訊息:{}",
                        hint, status, err_body
                    );
                }
            }
        }

        let wire: ChatResponseWire =
            serde_json::from_str(&text).context("groq chat: parse response")?;

        let first = wire
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("groq chat: empty choices"))?;

        let tool_calls = first
            .message
            .tool_calls
            .into_iter()
            .map(|tc| {
                let arguments: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                    .unwrap_or_else(|_| serde_json::json!({}));
                ToolCall {
                    id: tc.id,
                    name: tc.function.name,
                    arguments,
                }
            })
            .collect();

        Ok(ChatResponse {
            content: first.message.content,
            tool_calls,
        })
    }
}

#[async_trait]
impl TranscriptionProvider for GroqProvider {
    fn name(&self) -> &'static str {
        "groq"
    }

    async fn transcribe(&self, audio: Vec<u8>) -> Result<String> {
        let url = format!("{}/audio/transcriptions", self.base_url);
        tracing::debug!(
            bytes = audio.len(),
            model = %self.transcribe_model,
            "groq transcribe request"
        );

        // multipart::Form 會被 send() 消耗,所以每個 attempt 重建。
        // language=zh + prompt 為「對 AI 助手講話」框架,避免字幕幻覺。
        let build_form = || -> Result<multipart::Form> {
            let part = multipart::Part::bytes(audio.clone())
                .file_name("audio.wav")
                .mime_str("audio/wav")
                .context("groq transcribe: build part")?;
            Ok(multipart::Form::new()
                .part("file", part)
                .text("model", self.transcribe_model.clone())
                .text("response_format", "json")
                .text("language", "zh")
                .text(
                    "prompt",
                    "以下是使用者直接對 AI 助手 Mori 說的話,繁體中文。\
                     常見用語:程式、軟體、檔案、影片、電腦、滑鼠、伺服器、資料庫、\
                     記住、提醒、行事曆、會議。",
                ))
        };

        let mut text = String::new();
        for attempt in 1..=MAX_ATTEMPTS {
            let form = build_form()?;
            let resp = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .multipart(form)
                .send()
                .await
                .context("groq transcribe: send")?;

            let status = resp.status();
            if status.is_success() {
                text = resp
                    .text()
                    .await
                    .context("groq transcribe: read body")?;
                break;
            }

            let header_wait = Self::parse_retry_after_header(&resp);
            let err_body = resp.text().await.unwrap_or_default();

            if !Self::is_retriable(status) || attempt == MAX_ATTEMPTS {
                bail!("groq transcribe: HTTP {}: {}", status, err_body);
            }

            let decision = Self::decide_retry(
                &err_body,
                header_wait,
                BACKOFF_SECS[attempt - 1],
            );
            let reason = if status == StatusCode::TOO_MANY_REQUESTS {
                "rate_limit"
            } else {
                "server_error"
            };
            match decision {
                RetryDecision::Retry { wait_secs } => {
                    self.notify_and_wait("transcribe", attempt, wait_secs, reason).await;
                }
                RetryDecision::Surface { wait_secs } => {
                    let hint = Self::friendly_rate_limit_hint(&err_body, Some(wait_secs));
                    bail!(
                        "groq transcribe: {} (HTTP {}). 原始訊息:{}",
                        hint, status, err_body
                    );
                }
            }
        }

        let parsed: TranscriptionResponse =
            serde_json::from_str(&text).context("groq transcribe: parse response")?;
        Ok(parsed.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_body_groq_format() {
        let body = r#"{"error":{"message":"Rate limit reached for model `openai/gpt-oss-120b` in organization xxx. Limit 30000 TPM. Used 28000 TPM. Please try again in 4.5s. Visit https://...","type":"tokens_rate_limit_exceeded"}}"#;
        assert_eq!(GroqProvider::parse_retry_after_body(body), Some(5));
    }

    #[test]
    fn parse_body_integer_seconds() {
        let body = "Please try again in 12s.";
        assert_eq!(GroqProvider::parse_retry_after_body(body), Some(12));
    }

    #[test]
    fn parse_body_no_match() {
        let body = "Generic 503 service unavailable";
        assert_eq!(GroqProvider::parse_retry_after_body(body), None);
    }

    #[test]
    fn parse_body_min_sec_format_tpd() {
        // 真實 Groq TPD response,以前 parser 只抓到 12 秒,實際上是 12m12s = 732s
        let body = r#"{"error":{"message":"Rate limit reached ... Please try again in 12m12.24s. ..."}}"#;
        assert_eq!(GroqProvider::parse_retry_after_body(body), Some(733));
    }

    #[test]
    fn parse_body_minutes_only() {
        let body = "Please try again in 5m.";
        assert_eq!(GroqProvider::parse_retry_after_body(body), Some(300));
    }

    #[test]
    fn parse_body_hours_minutes() {
        let body = "Please try again in 1h30m.";
        assert_eq!(GroqProvider::parse_retry_after_body(body), Some(3600 + 1800));
    }

    #[test]
    fn parse_body_with_whitespace_between_units() {
        let body = "Please try again in 2m 30s.";
        assert_eq!(GroqProvider::parse_retry_after_body(body), Some(150));
    }

    #[test]
    fn decide_prefers_body_over_header_short_wait() {
        let body = "Please try again in 4.5s";
        assert_eq!(
            GroqProvider::decide_retry(body, Some(60), 8),
            RetryDecision::Retry { wait_secs: 6 } // ceil(4.5)+1
        );
    }

    #[test]
    fn decide_falls_back_to_header() {
        let body = "no rate hint here";
        assert_eq!(
            GroqProvider::decide_retry(body, Some(3), 8),
            RetryDecision::Retry { wait_secs: 4 }
        );
    }

    #[test]
    fn decide_falls_back_to_backoff() {
        let body = "no rate hint here";
        assert_eq!(
            GroqProvider::decide_retry(body, None, 8),
            RetryDecision::Retry { wait_secs: 9 }
        );
    }

    #[test]
    fn decide_surfaces_when_wait_exceeds_max() {
        // 120s > 60s threshold → Surface,不要傻等
        let body = "Please try again in 120s";
        assert_eq!(
            GroqProvider::decide_retry(body, None, 8),
            RetryDecision::Surface { wait_secs: 120 }
        );
    }

    #[test]
    fn decide_surfaces_on_tpd_minutes_format() {
        // 真實 TPD case:732s,遠超 60s 上限 → Surface
        let body = "Please try again in 12m12.24s.";
        assert_eq!(
            GroqProvider::decide_retry(body, None, 8),
            RetryDecision::Surface { wait_secs: 733 }
        );
    }

    #[test]
    fn friendly_hint_recognises_tpd() {
        let body = "Rate limit reached ... on tokens per day (TPD): Limit 200000 ...";
        let hint = GroqProvider::friendly_rate_limit_hint(body, Some(733));
        assert!(hint.contains("TPD"), "expected TPD hint, got: {hint}");
    }

    #[test]
    fn friendly_hint_recognises_tpm_short_wait() {
        let body = "Rate limit reached ... on tokens per minute (TPM): ...";
        let hint = GroqProvider::friendly_rate_limit_hint(body, Some(5));
        assert!(hint.contains("TPM"));
        assert!(hint.contains("5s"));
    }
}
