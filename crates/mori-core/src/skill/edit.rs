//! EditMemorySkill — 編輯既有記憶。LLM 該配 recall_memory 一起用。

use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::context::Context;
use crate::memory::MemoryStore;
use super::{Skill, SkillOutput};

/// 編輯既有記憶的內容(保留 name / type,只換 body / description)。
///
/// 跟「呼叫 `remember` 用同 title 覆寫」效果類似,但更明確 —
/// 強制透過 id 指定,LLM 不會誤建新檔。建議用法:
/// 1. recall_memory(id) 看舊 content
/// 2. edit_memory(id, new_content) 寫整合後版本
pub struct EditMemorySkill {
    memory: Arc<dyn MemoryStore>,
}

impl EditMemorySkill {
    pub fn new(memory: Arc<dyn MemoryStore>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Skill for EditMemorySkill {
    fn name(&self) -> &'static str {
        "edit_memory"
    }

    fn description(&self) -> &'static str {
        "Update an existing memory's body content (keeping its name and type). \
         Call this when the user is amending or correcting a memory you \
         already have — typically after recall_memory revealed the old \
         content. Use the same id from the memory index. \
         Prefer this over calling `remember` for updates: it makes the intent \
         explicit and avoids accidentally creating a duplicate from a \
         slightly-different title."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Memory id(檔名不含 .md)。從索引或先前 recall_memory 結果取得。"
                },
                "new_content": {
                    "type": "string",
                    "description": "整合後的完整 content。要把舊 content + 新訊息合併,不可只寫新訊息。"
                },
                "new_description": {
                    "type": "string",
                    "description": "(可選)更新索引行的短描述。不給就保留舊的。"
                }
            },
            "required": ["id", "new_content"]
        })
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let id = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing id"))?
            .trim()
            .to_string();
        let new_content = args
            .get("new_content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing new_content"))?
            .trim()
            .to_string();
        let new_description = args
            .get("new_description")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string());

        let mut existing = self
            .memory
            .read(&id)
            .await
            .context("memory store read")?
            .ok_or_else(|| anyhow!("no memory with id: {id} — cannot edit"))?;

        existing.body = new_content.clone();
        if let Some(desc) = new_description {
            existing.description = desc;
        } else {
            // 沒給新描述,從新 content 抽前 60 字當描述
            existing.description = new_content.chars().take(60).collect();
        }
        existing.last_used = chrono::Utc::now();

        self.memory
            .write(existing.clone())
            .await
            .context("memory store write")?;

        Ok(SkillOutput {
            user_message: format!("好,把「{}」更新了", existing.name),
            data: Some(serde_json::json!({
                "id": existing.id,
                "name": existing.name,
                "updated": true,
            })),
        })
    }
}
