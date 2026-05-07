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
                // 不是 fatal — 例如剪貼簿是圖片時,read_text 會失敗
                tracing::debug!(?e, "clipboard read_text returned err (non-text content?)");
            }
        }

        ctx
    }
}
