//! Operating mode — orthogonal to the conversation phase.
//!
//! 5G 起 mori 有兩種「動作模式」+ 一種「休眠」：
//!
//! - `Agent` — Mori 模式：熱鍵 → 錄音 → STT → agent loop（LLM 決定
//!   dispatch 哪個 skill / 直接對話 / 執行動作）。對應 Ctrl+Alt+N profile。
//!   名字之所以是 Agent 而非 Active：強調 Mori 在「做事」+ 有 agency，
//!   不只是「醒著」。
//! - `VoiceInput` — 語音輸入模式：熱鍵 → 錄音 → STT → 單輪 LLM cleanup
//!   → `PasteController` 直接貼到游標。對應 Alt+N profile。永遠單輪，
//!   不做動作，只做「字」。
//! - `Background` — 休眠：mic 硬關，UI 隱藏。privacy-first。
//!
//! ## 鍵盤直覺
//! - Alt+N         → VoiceInput profile（「我要輸入字」）
//! - Ctrl+Alt+N    → Agent profile（「我要叫 Mori 做事」）
//! - Ctrl+Alt+Space → toggle 錄音（兩個 mode 共用）

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Agent 模式：STT → agent loop → chat / skill dispatch / action。
    /// 5G 前叫 Active；改名為 Agent 強調「Mori 在做事」非「醒著」。
    Agent,
    /// 語音輸入模式：STT → 單輪 LLM cleanup → 貼游標。永遠單輪，
    /// 不做 tool calling、不做動作。
    VoiceInput,
    /// 休眠：麥克風完全關閉。
    Background,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Agent => "agent",
            Mode::VoiceInput => "voice_input",
            Mode::Background => "background",
        }
    }

    /// 麥克風能不能在這個 mode 下開。Agent / VoiceInput 都需要;Background 才硬關。
    pub fn allows_mic(&self) -> bool {
        matches!(self, Mode::Agent | Mode::VoiceInput)
    }
}

/// Skills (in mori-core) and other non-shell code talk to whoever owns
/// the mode through this trait. mori-tauri implements it on top of
/// `AppState`.
#[async_trait]
pub trait ModeController: Send + Sync {
    async fn current_mode(&self) -> Mode;
    /// Set mode. Should be idempotent — calling with the current mode
    /// returns Ok and doesn't emit a redundant transition event.
    async fn set_mode(&self, mode: Mode) -> anyhow::Result<()>;
}
