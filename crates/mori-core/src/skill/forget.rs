//! ForgetMemorySkill — 刪除一筆記憶。Destructive,需 confirm。

use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::context::Context;
use crate::memory::MemoryStore;
use super::{Skill, SkillOutput};

/// 刪除指定 id 的記憶(連同索引行)。LLM 在使用者明確要求「忘掉」、
/// 「刪除」、「不用記了」之類時呼叫。
///
/// 是 destructive 操作 — `confirm_required` 為 true,理想上 UI 該攔下
/// 二次確認再執行。Phase 1F 暫時沒攔(skill 直接執行)— phase 4+ 會做白名單
/// + UI confirm flow。
pub struct ForgetMemorySkill {
    memory: Arc<dyn MemoryStore>,
}

impl ForgetMemorySkill {
    pub fn new(memory: Arc<dyn MemoryStore>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Skill for ForgetMemorySkill {
    fn name(&self) -> &'static str {
        "forget_memory"
    }

    fn description(&self) -> &'static str {
        "Delete a memory by id. Call this when the user explicitly asks you \
         to forget / delete / remove a piece of information (e.g. '忘掉...', \
         '不用記了', '把那個刪掉'). Identify the right memory by checking \
         the memory index in system prompt. Operation is destructive — only \
         call when the user's intent to delete is clear."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Memory id(檔名不含 .md)。從 system prompt 索引段抓。"
                }
            },
            "required": ["id"]
        })
    }

    fn confirm_required(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let id = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing id"))?
            .trim()
            .to_string();

        // 先確認存在,不存在直接回錯,免得「忘了一個本來就沒有的」假成功
        let existed = self
            .memory
            .read(&id)
            .await
            .context("memory store read")?
            .is_some();
        if !existed {
            return Ok(SkillOutput {
                user_message: format!("找不到 id 為 `{id}` 的記憶,沒東西可忘"),
                data: None,
            });
        }

        self.memory
            .delete(&id)
            .await
            .context("memory store delete")?;

        Ok(SkillOutput {
            user_message: format!("好,把 `{id}` 的記憶忘掉了"),
            data: Some(serde_json::json!({ "id": id, "deleted": true })),
        })
    }
}
