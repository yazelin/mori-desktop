//! LLM 通訊抽象。
//!
//! 一份 agent 邏輯能打 Groq / Ollama / OpenAI / Anthropic 等任意 OpenAI 相容後端。
//! 每個 Skill 可指定偏好的 provider + model,允許:
//! - 任務 → 模型精細搭配
//! - Fallback chain
//! - Privacy::LocalOnly 強制本地
//!
//! 訊息結構支援 OpenAI tool-calling 多輪協定:
//! - `system` / `user`:role + content
//! - `assistant`(發起 tool_call):role + tool_calls(content 可能也有)
//! - `tool`(回傳結果):role + content + tool_call_id + name

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod groq;

/// 一則訊息。
///
/// 用 `Option<String>` 給 content 是因為 assistant 在發起 tool_call 時可能
/// 沒文字內容。`tool_calls` 只在 assistant 發起時非空。`tool_call_id` + `name`
/// 只在 role="tool" 時用,把回傳結果連回對應的 tool_call。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant_with_tool_calls(
        content: Option<String>,
        tool_calls: Vec<ToolCall>,
    ) -> Self {
        Self {
            role: "assistant".into(),
            content,
            tool_calls,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn tool_result(
        call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
            name: Some(name.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// API 給的唯一 id(回傳 tool 結果要 reference 它)
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    /// LLM 自由文字回應(若沒呼叫 tool 或 mid-thought)
    pub content: Option<String>,
    /// LLM 決定呼叫的 tools
    pub tool_calls: Vec<ToolCall>,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Provider 識別名(groq / ollama / openai / anthropic / ...)
    fn name(&self) -> &'static str;

    /// 模型 id
    fn model(&self) -> &str;

    /// 跑一輪 chat completion。
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
    ) -> Result<ChatResponse>;

    /// 音訊轉文字(支援的 provider 才有,如 Groq Whisper)
    async fn transcribe(&self, _audio: Vec<u8>) -> Result<String> {
        anyhow::bail!("provider {} does not support transcription", self.name());
    }
}
