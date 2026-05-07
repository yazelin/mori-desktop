//! RememberSkill — 把使用者告訴 Mori 的事寫進長期記憶。

use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::context::Context;
use crate::memory::{Memory, MemoryStore, MemoryType};
use super::{slugify, Skill, SkillOutput};

/// 把使用者告訴 Mori 的事寫進長期記憶(`~/.mori/memory/<id>.md`)。
///
/// LLM 在以下情況該呼叫:
/// - 使用者明確說「記住 X」/「以後 X」/「我喜歡 X」/「我老婆是 X」
/// - 重要日期(生日、紀念日、deadline)
/// - 進行中的專案、長期偏好、人名地名
pub struct RememberSkill {
    memory: Arc<dyn MemoryStore>,
}

impl RememberSkill {
    pub fn new(memory: Arc<dyn MemoryStore>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Skill for RememberSkill {
    fn name(&self) -> &'static str {
        "remember"
    }

    fn description(&self) -> &'static str {
        "Save a fact about the user to Mori's long-term memory. \
         Call this when the user explicitly asks you to remember something \
         (e.g. '記住...', '以後...'), or when they share personal info worth \
         keeping (preferences, important dates like birthdays, names of people \
         / pets / projects, recurring tasks). \
         Each call creates one memory file the user can later edit or delete."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "簡短標題,3-15 個字。例如「老婆生日」、「常用編輯器」。"
                },
                "content": {
                    "type": "string",
                    "description": "完整內容,自然語言寫清楚是什麼事 / 偏好。例如「老婆生日是 11 月 3 日,別忘記」。"
                },
                "category": {
                    "type": "string",
                    "enum": ["user_identity", "preference", "project", "reference", "other"],
                    "description": "類別:user_identity=使用者基本資料、preference=偏好習慣、project=進行中專案、reference=查詢資料、other=其他。"
                }
            },
            "required": ["title", "content", "category"]
        })
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing title"))?
            .trim()
            .to_string();
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing content"))?
            .trim()
            .to_string();
        let category_str = args
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("other");
        let memory_type = match category_str {
            "user_identity" => MemoryType::UserIdentity,
            "preference" => MemoryType::Preference,
            "project" => MemoryType::Project,
            "reference" => MemoryType::Reference,
            other => MemoryType::Other(other.to_string()),
        };

        let id = slugify(&title);
        let now = Utc::now();
        let memory = Memory {
            id: id.clone(),
            name: title.clone(),
            description: content.chars().take(60).collect(),
            memory_type,
            created: now,
            last_used: now,
            body: content,
        };

        self.memory
            .write(memory)
            .await
            .context("memory store write")?;

        Ok(SkillOutput {
            user_message: format!("好,我記下了:「{title}」"),
            data: Some(serde_json::json!({ "id": id })),
        })
    }
}
