//! Mori 的大腦。平台無關、UI 無關。
//!
//! 對外暴露的能力建構在四個 trait 上:
//! - [`memory::MemoryStore`] — 長期記憶
//! - [`context::ContextProvider`] — 環境資訊抓取
//! - [`skill::Skill`] — LLM 可呼叫的工具
//! - [`llm::LlmProvider`] — LLM 通訊抽象
//!
//! Phase 1 只實作每個 trait 的最小可運作骨架。後續 phase 加 module / 加 impl
//! 即可,trait 定義穩定,核心邏輯一行不動。

pub mod agent;
pub mod context;
pub mod llm;
pub mod memory;
pub mod skill;
pub mod voice;

/// crate 版本(供 UI 顯示)
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 當前 phase 名稱
pub const PHASE: &str = "1E — Multi-turn tools + RecallMemorySkill";
