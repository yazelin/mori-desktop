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
    /// 2. `~/.pi/agent/models.json` 的 `providers.groq.apiKey`
    pub fn discover_api_key() -> Option<String> {
        if let Ok(key) = std::env::var("GROQ_API_KEY") {
            if !key.is_empty() {
                return Some(key);
            }
        }
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .ok()?;
        let pi_config = std::path::Path::new(&home)
            .join(".pi")
            .join("agent")
            .join("models.json");
        if let Ok(text) = std::fs::read_to_string(&pi_config) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(key) = json
                    .pointer("/providers/groq/apiKey")
                    .and_then(|v| v.as_str())
                {
                    if !key.is_empty() && !key.starts_with("REPLACE") {
                        return Some(key.to_string());
                    }
                }
            }
        }
        None
    }
}

// ─── chat completion request/response wire types ────────────────────

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [WireMessage<'a>],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool<'a>>,
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    content: &'a str,
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
    function: ToolCallFunctionWire,
}

#[derive(Deserialize, Debug)]
struct ToolCallFunctionWire {
    name: String,
    arguments: String, // OpenAI returns args as a JSON string, not object
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
        let wire_messages: Vec<WireMessage> = messages
            .iter()
            .map(|m| WireMessage {
                role: &m.role,
                content: &m.content,
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
            messages: &wire_messages,
            tools: wire_tools,
        };

        let url = format!("{}/chat/completions", self.base_url);
        tracing::debug!(model = %self.model, "groq chat request");

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

        let form = multipart::Form::new()
            .part("file", part)
            .text("model", self.transcribe_model.clone())
            .text("response_format", "json");

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
