//! TranslateSkill — phase 2 stub。實作由平行 agent 填入。
//!
//! 目標:LLM 看到使用者明確要翻譯時呼叫。args 帶 source_text + target_lang。
//! Skill 內部用自己的 LlmProvider 跑一輪純翻譯 chat,把結果 user_message 回傳。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::context::Context;
use crate::llm::LlmProvider;
use super::{Skill, SkillOutput};

pub struct TranslateSkill {
    #[allow(dead_code)] // stub:agent 之後會用
    provider: Arc<dyn LlmProvider>,
}

impl TranslateSkill {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Skill for TranslateSkill {
    fn name(&self) -> &'static str {
        "translate"
    }

    fn description(&self) -> &'static str {
        "Translate text from one language to another. Use when the user \
         explicitly asks for translation."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "source_text": { "type": "string", "description": "要翻譯的原文" },
                "target_lang": { "type": "string", "description": "目標語言(zh-TW / en / ja / ...)" }
            },
            "required": ["source_text", "target_lang"]
        })
    }

    async fn execute(&self, _args: Value, _context: &Context) -> Result<SkillOutput> {
        Err(anyhow!("TranslateSkill not yet implemented (phase 2 stub)"))
    }
}
