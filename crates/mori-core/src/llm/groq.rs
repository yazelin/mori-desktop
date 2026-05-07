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
    /// Groq 訊息格式範例:
    /// "Rate limit reached for model `xxx`. ... Please try again in 4.5s. ..."
    /// "Please try again in 12.345s."
    ///
    /// 抓到 X 後 ceil 取整。
    fn parse_retry_after_body(body: &str) -> Option<u64> {
        let lower = body.to_lowercase();
        let i = lower.find("try again in ")?;
        let rest = &body[i + "try again in ".len()..];
        let mut num = String::new();
        for ch in rest.chars() {
            if ch.is_ascii_digit() || ch == '.' {
                num.push(ch);
            } else {
                break;
            }
        }
        let secs: f64 = num.parse().ok()?;
        if !(secs.is_finite() && secs >= 0.0) {
            return None;
        }
        Some(secs.ceil() as u64)
    }

    /// 計算「該等多少秒」 — 優先順序:body parse > Retry-After header > backoff schedule。
    /// 算出後加 +1s 緩衝(避免邊界 race),clamp 到 [1, 60]。
    fn compute_wait_secs(
        body: &str,
        header_secs: Option<u64>,
        fallback_secs: u64,
    ) -> u64 {
        let base = Self::parse_retry_after_body(body)
            .or(header_secs)
            .unwrap_or(fallback_secs);
        (base + 1).clamp(1, 60)
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
    /// (含 placeholder,使用者編輯一次後就可用)。
    pub fn bootstrap_mori_config() -> anyhow::Result<std::path::PathBuf> {
        let home = home_dir()
            .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
        let dir = home.join(".mori");
        std::fs::create_dir_all(&dir)?;

        let config = dir.join("config.json");
        if !config.exists() {
            let stub = serde_json::json!({
                "providers": {
                    "groq": {
                        "api_key": "REPLACE_ME_WITH_YOUR_GROQ_API_KEY",
                        "chat_model": GroqProvider::DEFAULT_CHAT_MODEL,
                        "transcribe_model": GroqProvider::DEFAULT_TRANSCRIBE_MODEL
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

fn read_json_pointer(path: &std::path::Path, pointer: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let key = json.pointer(pointer)?.as_str()?;
    if key.is_empty() || is_placeholder(key) {
        return None;
    }
    Some(key.to_string())
}

// ─── chat completion request/response wire types ────────────────────

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool<'a>>,
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    /// `null` 是合法的(assistant 發 tool_call 時可能 content=null)
    content: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireToolCallOut<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
    /// `tool` role 訊息要附 tool 名(可選但 OpenAI 標準有)
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

/// 送出時的 tool_call 結構(OpenAI 巢狀 function 格式)
#[derive(Serialize)]
struct WireToolCallOut<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str, // always "function"
    function: WireFunctionOut<'a>,
}

#[derive(Serialize)]
struct WireFunctionOut<'a> {
    name: &'a str,
    /// arguments 必須是 JSON-encoded 字串,不是物件
    arguments: String,
}

#[derive(Serialize)]
struct WireTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireFunction<'a>,
}

#[derive(Serialize)]
struct WireFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(Deserialize, Debug)]
struct ChatResponseWire {
    choices: Vec<ChoiceWire>,
}

#[derive(Deserialize, Debug)]
struct ChoiceWire {
    message: ChoiceMessageWire,
}

#[derive(Deserialize, Debug)]
struct ChoiceMessageWire {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallWire>,
}

#[derive(Deserialize, Debug)]
struct ToolCallWire {
    id: String,
    function: ToolCallFunctionWire,
}

#[derive(Deserialize, Debug)]
struct ToolCallFunctionWire {
    name: String,
    arguments: String, // JSON string
}

// ─── transcription wire ─────────────────────────────────────────────

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

            // 計算等待時間(body 內「try again in X」優先,header 次之,backoff 兜底,加 +1s 緩衝)
            let wait = Self::compute_wait_secs(
                &err_body,
                header_wait,
                BACKOFF_SECS[attempt - 1],
            );
            let reason = if status == StatusCode::TOO_MANY_REQUESTS {
                "rate_limit"
            } else {
                "server_error"
            };
            self.notify_and_wait("chat", attempt, wait, reason).await;
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

            let wait = Self::compute_wait_secs(
                &err_body,
                header_wait,
                BACKOFF_SECS[attempt - 1],
            );
            let reason = if status == StatusCode::TOO_MANY_REQUESTS {
                "rate_limit"
            } else {
                "server_error"
            };
            self.notify_and_wait("transcribe", attempt, wait, reason).await;
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
    fn compute_wait_prefers_body_over_header() {
        // body says 4.5s, header says 60 — should pick ceil(4.5)+1 = 6
        let body = "Please try again in 4.5s";
        assert_eq!(GroqProvider::compute_wait_secs(body, Some(60), 8), 6);
    }

    #[test]
    fn compute_wait_falls_back_to_header() {
        let body = "no rate hint here";
        assert_eq!(GroqProvider::compute_wait_secs(body, Some(3), 8), 4);
    }

    #[test]
    fn compute_wait_falls_back_to_backoff() {
        let body = "no rate hint here";
        assert_eq!(GroqProvider::compute_wait_secs(body, None, 8), 9);
    }

    #[test]
    fn compute_wait_clamps_to_60() {
        let body = "Please try again in 120s";
        assert_eq!(GroqProvider::compute_wait_secs(body, None, 8), 60);
    }
}
