//! Groq LLM provider — chat completion + Whisper transcription。
//!
//! Phase 1 提供:
//! - chat:呼叫 `openai/gpt-oss-120b`(或設定的其他模型)
//! - transcribe:呼叫 `whisper-large-v3-turbo`
//!
//! 實作 stub 在這個 PR;實際 HTTP 呼叫在下一個 PR(phase 1B)補上。

use anyhow::Result;
use async_trait::async_trait;

use super::{ChatMessage, ChatResponse, LlmProvider, ToolDefinition};

pub struct GroqProvider {
    api_key: String,
    model: String,
    base_url: String,
}

impl GroqProvider {
    pub const DEFAULT_BASE_URL: &'static str = "https://api.groq.com/openai/v1";
    pub const DEFAULT_CHAT_MODEL: &'static str = "openai/gpt-oss-120b";
    pub const DEFAULT_TRANSCRIBE_MODEL: &'static str = "whisper-large-v3-turbo";

    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: Self::DEFAULT_BASE_URL.to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// 嘗試從以下來源依序取得 GROQ_API_KEY:
    /// 1. `GROQ_API_KEY` 環境變數
    /// 2. `~/.pi/agent/models.json` 的 `providers.groq.apiKey`
    /// 3. `~/.mori/config.json` 的 `groq.api_key`(phase 1B 加)
    pub fn discover_api_key() -> Option<String> {
        if let Ok(key) = std::env::var("GROQ_API_KEY") {
            if !key.is_empty() {
                return Some(key);
            }
        }
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok()?;
        let pi_config = std::path::Path::new(&home).join(".pi").join("agent").join("models.json");
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
        _messages: Vec<ChatMessage>,
        _tools: Vec<ToolDefinition>,
    ) -> Result<ChatResponse> {
        // TODO(phase 1B): POST {base_url}/chat/completions with Bearer api_key
        //                 parse function-calling response into ChatResponse.
        let _ = (&self.api_key, &self.base_url);
        anyhow::bail!("GroqProvider::chat not yet implemented (phase 1B)")
    }

    async fn transcribe(&self, _audio: Vec<u8>) -> Result<String> {
        // TODO(phase 1B): POST multipart/form-data to {base_url}/audio/transcriptions
        //                 with file=audio, model=whisper-large-v3-turbo.
        anyhow::bail!("GroqProvider::transcribe not yet implemented (phase 1B)")
    }
}
