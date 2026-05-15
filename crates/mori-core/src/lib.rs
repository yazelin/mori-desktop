//! Mori 的大腦。平台無關、UI 無關。
//!
//! 對外暴露的能力建構在五個 trait 上:
//! - [`memory::MemoryStore`] — 長期記憶
//! - [`context::ContextProvider`] — 環境資訊抓取
//! - [`skill::Skill`] — LLM 可呼叫的工具
//! - [`llm::LlmProvider`] — chat / tool-calling 抽象
//! - [`llm::transcribe::TranscriptionProvider`] — speech-to-text 抽象(5C 起拆出來)
//!
//! 後續 phase 加 module / 加 impl 即可,trait 定義穩定,核心邏輯一行不動。

pub mod agent;
pub mod agent_profile;
pub mod annuli;
pub mod context;
pub mod corrections;
pub mod event_log;
pub mod installed_apps;
pub mod llm;
pub mod memory;
pub mod mode;
pub mod paste;
pub mod redact;
pub mod runtime;
pub mod skill;
pub mod tokenize;
pub mod url_detect;
pub mod voice_cleanup;
pub mod voice_input_profile;

/// crate 版本(供 UI 顯示)
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 當前 phase 名稱
pub const PHASE: &str = "4C — primary selection + ydotool paste-back";
