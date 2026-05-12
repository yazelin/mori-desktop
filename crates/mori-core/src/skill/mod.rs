//! Skill = LLM 可呼叫的工具。
//!
//! 設計原則:
//! - 每個 skill 自成一個檔案在 `skill/` 下,實作 [`Skill`] trait
//! - schema 給 LLM 看(透過 JSON Schema)→ 餵進 OpenAI tool-calling
//! - destructive 操作 `confirm_required: true`,UI / TTS 二次確認
//! - 跨裝置:`target_capability` 表明能在哪跑(Local / Remote / Anywhere)
//!
//! 使用流程:
//! 1. [`SkillRegistry::new`] + [`register`](SkillRegistry::register) 註冊 skill
//! 2. 把 [`SkillRegistry::tool_definitions`] 拿到的 list 交給 LLM
//! 3. LLM 回 tool_calls → [`SkillRegistry::dispatch`] 拿 skill 跑
//!
//! 加新 skill 的 SOP:
//! 1. 在 `skill/` 下開新檔(例:`translate.rs`),實作 `impl Skill for TranslateSkill`
//! 2. 在這個檔末尾加 `pub mod translate;` + `pub use translate::TranslateSkill;`
//! 3. 在 `mori-tauri/src/main.rs` 的 stop_and_transcribe registry 段註冊
//! 4. system prompt 裡加一段使用守則(在 build_system_prompt 加 entry)

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context::Context;
use crate::llm::ToolDefinition;

// ─── 公開類型 ──────────────────────────────────────────────────────

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
    /// 給使用者看的自然語言訊息(在 single-round 模式會疊在 LLM 回覆之後;
    /// multi-turn 模式會被當 tool result 餵回 LLM)
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

// ─── Registry ──────────────────────────────────────────────────────

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

// ─── 內建 skills ──────────────────────────────────────────────────

mod echo;
mod edit;
mod forget;
mod recall;
mod remember;

// Phase 2 基礎 skills(平行開發中,各自一個檔案)
mod compose;
mod polish;
mod summarize;
mod translate;

// Phase 4B-2: 模式控制(Active / Background)
mod set_mode;

// Phase 4C: 反白 → 講話 → 結果貼回
mod paste_selection;

// Phase 3B: URL 偵測 + 抓網址內容
mod fetch_url;

pub use echo::EchoSkill;
pub use edit::EditMemorySkill;
pub use forget::ForgetMemorySkill;
pub use recall::RecallMemorySkill;
pub use remember::RememberSkill;

pub use compose::ComposeSkill;
pub use polish::PolishSkill;
pub use summarize::SummarizeSkill;
pub use translate::TranslateSkill;

pub use set_mode::SetModeSkill;
pub use paste_selection::PasteSelectionBackSkill;

pub use fetch_url::FetchUrlSkill;

// ─── 共用 helpers(memory-related skills 用)────────────────────────

/// 把標題 slug 化成檔名安全的 id。
/// 規則:保留 CJK / 英數字 / 底線,空白→底線,其他標點丟掉。
///
/// **故意不加 timestamp**:同 title 會 overwrite 既有檔案。讓 LLM 用同 title
/// 寫「整合後的完整 content」就能更新既有記憶。
pub(crate) fn slugify(title: &str) -> String {
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
