//! Tauri 平台的 ContextProvider 實作。
//!
//! Phase 3A:剪貼簿文字。
//! Phase 4C:Linux primary selection(`wl-paste --primary` shell out)— 給
//!          反白即改寫流程用。

use async_trait::async_trait;
use mori_core::context::{Context, ContextProvider};
use tauri::AppHandle;
#[cfg(not(target_os = "linux"))]
use tauri_plugin_clipboard_manager::ClipboardExt;

pub struct TauriContextProvider {
    #[cfg_attr(target_os = "linux", allow(dead_code))] // Linux 讀剪貼簿走 xclip,用不到
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

        // Linux 用 xclip 讀:Tauri clipboard plugin → arboard → wl-clipboard-rs 在 Wayland 下
        // 會跳「wl-clipboard 要求存取剪貼簿」通知,不點就卡住整條語音輸入(10/8 實測)。
        #[cfg(target_os = "linux")]
        let read = std::process::Command::new("xclip")
            .args(["-selection", "clipboard", "-o"])
            .output()
            .map_err(|e| e.to_string())
            .and_then(|o| if o.status.success() { Ok(String::from_utf8_lossy(&o.stdout).into_owned()) } else { Err(format!("xclip exit {}", o.status)) });
        #[cfg(not(target_os = "linux"))]
        let read = self.app.clipboard().read_text().map_err(|e| e.to_string());

        match read {
            Ok(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    ctx.clipboard = Some(text);
                }
            }
            Err(e) => {
                tracing::warn!(?e, "clipboard read_text failed (image content / missing permission / Wayland quirk)");
            }
        }

        // Phase 4C:讀 primary selection(滑鼠反白)。Linux X11 真的能讀;
        // Windows 沒這概念,selection_windows.rs 一律回 None,fall through。
        if let Some(sel) = crate::selection::read_primary_selection() {
            tracing::info!(chars = sel.chars().count(), "captured primary selection");
            ctx.selected_text = Some(sel);
        }

        ctx
    }
}
