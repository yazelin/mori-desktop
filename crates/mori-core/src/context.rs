//! 「按熱鍵那一瞬間」抓到的環境資訊。
//!
//! 各平台實作各自的 [`ContextProvider`](crate::context::ContextProvider)。
//! Wayland 受沙箱限制,部分欄位需走 xdg-desktop-portal,phase 4+ 處理。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Context {
    /// 使用者語音(原始音訊位元組,通常 wav/opus)
    pub voice_audio: Option<Vec<u8>>,

    /// 反白選中的文字。Linux 走 Wayland primary selection
    /// (`wl-paste --primary`)或 X11 PRIMARY。macOS / Windows 待實作
    /// (沒 primary 概念,要走 Accessibility API 或模擬 Cmd/Ctrl+C)。
    pub selected_text: Option<String>,

    /// 剪貼簿內容
    pub clipboard: Option<String>,

    /// 滑鼠座標(螢幕絕對位置)
    pub cursor_position: Option<(i32, i32)>,

    /// 活躍視窗標題
    pub active_window_title: Option<String>,

    /// 活躍 app 識別(bundle id / exec name / wm class)
    pub active_app: Option<String>,

    /// 滑鼠附近截圖(供 vision LLM 看)
    pub screenshot_around_cursor: Option<Vec<u8>>,

    /// 從 selected_text / clipboard / URL bar 抽到的網址
    pub urls_detected: Vec<String>,
}

/// 平台特定的 context 抓取實作。
///
/// Phase 1: 只實作 voice + clipboard。
/// Phase 3+: selected_text / cursor / urls。
/// Phase 4+: screenshot / active_window(各平台 + Wayland portal)。
#[async_trait]
pub trait ContextProvider: Send + Sync {
    /// 抓取當下能拿到的所有 context。允許部分欄位 None。
    async fn capture(&self) -> Context;
}
