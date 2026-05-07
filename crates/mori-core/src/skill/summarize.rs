//! SummarizeSkill — 把長文濃縮成重點。

use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::context::Context;
use crate::llm::{ChatMessage, LlmProvider};
use super::{Skill, SkillOutput};

pub struct SummarizeSkill {
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
         long content (article, transcript, log) and wants the gist — \
         '幫我摘要', '重點是什麼', 'tl;dr'."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "要摘要的長文" },
                "style": {
                    "type": "string",
                    "description": "(可選)摘要風格,預設 bullet_points",
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

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing text"))?
            .trim()
            .to_string();
        let style = args
            .get("style")
            .and_then(|v| v.as_str())
            .unwrap_or("bullet_points");
        let max_points = args
            .get("max_points")
            .and_then(|v| v.as_u64())
            .unwrap_or(5)
            .max(2)
            .min(10);

        let style_directive = match style {
            "one_paragraph" => "輸出**一段話**(不超過 5 句),濃縮成連貫敘述。".to_string(),
            "tldr" => "輸出**一句話結論**(15-30 字),用 TL;DR 心態壓到最簡。".to_string(),
            _ => format!(
                "輸出 **bullet points**,最多 {max_points} 條,每條一行,以 `- ` 開頭。\
                 每條一個獨立要點,不要把多個點塞在一條。"
            ),
        };

        let messages = vec![
            ChatMessage::system(format!(
                "你是摘要助手。把使用者給的長文濃縮成重點。\n\n\
                 規則:\n\
                 - 只保留事實 / 觀點,**不加自己的評論**\n\
                 - **不擴寫不存在的內容**,寧願簡短也不亂編\n\
                 - 用繁體中文\n\
                 - 不要前言「以下是摘要:」之類\n\n\
                 風格:{style_directive}"
            )),
            ChatMessage::user(text),
        ];

        let resp = self
            .provider
            .chat(messages, vec![])
            .await
            .context("summarize: provider chat")?;
        let summary = resp
            .content
            .ok_or_else(|| anyhow!("LLM returned no content"))?
            .trim()
            .to_string();

        Ok(SkillOutput {
            user_message: summary.clone(),
            data: Some(serde_json::json!({
                "style": style,
                "summary": summary,
            })),
        })
    }
}
