//! Ollama 本地 LLM provider — OpenAI-compatible HTTP endpoint.
//!
//! 預設打 `http://localhost:11434/v1/chat/completions`,wire format 跟
//! Groq / OpenAI 一樣 → 整套 wire types 借用 [`super::openai_compat`]。
//!
//! 跟 Groq 比較,Ollama 路徑簡單很多:
//! - 沒 API key(本機),所以不送 Authorization header
//! - 沒 rate-limit / TPD / TPM 概念,失敗就 server 沒在跑或 model 沒
//!   下載,沒有 retry-after 邏輯,只做一次失敗即報錯
//! - Tool-calling 支援度 看 model:
//!     - **支援**:qwen3、llama3.3、mistral、命名為 `*-tools` 的 model
//!     - **不支援**:很多舊版或基礎 model — server 會回 400 / tool calls 為空
//!   Mori 的 agent loop 在沒拿到 tool_calls 時會 fall through 到純文字回應,
//!   不會崩,只是 skill routing 會吃癟。建議 main agent 還是 Groq,Ollama
//!   留給 skill 內部 chat。
//!
//! ## Config(`~/.mori/config.json`)
//!
//! ```json
//! {
//!   "providers": {
//!     "ollama": {
//!       "base_url": "http://localhost:11434",
//!       "model": "qwen3:8b"
//!     }
//!   }
//! }
//! ```

use std::time::Duration;

use anyhow::{bail, Context as _, Result};
use async_trait::async_trait;
use reqwest::Client;

use super::openai_compat::{
    ChatRequest, ChatResponseWire, WireFunction, WireFunctionOut, WireMessage, WireTool,
    WireToolCallOut,
};
use super::{ChatMessage, ChatResponse, LlmProvider, ToolCall, ToolDefinition};

pub struct OllamaProvider {
    client: Client,
    base_url: String,
    model: String,
}

impl OllamaProvider {
    pub const DEFAULT_BASE_URL: &'static str = "http://localhost:11434";
    pub const DEFAULT_MODEL: &'static str = "qwen3:8b";

    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        // 較寬鬆 timeout — 本機 LLM 第一次 load model 可能要 10-30s,後續才秒回。
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("reqwest client");
        let base_url = base_url.into();
        // 容忍 trailing slash
        let base_url = base_url.trim_end_matches('/').to_string();
        Self {
            client,
            base_url,
            model: model.into(),
        }
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn name(&self) -> &'static str {
        "ollama"
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
                content: m.content.as_deref(),
                tool_calls: m
                    .tool_calls
                    .iter()
                    .map(|tc| WireToolCallOut {
                        id: &tc.id,
                        kind: "function",
                        function: WireFunctionOut {
                            name: &tc.name,
                            arguments: tc.arguments.to_string(),
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

        let url = format!("{}/v1/chat/completions", self.base_url);
        tracing::debug!(model = %self.model, msgs = messages.len(), "ollama chat request");

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context(
                "ollama chat: send (is the daemon running? `systemctl --user status ollama` \
                 or `ollama serve`)",
            )?;

        let status = resp.status();
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            bail!(
                "ollama chat: HTTP {}: {} (model `{}` 沒下載?跑 `ollama pull {}`)",
                status,
                err_body,
                self.model,
                self.model,
            );
        }

        let text = resp.text().await.context("ollama chat: read body")?;
        let wire: ChatResponseWire = serde_json::from_str(&text)
            .context("ollama chat: parse response (server returned non-OpenAI shape?)")?;

        let first = wire
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("ollama chat: empty choices"))?;

        let tool_calls = first
            .message
            .tool_calls
            .into_iter()
            .map(|tc| {
                let arguments: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or_else(|_| {
                        serde_json::json!({ "_raw": tc.function.arguments })
                    });
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

    // transcribe → 預設 trait impl 回 "not supported" — Ollama 沒 STT。
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_url_no_trailing_slash() {
        let p = OllamaProvider::new("http://localhost:11434/", "qwen3:8b");
        assert_eq!(p.base_url, "http://localhost:11434");
    }

    #[test]
    fn name_is_ollama() {
        let p = OllamaProvider::new(OllamaProvider::DEFAULT_BASE_URL, "qwen3:8b");
        assert_eq!(p.name(), "ollama");
    }
}
