//! Tauri 平台的 ContextProvider 實作。
//!
//! Phase 3A:只先抓**剪貼簿文字**。其他欄位(selected_text、cursor_position、
//! active_window 等)留 phase 3B/4 各平台再實作 — 那些跨 app 在 Wayland 上
//! 受沙箱限制較多,需走 xdg-desktop-portal,工程量較大。

use async_trait::async_trait;
use mori_core::context::{Context, ContextProvider};
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

pub struct TauriContextProvider {
    app: AppHandle,
}

impl TauriContextProvider {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

#[async_trait]
impl ContextProvider for TauriContextProvider {
    async fn capture(&self) -> Context {
        let mut ctx = Context::default();

        match self.app.clipboard().read_text() {
            Ok(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    ctx.clipboard = Some(text);
                }
            }
            Err(e) => {
                // 非 fatal,但提到 warn — 之前 capabilities 漏 allow-read-text
                // 時整個 context 一直空白,debug log 看不到根本不知道。
                tracing::warn!(?e, "clipboard read_text failed (image content / missing permission / Wayland quirk)");
            }
        }

        ctx
    }
}
