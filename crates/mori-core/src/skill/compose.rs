//! ComposeSkill — 替使用者草擬文字(信件、訊息、貼文、短文)。

use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::context::Context;
use crate::llm::{ChatMessage, LlmProvider};
use super::{Skill, SkillOutput};

pub struct ComposeSkill {
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
         something rather than answer a question. Triggers: '幫我寫一封信', \
         '草稿一個貼文', 'draft an email to ...'."
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
                "topic": { "type": "string", "description": "主題 / 核心想表達的事" },
                "audience": {
                    "type": "string",
                    "description": "(可選)讀者是誰,例:同事、客戶、朋友、不特定大眾"
                },
                "length_hint": {
                    "type": "string",
                    "description": "(可選)長度,預設 medium",
                    "enum": ["short", "medium", "long"]
                }
            },
            "required": ["kind", "topic"]
        })
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let kind = args
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing kind"))?
            .to_string();
        let topic = args
            .get("topic")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing topic"))?
            .trim()
            .to_string();
        let audience = args
            .get("audience")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string());
        let length = args
            .get("length_hint")
            .and_then(|v| v.as_str())
            .unwrap_or("medium");

        let kind_directive = match kind.as_str() {
            "email" => {
                "格式:Email。包含適當稱呼開頭、正文、結尾署名(如「敬上」、\
                 「Best regards」依語言)。"
            }
            "message" => "格式:即時訊息(LINE / Slack / Telegram 風格)。簡短直接,不要 email 客套。",
            "essay" => "格式:短文 / 部落格段落。有明確開頭、發展、結尾。",
            "social_post" => "格式:社群貼文(Twitter / Threads / FB 風格)。吸睛、自然、可放 hashtag。",
            _ => "格式:依主題自然發揮。",
        };

        let length_directive = match length {
            "short" => "長度:**短**,50-80 字內。",
            "long" => "長度:**長**,300-600 字。",
            _ => "長度:**中等**,120-200 字。",
        };

        let audience_directive = match audience.as_deref() {
            Some(a) if !a.is_empty() => format!("讀者:{a}。稱呼跟禮貌程度依此調整。\n"),
            _ => String::new(),
        };

        let messages = vec![
            ChatMessage::system(format!(
                "你是寫作助手。依使用者需求草擬文字。\n\n\
                 規則:\n\
                 - 用繁體中文(除非主題明顯需要英文)\n\
                 - **只輸出草稿本身**,不要前言「以下是草稿:」、不要事後解釋\n\
                 - 不要硬塞「希望這對你有幫助」之類客套尾巴\n\n\
                 {kind_directive}\n\
                 {length_directive}\n\
                 {audience_directive}"
            )),
            ChatMessage::user(format!("主題:{topic}")),
        ];

        let resp = self
            .provider
            .chat(messages, vec![])
            .await
            .context("compose: provider chat")?;
        let draft = resp
            .content
            .ok_or_else(|| anyhow!("LLM returned no content"))?
            .trim()
            .to_string();

        Ok(SkillOutput {
            user_message: draft.clone(),
            data: Some(serde_json::json!({
                "kind": kind,
                "topic": topic,
                "draft": draft,
            })),
        })
    }
}
