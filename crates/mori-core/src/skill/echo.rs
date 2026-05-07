//! EchoSkill — sanity check 用的最簡 skill。

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::context::Context;
use super::{Skill, SkillOutput};

pub struct EchoSkill;

#[async_trait]
impl Skill for EchoSkill {
    fn name(&self) -> &'static str {
        "echo"
    }
    fn description(&self) -> &'static str {
        "Repeat or rephrase the user's input back to them. Useful for confirming what was heard."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": { "type": "string", "description": "The text to echo back." }
            },
            "required": ["message"]
        })
    }
    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("(empty)")
            .to_string();
        Ok(SkillOutput {
            user_message: message,
            data: None,
        })
    }
}
