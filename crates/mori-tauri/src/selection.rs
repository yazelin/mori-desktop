//! Linux primary selection + paste-back via shell-out.
//!
//! Mori reads what the user has highlighted in another app via Wayland's
//! primary-selection protocol (or X11 PRIMARY under XWayland) using
//! `wl-paste --primary`. To replace the highlighted range, we write the
//! result to the clipboard via `wl-copy` and then synthesize a Ctrl+V
//! keypress with `ydotool` so the focused (still the original) app
//! receives a paste.
//!
//! Why ydotool, not wtype: GNOME mutter doesn't implement
//! `zwp_virtual_keyboard_v1`, so wtype silently does nothing. ydotool
//! works at the kernel uinput layer, compositor-agnostic.
//!
//! Setup is one-time: `sudo bash setup-wayland-input.sh` from
//! yazelin/ubuntu-26.04-setup installs `wl-clipboard` + `ydotool`,
//! adds the user to the `input` group, enables the ydotoold systemd
//! user unit. Reboot once for input-group membership to take effect on
//! the systemd manager.

use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context as _, Result};
use arboard::{Clipboard, GetExtLinux, LinuxClipboardKind};
use async_trait::async_trait;
use mori_core::paste::{PasteController, PasteResult};
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

/// 最大允許的反白字數 — 太長就視為使用者選了整篇，不適合直接送 Whisper /
/// LLM tool args。1500 是經驗值(中文 ~2000 token,加上提示 + 結果輸出
/// 大概 5-6K total,留有餘地給 Groq gpt-oss-120b TPM)。
const MAX_SELECTION_CHARS: usize = 1500;

/// 讀 X11 PRIMARY selection(滑鼠反白文字)。
///
/// 走 `arboard` 的 X11 backend — 我們強制 `GDK_BACKEND=x11` 讓 Mori 跑
/// 在 XWayland 相容層,XWayland 自動把 Wayland primary selection 同步到
/// X11 PRIMARY,所以從 X11 client 視角讀就拿到使用者的反白。**完全不
/// 走 Wayland portal**,GNOME 不會跳「未知 wl-clipboard 要求剪貼簿存
/// 取」對話框。
///
/// 失敗 / 反白為空 → 回 None。**不**做 fatal。
pub fn read_primary_selection() -> Option<String> {
    let mut clipboard = match Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(?e, "arboard Clipboard::new failed (no display?)");
            return None;
        }
    };
    let text = match clipboard.get().clipboard(LinuxClipboardKind::Primary).text() {
        Ok(t) => t,
        Err(e) => {
            // 反白為空 / 不是文字(圖片) / 沒 selection owner 都會 Err,
            // 全部當「沒抓到」即可,不要 warn 洗 log。
            tracing::trace!(?e, "primary selection unavailable");
            return None;
        }
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Cap 過長選取(整篇文章不該整個塞 LLM context)。
    let truncated = if trimmed.chars().count() > MAX_SELECTION_CHARS {
        let head: String = trimmed.chars().take(MAX_SELECTION_CHARS).collect();
        tracing::info!(
            total = trimmed.chars().count(),
            kept = MAX_SELECTION_CHARS,
            "primary selection truncated for context",
        );
        head
    } else {
        trimmed.to_string()
    };

    Some(truncated)
}

/// PasteController 的 Linux 實作:**Tauri 剪貼簿插件**寫剪貼簿(用
/// Mori 自己 process 走 portal,不會跳「未知 wl-clipboard 要求權限」對話
/// 框)+ ydotool 模擬 Ctrl+V。
///
/// 之前用 `wl-copy` shell-out 會在 GNOME 50 的 xdg-desktop-portal
/// 跳「未知 wl-clipboard」權限對話框,使用者每次都要手動點允許 — 改
/// 走 Tauri plugin 之後 Mori 是已註冊的 host app(via portal_hotkey
/// 的 register_host_app),GNOME 就視為 trusted。
pub struct LinuxPasteController {
    app: AppHandle,
}

impl LinuxPasteController {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

#[async_trait]
impl PasteController for LinuxPasteController {
    async fn paste_back(&self, text: &str) -> Result<PasteResult> {
        // ── Step 1:寫入 Wayland 剪貼簿(走 Tauri plugin,不 shell-out)──
        // 這步如果壞了等於 paste-back 整套沒希望(連 user 手動 Ctrl+V
        // 都救不了)。Bail out 為 hard error。
        self.app
            .clipboard()
            .write_text(text.to_string())
            .context("Tauri clipboard write_text (capability allow-write-text granted?)")?;

        // ── Step 2:讓合成器消化一下 selection 變更 ───────────────
        // 太快送 ydotool 偶爾會在 selection 還沒設好之前就觸發 paste,
        // 目標 app 抓到「上一個」剪貼簿值。80ms 夠。
        tokio::time::sleep(Duration::from_millis(80)).await;

        // ── Step 3:ydotool 模擬 Ctrl+V ────────────────────────────
        // 這步比較脆弱(ydotoold 沒跑、user 沒進 input group、ydotool
        // 沒裝)。失敗時**不要 bail** — 文字已經在剪貼簿,user 手動
        // Ctrl+V 還能補上,所以回 `ClipboardOnly` 讓上層友善降級。
        let ydotool_outcome = Command::new("ydotool")
            .args(["key", "29:1", "47:1", "47:0", "29:0"])
            .status();

        match ydotool_outcome {
            Ok(s) if s.success() => {
                tracing::info!(
                    chars = text.chars().count(),
                    "paste-back: wl-copy + ydotool Ctrl+V dispatched",
                );
                Ok(PasteResult::Pasted)
            }
            Ok(s) => {
                tracing::warn!(
                    status = ?s,
                    "ydotool exited non-zero — text in clipboard but paste-key not sent. \
                     Check `systemctl --user status ydotool` and that user is in `input` group.",
                );
                Ok(PasteResult::ClipboardOnly)
            }
            Err(e) => {
                tracing::warn!(
                    ?e,
                    "ydotool failed to spawn — text in clipboard but paste-key not sent. \
                     Run setup-wayland-input.sh and reboot once for input-group membership.",
                );
                Ok(PasteResult::ClipboardOnly)
            }
        }
    }
}

/// 啟動時的健康檢查 — 看 wl-clipboard / ydotool 在不在 PATH,缺什麼
/// 早警告。**不要** fail app — 反白即改寫只是 phase 4C 的功能,沒它
/// Mori 還是能跑(語音、剪貼簿、記憶都不受影響)。只是讓 user 早點
/// 知道為何 paste-back 待會會 fallback 到 ClipboardOnly。
pub fn warn_if_setup_missing() {
    // 寫剪貼簿走 Tauri plugin(arboard),讀 primary 也走 arboard,
    // 兩者都是 in-process X11/XWayland API。剩下的 shell-out 只有 ydotool
    // (Ctrl+V 模擬,沒有更乾淨的替代)。
    for tool in ["ydotool"] {
        let ok = Command::new("which")
            .arg(tool)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            tracing::warn!(
                tool,
                "selection / paste-back tool missing — phase 4C 功能會降級。\
                 跑 yazelin/ubuntu-26.04-setup 的 setup-wayland-input.sh 裝齊。",
            );
        }
    }
}
