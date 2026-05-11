//! Generic OpenAI-compatible provider.
//!
//! ZeroType 的 ZEROTYPE_AIPROMPT_API_BASE + KEY + MODEL 三個 frontmatter
//! 鍵可以指向任何 OpenAI-compatible endpoint（Gemini、Azure OpenAI、OpenRouter 等）。
//! 這個 provider 把它們組成一個完整的 LlmProvider 實作。
//!
//! 比 GroqProvider 簡單：沒有 Groq-specific retry logic、沒有 rate-limit 解析，
//! 只做基本的 HTTP chat completion。

use anyhow::{Context as _, Result};
use async_trait::async_trait;

use super::{ChatMessage, ChatResponse, LlmProvider, ToolCall, ToolDefinition};
use super::openai_compat::{
    ChatRequest, ChatResponseWire, WireFunctionOut, WireMessage, WireTool, WireToolCallOut,
    WireFunction,
};

pub struct GenericOpenAiProvider {
    api_base: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl GenericOpenAiProvider {
    pub fn new(
        api_base: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            api_base: api_base.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model: model.into(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl LlmProvider for GenericOpenAiProvider {
    fn name(&self) -> &'static str {
        "openai-compat"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn supports_tool_calling(&self) -> bool {
        true
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
    ) -> Result<ChatResponse> {
        let wire_messages: Vec<WireMessage<'_>> = messages
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

        let wire_tools: Vec<WireTool<'_>> = tools
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

        let url = format!("{}/chat/completions", self.api_base);

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .context("openai-compat chat request")?;

        let status = resp.status();
        let text = resp.text().await.context("read response body")?;

        if !status.is_success() {
            anyhow::bail!(
                "openai-compat chat failed with status {status}: {text}"
            );
        }

        let wire: ChatResponseWire =
            serde_json::from_str(&text).context("parse chat response")?;

        let choice = wire
            .choices
            .into_iter()
            .next()
            .context("no choices in response")?;

        let tool_calls = choice
            .message
            .tool_calls
            .into_iter()
            .map(|tc| {
                let args = serde_json::from_str(&tc.function.arguments)
                    .unwrap_or(serde_json::Value::Null);
                ToolCall {
                    id: tc.id,
                    name: tc.function.name,
                    arguments: args,
                }
            })
            .collect();

        Ok(ChatResponse {
            content: choice.message.content,
            tool_calls,
        })
    }
}
