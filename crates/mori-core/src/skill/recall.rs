//! RecallMemorySkill — LLM 看索引按需拉單筆 memory body。

use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::context::Context;
use crate::memory::MemoryStore;
use super::{Skill, SkillOutput};

/// 從長期記憶讀取單筆 memory 的完整 body。
///
/// LLM 在 system prompt 裡只看到記憶**索引**(name + 短描述);若覺得某筆
/// memory 對當前對話有幫助,呼叫這個 skill 把 body 拉進來。這個 skill 的
/// 結果會回傳給 LLM(透過 multi-turn tool call),不直接顯示給使用者。
pub struct RecallMemorySkill {
    memory: Arc<dyn MemoryStore>,
}

impl RecallMemorySkill {
    pub fn new(memory: Arc<dyn MemoryStore>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Skill for RecallMemorySkill {
    fn name(&self) -> &'static str {
        "recall_memory"
    }

    fn description(&self) -> &'static str {
        "Read the full content of a specific long-term memory by id. \
         Use this when the memory index (shown in system prompt) lists a \
         memory whose name/description suggests it's relevant to the user's \
         current question, but you need the full body to answer accurately. \
         The id is the filename without .md (e.g. for `2026-05-11_會議.md` \
         the id is `2026-05-11_會議`)."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Memory id(檔名不含 .md)。從 system prompt 的索引段抓。"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let id = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing id"))?
            .trim()
            .to_string();

        let memory = self
            .memory
            .read(&id)
            .await
            .context("memory store read")?
            .ok_or_else(|| anyhow!("no memory with id: {id}"))?;

        // user_message 餵回 LLM(multi-turn 機制),不直接顯示給使用者。
        // 內容直接拼整篇,讓 LLM 看到完整脈絡。
        let body_dump = format!(
            "Memory id={} name={:?} type={:?}\n\n{}",
            memory.id, memory.name, memory.memory_type, memory.body
        );

        Ok(SkillOutput {
            user_message: body_dump,
            data: Some(serde_json::json!({
                "id": memory.id,
                "name": memory.name,
                "body": memory.body,
            })),
        })
    }
}
