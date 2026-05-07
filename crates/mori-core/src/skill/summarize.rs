//! SummarizeSkill — phase 2 stub。實作由平行 agent 填入。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::context::Context;
use crate::llm::LlmProvider;
use super::{Skill, SkillOutput};

pub struct SummarizeSkill {
    #[allow(dead_code)]
    provider: Arc<dyn LlmProvider>,
}

impl SummarizeSkill {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Skill for SummarizeSkill {
    fn name(&self) -> &'static str {
        "summarize"
    }

    fn description(&self) -> &'static str {
        "Summarize a chunk of text into key points. Use when the user gives \
         long content (article, transcript, log) and wants the gist."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "要摘要的長文" },
                "style": {
                    "type": "string",
                    "description": "(可選)摘要風格",
                    "enum": ["bullet_points", "one_paragraph", "tldr"]
                },
                "max_points": {
                    "type": "integer",
                    "description": "(可選)bullet_points 模式時的條數上限,預設 5"
                }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, _args: Value, _context: &Context) -> Result<SkillOutput> {
        Err(anyhow!("SummarizeSkill not yet implemented (phase 2 stub)"))
    }
}
