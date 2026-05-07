//! ComposeSkill — phase 2 stub。實作由平行 agent 填入。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::context::Context;
use crate::llm::LlmProvider;
use super::{Skill, SkillOutput};

pub struct ComposeSkill {
    #[allow(dead_code)]
    provider: Arc<dyn LlmProvider>,
}

impl ComposeSkill {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Skill for ComposeSkill {
    fn name(&self) -> &'static str {
        "compose"
    }

    fn description(&self) -> &'static str {
        "Compose / draft a piece of writing for the user — email, message, \
         short essay, social post, etc. Use when the user wants you to *write* \
         something rather than answer a question."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "description": "要寫什麼類型",
                    "enum": ["email", "message", "essay", "social_post", "other"]
                },
                "topic": { "type": "string", "description": "主題 / 想表達的核心" },
                "audience": { "type": "string", "description": "(可選)讀者是誰,例:同事、客戶、朋友" },
                "length_hint": {
                    "type": "string",
                    "description": "(可選)長度",
                    "enum": ["short", "medium", "long"]
                }
            },
            "required": ["kind", "topic"]
        })
    }

    async fn execute(&self, _args: Value, _context: &Context) -> Result<SkillOutput> {
        Err(anyhow!("ComposeSkill not yet implemented (phase 2 stub)"))
    }
}
