//! 長期記憶系統。三層分離:Core / Working / Archival。
//!
//! 設計細節見 docs/memory.md。

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};

pub mod annuli;
pub mod markdown;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryType {
    UserIdentity,
    Preference,
    SkillOutcome,
    Project,
    Reference,
    /// 5E-3:VoiceInput cleanup 用的「校正詞庫 / 專有名詞」記憶。
    /// 例如「Annuli 別翻成『安奴利』」、人名 / 公司名 / 慣用語 — 由 user 在
    /// Agent 模式 remember 寫入,VoiceInput 模式 read-only 注入到 cleanup prompt。
    VoiceDict,
    Other(String),
}

impl MemoryType {
    /// frontmatter `type: <X>` 的 canonical 字串。`Other` 直接回原值,
    /// 其餘走 snake_case 規格(對齊 markdown.rs frontmatter 寫出格式)。
    pub fn as_str(&self) -> String {
        match self {
            MemoryType::UserIdentity => "user_identity".into(),
            MemoryType::Preference => "preference".into(),
            MemoryType::SkillOutcome => "skill_outcome".into(),
            MemoryType::Project => "project".into(),
            MemoryType::Reference => "reference".into(),
            MemoryType::VoiceDict => "voice_dict".into(),
            MemoryType::Other(s) => s.clone(),
        }
    }

    /// frontmatter `type:` 字串 → variant。case-insensitive,接受 `-` / `_` /
    /// 無分隔符三種寫法(對齊既有 `parse_memory_type` 的彈性)。未知字串 →
    /// `Other(原值)`。
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "user_identity" | "user-identity" | "useridentity" => MemoryType::UserIdentity,
            "preference" => MemoryType::Preference,
            "skill_outcome" | "skill-outcome" | "skilloutcome" => MemoryType::SkillOutcome,
            "project" => MemoryType::Project,
            "reference" => MemoryType::Reference,
            "voice_dict" | "voice-dict" | "voicedict" => MemoryType::VoiceDict,
            other => MemoryType::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub name: String,
    pub description: String,
    pub memory_type: MemoryType,
    pub created: DateTime<Utc>,
    pub last_used: DateTime<Utc>,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryIndexEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub memory_type: MemoryType,
}

#[derive(Debug, Clone)]
pub enum MemoryEvent {
    Written(Memory),
    Updated(Memory),
    Deleted(String),
}

/// 長期記憶 store 抽象。
///
/// Phase 1: [`markdown::LocalMarkdownMemoryStore`]
/// Phase 5+: VecMemoryStore(加 sqlite-vec 加速)
/// Phase 7+: SyncedMemoryStore(跨裝置 CRDT)
/// Phase 9+: AnnuliMcpMemoryStore(透過 MCP 接 Annuli)
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// 讀取索引(MEMORY.md)
    async fn read_index(&self) -> Result<Vec<MemoryIndexEntry>>;

    /// 讀取單一 memory
    async fn read(&self, id: &str) -> Result<Option<Memory>>;

    /// 寫入或更新
    async fn write(&self, memory: Memory) -> Result<()>;

    /// 搜尋(phase 1: grep + LLM 判斷;phase 5+: vector search)
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Memory>>;

    /// 刪除
    async fn delete(&self, id: &str) -> Result<()>;

    /// 訂閱事件流
    fn observe(&self) -> BoxStream<'static, MemoryEvent>;

    /// 5E-3:列出 `memory_type` 屬於 `types` 任一個的 Memory(整本讀進來)。
    /// 給 VoiceInput cleanup 注入「校正詞庫」用 — `voice_dict` 之類短小條目,
    /// 整本拼進 system prompt 還在合理 token 範圍內(<2KB)。
    ///
    /// 預設 impl 走 `read_index` + 逐筆 `read`(index 不存 type,要逐檔讀
    /// frontmatter 才知道 type 真值)。memory 通常 <50 篇,IO 量小不需 cache;
    /// 自己 impl `MemoryStore`(例 phase 5+ vector store)可以 override 走 index。
    async fn list_by_types(&self, types: &[MemoryType]) -> Result<Vec<Memory>> {
        if types.is_empty() {
            return Ok(Vec::new());
        }
        let entries = self.read_index().await?;
        let mut out = Vec::new();
        for entry in entries {
            let Some(mem) = self.read(&entry.id).await? else { continue };
            if types.iter().any(|t| t == &mem.memory_type) {
                out.push(mem);
            }
        }
        Ok(out)
    }
}
