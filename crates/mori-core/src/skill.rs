//! Skill = LLM 可呼叫的工具。
//!
//! 設計原則:
//! - 每個 skill 自成一個 struct,實作 [`Skill`] trait
//! - schema 給 LLM 看(透過 JSON Schema)→ 餵進 OpenAI tool-calling
//! - destructive 操作 `confirm_required: true`,UI / TTS 二次確認
//! - 跨裝置:`target_capability` 表明能在哪跑(Local / Remote / Anywhere)
//!
//! 使用流程:
//! 1. [`SkillRegistry::new`] + [`register`](SkillRegistry::register) 註冊 skill
//! 2. 把 [`SkillRegistry::tool_definitions`] 拿到的 list 交給 LLM
//! 3. LLM 回 tool_calls → [`SkillRegistry::dispatch`] 拿 skill 跑

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context::Context;
use crate::llm::ToolDefinition;
use crate::memory::{Memory, MemoryStore, MemoryType};

/// 這個 skill 可以在哪邊執行?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionTarget {
    /// 只能本機(例如:存取本地檔案、控制本機音量)
    Local,
    /// 必須指定遠端裝置(例如:操作家裡那台桌機的某個服務)
    Remote(u64),
    /// 任一線上裝置都可以(由 LLM 或負載決定)
    Anywhere,
}

/// 隱私等級。LocalOnly 的 skill 強制只用本地 LLM,絕不送雲端。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Privacy {
    Cloud,
    LocalOnly,
}

/// Skill 執行結果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillOutput {
    /// 給使用者看的自然語言訊息(會疊在 LLM 回覆之後)
    pub user_message: String,
    /// 結構化資料(供下游 skill / UI 串接)
    pub data: Option<Value>,
}

#[async_trait]
pub trait Skill: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters_schema(&self) -> Value;

    fn confirm_required(&self) -> bool {
        false
    }
    fn target_capability(&self) -> ExecutionTarget {
        ExecutionTarget::Local
    }
    fn privacy(&self) -> Privacy {
        Privacy::Cloud
    }

    async fn execute(&self, args: Value, context: &Context) -> Result<SkillOutput>;
}

// ─── Registry ───────────────────────────────────────────────────────

/// Skill 註冊表。Agent 用它把 skill 暴露給 LLM,並 dispatch 回呼。
pub struct SkillRegistry {
    skills: HashMap<&'static str, Arc<dyn Skill>>,
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    pub fn register(&mut self, skill: Arc<dyn Skill>) {
        let name = skill.name();
        if self.skills.contains_key(name) {
            tracing::warn!(name, "skill name collision — replacing existing");
        }
        self.skills.insert(name, skill);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Skill>> {
        self.skills.get(name).cloned()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.skills.keys().copied().collect()
    }

    /// 把所有 skill 轉成 LLM tool definitions(OpenAI / Groq 相容)
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.skills
            .values()
            .map(|s| ToolDefinition {
                name: s.name().to_string(),
                description: s.description().to_string(),
                parameters: s.parameters_schema(),
            })
            .collect()
    }

    /// 根據 LLM 回的 tool_call 派發給對應 skill
    pub async fn dispatch(
        &self,
        name: &str,
        args: Value,
        context: &Context,
    ) -> Result<SkillOutput> {
        let skill = self
            .get(name)
            .ok_or_else(|| anyhow!("unknown skill: {name}"))?;
        tracing::info!(skill = name, "dispatching");
        skill.execute(args, context).await
    }
}

// ─── EchoSkill(sanity check 用)──────────────────────────────────

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

// ─── RememberSkill ──────────────────────────────────────────────────

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

// ─── RecallMemorySkill ──────────────────────────────────────────────

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

/// 把標題 slug 化成檔名安全的 id。
/// 規則:保留 CJK / 英數字 / 底線,空白→底線,其他標點丟掉。
///
/// **故意不加 timestamp**:同 title 會 overwrite 既有檔案。這是有意的 —
/// 讓 LLM 用同 title 寫「整合後的完整 content」就能更新既有記憶。
/// 整合語意責任在 LLM 端(讀舊 content + 新訊息 → 寫整合版本),
/// store 端只負責覆寫。詳見 system prompt 裡 remember tool 的使用規則。
fn slugify(title: &str) -> String {
    let mut out = String::new();
    for ch in title.chars() {
        if ch.is_alphanumeric()
            || (ch as u32 >= 0x4E00 && ch as u32 <= 0x9FFF) // CJK 漢字
            || ch == '_'
        {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("memory");
    }
    out
}
