//! SetModeSkill — toggle Mori between Active and Background.
//!
//! Voice cues:
//! - Background:「晚安」「假寐」「安靜」「我先離開了」「下班了」
//! - Active:    「醒醒」「起床」「回來」「在嗎」「我回來了」
//!
//! Setting Background hard-stops the mic; the user can be sure no audio
//! is captured. Setting Active does NOT auto-start recording — that's
//! still a separate hotkey press.

use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::context::Context;
use crate::mode::{Mode, ModeController};
use super::{Skill, SkillOutput};

pub struct SetModeSkill {
    controller: Arc<dyn ModeController>,
}

impl SetModeSkill {
    pub fn new(controller: Arc<dyn ModeController>) -> Self {
        Self { controller }
    }
}

#[async_trait]
impl Skill for SetModeSkill {
    fn name(&self) -> &'static str {
        "set_mode"
    }

    fn description(&self) -> &'static str {
        "Switch Mori between Active (mic ready, UI visible) and Background \
         (mic OFF, UI hidden — privacy mode). Call when the user clearly \
         signals they're done talking for now (e.g. '晚安','安靜一下', \
         '我去開會了') with mode='background', or wants Mori back \
         (e.g. '醒醒','回來','我回來了') with mode='active'. Don't call \
         it for ambiguous statements — only when the user's intent is \
         explicit. Setting 'active' does NOT start a recording — that's \
         still a separate user action."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["active", "background"],
                    "description": "Target operating mode."
                }
            },
            "required": ["mode"]
        })
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let raw = args
            .get("mode")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing mode"))?;

        let target = match raw {
            "active" => Mode::Active,
            "background" => Mode::Background,
            other => return Err(anyhow!("invalid mode: {other}")),
        };

        let current = self.controller.current_mode().await;
        if current == target {
            return Ok(SkillOutput {
                user_message: match target {
                    Mode::Active => "我已經在這了。".to_string(),
                    Mode::Background => "我已經在假寐了。".to_string(),
                },
                data: Some(serde_json::json!({
                    "mode": target.as_str(),
                    "changed": false,
                })),
            });
        }

        self.controller
            .set_mode(target)
            .await
            .context("set mode")?;

        let user_message = match target {
            Mode::Active => "我回來了。".to_string(),
            Mode::Background => "好,我先安靜下,有事按熱鍵叫我。".to_string(),
        };

        Ok(SkillOutput {
            user_message,
            data: Some(serde_json::json!({
                "mode": target.as_str(),
                "changed": true,
            })),
        })
    }
}
