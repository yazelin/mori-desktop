//! SetModeSkill — toggle Mori between Active / VoiceInput / Background.
//!
//! Voice cues:
//! - Background:「晚安」「先休眠」「安靜」「我先離開了」「下班了」
//! - Active:    「醒醒」「起來」「回來」「在嗎」「我回來了」
//! - VoiceInput:「切到輸入模式」「我要打字」「我要 dictation」
//!
//! Setting Background hard-stops the mic; the user can be sure no audio
//! is captured. Setting Active or VoiceInput does NOT auto-start
//! recording — that's still a separate hotkey press。

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
        "Switch Mori between three modes:\n\
         - 'active' — 對話模式(預設):熱鍵錄音 → STT → agent loop。\n\
         - 'voice_input' — 語音輸入模式:熱鍵錄音 → STT → 輕度清理 → \
           直接貼到游標位置(跳過 agent)。適合在編輯器/瀏覽器裡聽寫。\n\
         - 'background' — 休眠:麥克風完全關閉(privacy)。\n\
         Voice cues:'晚安' / '先休眠' → background;'醒醒' / '回來' → \
         active;'切到輸入模式' / '我要 dictation' → voice_input。"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["active", "voice_input", "background"],
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
            "voice_input" => Mode::VoiceInput,
            "background" => Mode::Background,
            other => return Err(anyhow!("invalid mode: {other}")),
        };

        let current = self.controller.current_mode().await;
        if current == target {
            return Ok(SkillOutput {
                user_message: match target {
                    Mode::Active => "我醒著呢。".to_string(),
                    Mode::VoiceInput => "已經在輸入模式了。".to_string(),
                    Mode::Background => "我已經在休眠了。".to_string(),
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
            Mode::Active => "醒了,我在這。".to_string(),
            Mode::VoiceInput => "切到輸入模式 — 接下來的話會直接貼到游標位置。".to_string(),
            Mode::Background => "好,我先閉眼,叫我就回來。".to_string(),
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
