//! 長期記憶系統。三層分離:Core / Working / Archival。
//!
//! 設計細節見 docs/memory.md。

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};

pub mod markdown;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemoryType {
    UserIdentity,
    Preference,
    SkillOutcome,
    Project,
    Reference,
    Other(String),
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
}
