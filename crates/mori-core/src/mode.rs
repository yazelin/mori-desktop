//! Operating mode — orthogonal to the conversation phase.
//!
//! - `Active` — Mori is "here":對話模式。floating UI visible, mic
//!   available for the hotkey, scheduler running。熱鍵 → 錄音 → STT →
//!   走 **agent loop**(LLM 決定 dispatch 哪個 skill,或直接回應)。
//! - `VoiceInput` — 語音輸入模式(Phase 5E)。熱鍵 → 錄音 → STT → **跳過
//!   agent loop**,走輕度 LLM cleanup(標點 / 幻聽修正,保留原詞)→ 用
//!   `PasteController` 把結果直接貼到游標位置。把 Mori 變成一個 LLM
//!   加持的 dictation 工具,適合在瀏覽器 / 編輯器裡聽寫長文。
//! - `Background` — 休眠:mic completely off, UI hidden except for the
//!   tray icon, scheduler still ticking. Privacy-first:the user can be
//!   sure no microphone capture happens in this mode.
//!
//! Mode transitions are user-initiated (tray menu, UI button, voice
//! command「晚安」/「醒醒」). The shell crate (mori-tauri) owns the
//! actual state; mori-core just exposes the [`ModeController`] trait
//! so a [`crate::skill::Skill`] can change mode without depending on
//! Tauri.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// 對話模式:熱鍵 → STT → agent loop → chat / skill dispatch。
    Active,
    /// 語音輸入模式:熱鍵 → STT → 輕度清理 → 貼到游標位置(跳過 agent)。
    VoiceInput,
    /// 休眠:麥克風完全關閉。
    Background,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Active => "active",
            Mode::VoiceInput => "voice_input",
            Mode::Background => "background",
        }
    }

    /// 麥克風能不能在這個 mode 下開。VoiceInput 跟 Active 一樣需要
    /// 收音;Background 才硬關。
    pub fn allows_mic(&self) -> bool {
        matches!(self, Mode::Active | Mode::VoiceInput)
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
