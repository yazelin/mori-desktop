//! Operating mode — orthogonal to the conversation phase.
//!
//! - `Active` — Mori is "here": floating UI visible, mic available for
//!   the hotkey, scheduler running. The default and most-of-the-time
//!   state.
//! - `Background` — Mori 在休眠:mic completely off, UI hidden
//!   except for the tray icon, scheduler still ticking. Privacy-first:
//!   the user can be sure no microphone capture happens in this mode.
//!
//! Mode transitions are user-initiated (tray menu, hotkey from
//! background to wake, voice command「晚安」/「醒醒」). The shell crate
//! (mori-tauri) owns the actual state; mori-core just exposes the
//! [`ModeController`] trait so a [`crate::skill::Skill`] can change
//! mode without depending on Tauri.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// 正常運作:UI 可見、麥克風待命、熱鍵可錄音。
    Active,
    /// 休眠:UI 隱藏、麥克風完全關閉、排程繼續跑。
    Background,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Active => "active",
            Mode::Background => "background",
        }
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
