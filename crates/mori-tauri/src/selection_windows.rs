//! Windows 剪貼簿 + paste-back primitive。Linux X11 PRIMARY selection 在
//! Windows 沒有對等概念(Windows 只有「Ctrl+C 之後才在 clipboard」的
//! 模式,沒有「滑鼠反白即可讀」),所以 `read_primary_selection` 一律
//! 回 `None`。要做反白即改寫的 user 自己先 Ctrl+C。
//!
//! Paste-back:Tauri clipboard plugin 寫剪貼簿 + Win32 `SendInput` 模擬
//! Ctrl+V / Ctrl+Shift+V。`SendInput` 是 Microsoft 推薦的 modern API
//! (老的 `keybd_event` 已 deprecated)。不需 daemon、不需 user 加 group,
//! 開箱即用。

use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use mori_core::paste::{PasteController, PasteResult};
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_RETURN, VK_SHIFT, VK_V,
};

/// Windows 沒有 X11 PRIMARY selection 那種「反白即可讀」概念。
/// 一律回 None;ContextProvider 會 fall through,不影響功能。
/// (UI Automation 也能拿但會跨 app 不穩,先不做。)
pub fn read_primary_selection() -> Option<String> {
    None
}

/// PasteController 的 Windows 實作:Tauri clipboard plugin 寫剪貼簿
/// (跨平台抽象,Windows 上走 `OpenClipboard` + `SetClipboardData`)
/// + `SendInput` 模擬 Ctrl+V。
pub struct WindowsPasteController {
    app: AppHandle,
}

impl WindowsPasteController {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    /// 跟 Linux 版相同 signature — main.rs 兩條 path 共用。
    /// 1. profile 設了 `paste_shortcut` → 完全照辦
    /// 2. 沒設 → 用 process name 偵測 terminal vs 一般 app
    pub async fn paste_back_for_process(
        &self,
        text: &str,
        process_name: &str,
        override_shortcut: Option<mori_core::voice_input_profile::PasteShortcut>,
    ) -> Result<PasteResult> {
        // 寫剪貼簿 — Tauri plugin 在 Windows 上是 `OpenClipboard` + `SetClipboardData`
        // 標準 Win32 API,沒任何權限對話框問題。
        self.app
            .clipboard()
            .write_text(text.to_string())
            .context("Tauri clipboard write_text — open/set clipboard failed")?;

        // 小睡讓 clipboard ownership 穩定(跟 Linux 80ms 同理)。
        tokio::time::sleep(Duration::from_millis(80)).await;

        use mori_core::voice_input_profile::PasteShortcut;
        let use_shift_v = match override_shortcut {
            Some(PasteShortcut::CtrlShiftV) => true,
            Some(PasteShortcut::CtrlV) => false,
            None => needs_shift_for_paste(process_name),
        };
        let label = if use_shift_v { "Ctrl+Shift+V" } else { "Ctrl+V" };

        match send_paste(use_shift_v) {
            Ok(()) => {
                tracing::info!(
                    chars = text.chars().count(),
                    target_process = %process_name,
                    paste_keys = label,
                    tool = "SendInput",
                    "paste-back dispatched",
                );
                Ok(PasteResult::Pasted)
            }
            Err(e) => {
                tracing::warn!(
                    ?e,
                    tool = "SendInput",
                    "paste-back SendInput failed — text in clipboard only.",
                );
                Ok(PasteResult::ClipboardOnly)
            }
        }
    }
}

/// Windows terminal 偵測 — Windows Terminal / pwsh / cmd 等多數情境下
/// Ctrl+V 已被支援(舊版 cmd 例外),但保留 Ctrl+Shift+V fallback 給
/// cross-platform terminal 跟 conpty/Windows Terminal 偏好的 user。
fn needs_shift_for_paste(process_name: &str) -> bool {
    let p = process_name.to_lowercase();
    [
        "windowsterminal", "wt",                 // Windows Terminal
        "alacritty", "kitty", "wezterm",         // 跨平台 terminal
        "mintty",                                // Git Bash / Cygwin
        "conemu", "cmder",                       // 第三方 console
    ]
    .iter()
    .any(|t| p.contains(t))
}

/// SendInput Ctrl+V (or Ctrl+Shift+V) 序列。
/// 用 KEYEVENTF_KEYUP 分 down/up,modifiers 先按下、V 點一下、modifiers 放開。
fn send_paste(use_shift: bool) -> Result<()> {
    let mut inputs: Vec<INPUT> = Vec::with_capacity(6);

    inputs.push(make_key(VK_CONTROL, false));
    if use_shift {
        inputs.push(make_key(VK_SHIFT, false));
    }
    inputs.push(make_key(VK_V, false));
    inputs.push(make_key(VK_V, true));
    if use_shift {
        inputs.push(make_key(VK_SHIFT, true));
    }
    inputs.push(make_key(VK_CONTROL, true));

    let expected = inputs.len() as u32;
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != expected {
        anyhow::bail!("SendInput injected {sent}/{expected} events");
    }
    Ok(())
}

fn make_key(vk: VIRTUAL_KEY, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if key_up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[async_trait]
impl PasteController for WindowsPasteController {
    /// trait 預設方法 — 沒 profile override / 不知道 process 時 fallback Ctrl+V
    async fn paste_back(&self, text: &str) -> Result<PasteResult> {
        self.paste_back_for_process(text, "", None).await
    }
}

/// 平台抽象別名 — main.rs 透過這個用,不直接綁 Linux/Windows。
pub type PlatformPasteController = WindowsPasteController;

/// auto-enter:SendInput VK_RETURN press+release。
/// 失敗只 warn 不 bail — auto-enter 是可選步驟,失敗 user 自己按 Enter 即可。
pub fn send_enter() {
    let inputs = [make_key(VK_RETURN, false), make_key(VK_RETURN, true)];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent == inputs.len() as u32 {
        tracing::debug!("auto-enter sent via SendInput");
    } else {
        tracing::warn!(sent, expected = inputs.len(), "auto-enter SendInput partial");
    }
}

/// Windows 啟動時不需要外部工具檢查(Win32 API built-in)。保留 entry
/// point 讓 main.rs 跨平台 call 同一個函式。
pub fn warn_if_setup_missing() {
    tracing::debug!("Windows paste-back uses built-in Win32 SendInput — no external tools needed");
}
