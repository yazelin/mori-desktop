//! OpenAI-compatible wire types for chat completion + tool calling.
//!
//! Shared between every provider that speaks the de-facto-standard
//! `/v1/chat/completions` schema:
//! - Groq (`api.groq.com/openai/v1/...`)
//! - Ollama (`localhost:11434/v1/...`)
//! - OpenAI 自身, OpenRouter, vLLM, etc.
//!
//! Providers with totally different shapes(Anthropic native, Claude CLI
//! subprocess)don't use these types — they implement [`LlmProvider`]
//! against their own protocol.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ─── Outgoing(client → server)──────────────────────────────────────

#[derive(Serialize)]
pub(crate) struct ChatRequest<'a> {
    pub model: &'a str,
    pub messages: Vec<WireMessage<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<WireTool<'a>>,
}

#[derive(Serialize)]
pub(crate) struct WireMessage<'a> {
    pub role: &'a str,
    /// `null` 合法(assistant 發起 tool_call 時 content 可能 null)
    pub content: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<WireToolCallOut<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<&'a str>,
    /// `tool` role 訊息要附 tool 名(OpenAI 標準)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'a str>,
}

/// 送出時的 tool_call 結構(OpenAI 巢狀 function 格式)
#[derive(Serialize)]
pub(crate) struct WireToolCallOut<'a> {
    pub id: &'a str,
    #[serde(rename = "type")]
    pub kind: &'static str, // always "function"
    pub function: WireFunctionOut<'a>,
}

#[derive(Serialize)]
pub(crate) struct WireFunctionOut<'a> {
    pub name: &'a str,
    /// arguments 必須是 JSON-encoded 字串,不是物件
    pub arguments: String,
}

#[derive(Serialize)]
pub(crate) struct WireTool<'a> {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: WireFunction<'a>,
}

#[derive(Serialize)]
pub(crate) struct WireFunction<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub parameters: &'a Value,
}

// ─── Incoming(server → client)──────────────────────────────────────

#[derive(Deserialize, Debug)]
pub(crate) struct ChatResponseWire {
    pub choices: Vec<ChoiceWire>,
}

#[derive(Deserialize, Debug)]
pub(crate) struct ChoiceWire {
    pub message: ChoiceMessageWire,
}

#[derive(Deserialize, Debug)]
pub(crate) struct ChoiceMessageWire {
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallWire>,
}

#[derive(Deserialize, Debug)]
pub(crate) struct ToolCallWire {
    pub id: String,
    pub function: ToolCallFunctionWire,
}

#[derive(Deserialize, Debug)]
pub(crate) struct ToolCallFunctionWire {
    pub name: String,
    pub arguments: String, // JSON string
}
