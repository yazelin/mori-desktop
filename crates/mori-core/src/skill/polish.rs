//! PolishSkill — phase 2 stub。實作由平行 agent 填入。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::context::Context;
use crate::llm::LlmProvider;
use super::{Skill, SkillOutput};

pub struct PolishSkill {
    #[allow(dead_code)]
    provider: Arc<dyn LlmProvider>,
}

impl PolishSkill {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Skill for PolishSkill {
    fn name(&self) -> &'static str {
        "polish"
    }

    fn description(&self) -> &'static str {
        "Polish / proofread / improve the user's writing. Fix typos, grammar, \
         awkward phrasing while preserving meaning and tone."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "要潤稿的文字" },
                "tone": {
                    "type": "string",
                    "description": "(可選)指定語氣 — 正式 / 口語 / 簡潔 / 詳細",
                    "enum": ["formal", "casual", "concise", "detailed", "auto"]
                }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, _args: Value, _context: &Context) -> Result<SkillOutput> {
        Err(anyhow!("PolishSkill not yet implemented (phase 2 stub)"))
    }
}
