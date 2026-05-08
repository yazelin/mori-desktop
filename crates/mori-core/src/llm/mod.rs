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
pub mod ollama;
mod openai_compat;

// ─── Provider factory ───────────────────────────────────────────────
//
// `build_chat_provider` 讀 `~/.mori/config.json` 的 `default_provider`
// 欄位,構造對應 LlmProvider 回傳。Groq / Ollama 走不同 default。
// retry_callback 只對 Groq 有意義(Ollama 本機沒 rate limit)。

use std::sync::Arc;

/// 從 `~/.mori/config.json` 蓋出 chat provider。
/// 配置:
/// - `default_provider`: "groq"(預設) | "ollama"
/// - `providers.groq.{api_key, chat_model}`
/// - `providers.ollama.{base_url, model}`
///
/// retry_callback 只在 Groq 路徑套用(Ollama 本機沒 rate-limit)。
pub fn build_chat_provider(
    retry_cb: Option<groq::RetryCallback>,
) -> anyhow::Result<Arc<dyn LlmProvider>> {
    let default = mori_config_path()
        .as_deref()
        .and_then(|p| groq::read_json_pointer(p, "/default_provider"))
        .unwrap_or_else(|| "groq".to_string());

    match default.as_str() {
        "ollama" => {
            let base_url = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/ollama/base_url"))
                .unwrap_or_else(|| ollama::OllamaProvider::DEFAULT_BASE_URL.to_string());
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/ollama/model"))
                .unwrap_or_else(|| ollama::OllamaProvider::DEFAULT_MODEL.to_string());
            tracing::info!(provider = "ollama", model = %model, base_url = %base_url, "chat provider selected");
            Ok(Arc::new(ollama::OllamaProvider::new(base_url, model)))
        }
        other => {
            if other != "groq" {
                tracing::warn!(
                    provider = other,
                    "unknown default_provider — falling back to 'groq'",
                );
            }
            let key = groq::GroqProvider::discover_api_key().ok_or_else(|| {
                anyhow::anyhow!(
                    "no GROQ_API_KEY configured. Edit ~/.mori/config.json or set $GROQ_API_KEY \
                     (or set default_provider to 'ollama' if you want to use local LLM only)"
                )
            })?;
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/groq/chat_model"))
                .unwrap_or_else(|| groq::GroqProvider::DEFAULT_CHAT_MODEL.to_string());
            tracing::info!(provider = "groq", model = %model, "chat provider selected");
            let p = groq::GroqProvider::new(key, model);
            let p = if let Some(cb) = retry_cb {
                p.with_retry_callback(cb)
            } else {
                p
            };
            Ok(Arc::new(p))
        }
    }
}

/// 啟動時的 best-effort warm-up:若使用者把 `default_provider` 設成 ollama,
/// 背景發一個 1-token 的 chat 把模型載進 RAM,使用者第一次按熱鍵時就不用
/// 等 cold start(qwen3:8b 5.2GB 在 Intel CPU 沒 GPU 加速可能要分鐘級)。
///
/// Provider 是 groq 時直接 no-op(網路 LLM 沒 cold start)。
pub async fn warm_up_default_provider() {
    let default = mori_config_path()
        .as_deref()
        .and_then(|p| groq::read_json_pointer(p, "/default_provider"))
        .unwrap_or_else(|| "groq".to_string());

    if default != "ollama" {
        return;
    }

    let base_url = mori_config_path()
        .as_deref()
        .and_then(|p| groq::read_json_pointer(p, "/providers/ollama/base_url"))
        .unwrap_or_else(|| ollama::OllamaProvider::DEFAULT_BASE_URL.to_string());
    let model = mori_config_path()
        .as_deref()
        .and_then(|p| groq::read_json_pointer(p, "/providers/ollama/model"))
        .unwrap_or_else(|| ollama::OllamaProvider::DEFAULT_MODEL.to_string());

    ollama::OllamaProvider::warm_up(&base_url, &model).await;
}

fn mori_config_path() -> Option<std::path::PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".mori").join("config.json"))
}

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
