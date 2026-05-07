//! PolishSkill — 改錯字 / 改文法 / 修語氣,保留作者本意。

use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::context::Context;
use crate::llm::{ChatMessage, LlmProvider};
use super::{Skill, SkillOutput};

pub struct PolishSkill {
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
         awkward phrasing while preserving meaning and tone. Use when the user \
         asks to '潤稿' / '改一下' / '幫我修' / 'fix the grammar' on a piece \
         of their own writing."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "要潤稿的文字" },
                "tone": {
                    "type": "string",
                    "description": "(可選)指定語氣風格,預設 auto 保留原本語氣",
                    "enum": ["formal", "casual", "concise", "detailed", "auto"]
                }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing text"))?
            .trim()
            .to_string();
        let tone = args
            .get("tone")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");

        let tone_directive = match tone {
            "formal" => "改成正式書面語氣(用於商業書信、報告)",
            "casual" => "改成口語、輕鬆語氣(用於朋友訊息、社群貼文)",
            "concise" => "盡量簡潔,刪贅字、合併重複句意",
            "detailed" => "適度展開,讓讀者更清楚脈絡",
            _ => "保留作者原本的語氣風格,不要改變正式 / 口語的程度",
        };

        let messages = vec![
            ChatMessage::system(format!(
                "你是專業文字編輯。改善使用者給的文字。\n\n\
                 規則:\n\
                 - 修正錯字、文法、標點\n\
                 - 改善生硬或不順的表達\n\
                 - **保留作者本意**,不要過度改寫或加新內容\n\
                 - **只輸出潤稿後的版本**,不要說明改了什麼、不要前言\n\
                 - 不要加引號包住輸出,除非原文就有引號\n\
                 - 保留原本的格式(換行、條列、Markdown)\n\n\
                 語氣指示:{tone_directive}"
            )),
            ChatMessage::user(text),
        ];

        let resp = self
            .provider
            .chat(messages, vec![])
            .await
            .context("polish: provider chat")?;
        let polished = resp
            .content
            .ok_or_else(|| anyhow!("LLM returned no content"))?
            .trim()
            .to_string();

        Ok(SkillOutput {
            user_message: polished.clone(),
            data: Some(serde_json::json!({
                "tone": tone,
                "polished": polished,
            })),
        })
    }
}
