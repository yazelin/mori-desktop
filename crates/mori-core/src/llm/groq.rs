//! Groq LLM provider — chat completion + Whisper transcription。
//!
//! 接 OpenAI 相容 API:
//! - `POST /chat/completions` for chat
//! - `POST /audio/transcriptions` for Whisper

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use reqwest::multipart;
use serde::{Deserialize, Serialize};

use super::{ChatMessage, ChatResponse, LlmProvider, ToolCall, ToolDefinition};

pub struct GroqProvider {
    api_key: String,
    model: String,
    transcribe_model: String,
    base_url: String,
    client: reqwest::Client,
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

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .context("groq chat: send")?;

        let status = resp.status();
        let text = resp.text().await.context("groq chat: read body")?;
        if !status.is_success() {
            bail!("groq chat: HTTP {}: {}", status, text);
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

        let part = multipart::Part::bytes(audio)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .context("groq transcribe: build part")?;

        // language=zh 強制中文 + prompt 偏好繁中,避免 Whisper 預設輸出簡中。
        // prompt 也順便包進台灣常見用語,讓辭彙選擇更在地化。
        let form = multipart::Form::new()
            .part("file", part)
            .text("model", self.transcribe_model.clone())
            .text("response_format", "json")
            .text("language", "zh")
            .text(
                "prompt",
                "以下是台灣繁體中文逐字稿,使用繁體中文字。\
                 常見用語:程式、軟體、檔案、影片、電腦、滑鼠、伺服器、資料庫。",
            );

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await
            .context("groq transcribe: send")?;

        let status = resp.status();
        let text = resp.text().await.context("groq transcribe: read body")?;
        if !status.is_success() {
            bail!("groq transcribe: HTTP {}: {}", status, text);
        }

        let parsed: TranscriptionResponse =
            serde_json::from_str(&text).context("groq transcribe: parse response")?;
        Ok(parsed.text)
    }
}
