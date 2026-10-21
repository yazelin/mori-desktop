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
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardSequenceNumber, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
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
        // 貼上前先備份使用者原本複製的內容,貼完再還原(#185)。
        let backup = snapshot_clipboard().filter(|s| !s.formats.is_empty());

        // 寫剪貼簿 — Tauri plugin 在 Windows 上是 `OpenClipboard` + `SetClipboardData`
        // 標準 Win32 API,沒任何權限對話框問題。
        self.app
            .clipboard()
            .write_text(text.to_string())
            .context("Tauri clipboard write_text — open/set clipboard failed")?;

        // 小睡讓 clipboard ownership 穩定(跟 Linux 80ms 同理)。
        tokio::time::sleep(Duration::from_millis(80)).await;
        // 記下我們寫入後的序號;還原前序號變了,代表使用者或別的程式又複製了東西。
        let seq_after_write = unsafe { GetClipboardSequenceNumber() };

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
                if let Some(snap) = backup {
                    schedule_clipboard_restore(snap, seq_after_write);
                }
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

/// 貼上後等多久再還原剪貼簿。目標程式收到 Ctrl+V 後才會去讀剪貼簿,太早還原
/// 會貼到舊內容。競品實測約 1.2 秒才完成,這裡取 700ms 再加 SendInput 本身的時間。
const CLIPBOARD_RESTORE_DELAY_MS: u64 = 700;

/// 備份上限。複製了超大圖片時不備份超過這個量,避免貼上前卡住。
const MAX_BACKUP_BYTES: usize = 64 * 1024 * 1024;

/// 備份下來的剪貼簿:每個格式一份位元組。
struct ClipboardSnapshot {
    formats: Vec<(u32, Vec<u8>)>,
}

/// 只備份資料放在 HGLOBAL 的格式。GDI 物件(CF_BITMAP、CF_PALETTE、
/// CF_METAFILEPICT、CF_ENHMETAFILE)與顯示用、私有格式的資料不是記憶體區塊,
/// 不能這樣複製;CF_BITMAP 會由系統從 CF_DIB 自動合成,所以還原 CF_DIB 就夠。
fn is_hglobal_format(fmt: u32) -> bool {
    !matches!(fmt, 2 | 3 | 9 | 14 | 0x80..=0x83 | 0x8E | 0x200..=0x3FF)
}

/// 開著剪貼簿的期間才能讀寫;離開作用域自動關閉。
struct ClipboardGuard;

impl ClipboardGuard {
    /// 別的程式正在使用剪貼簿時會開失敗,短暫重試幾次。
    fn open() -> Option<Self> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(HWND::default()) }.is_ok() {
                return Some(Self);
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        None
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn snapshot_clipboard() -> Option<ClipboardSnapshot> {
    let _guard = ClipboardGuard::open()?;
    let mut formats = Vec::new();
    let mut total = 0usize;
    let mut fmt = 0u32;
    loop {
        fmt = unsafe { EnumClipboardFormats(fmt) };
        if fmt == 0 {
            break;
        }
        if !is_hglobal_format(fmt) {
            continue;
        }
        let Ok(handle) = (unsafe { GetClipboardData(fmt) }) else {
            continue;
        };
        let hglobal = HGLOBAL(handle.0);
        let size = unsafe { GlobalSize(hglobal) };
        if size == 0 || total + size > MAX_BACKUP_BYTES {
            continue;
        }
        let ptr = unsafe { GlobalLock(hglobal) };
        if ptr.is_null() {
            continue;
        }
        let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, size) }.to_vec();
        unsafe {
            let _ = GlobalUnlock(hglobal);
        }
        total += size;
        formats.push((fmt, bytes));
    }
    Some(ClipboardSnapshot { formats })
}

/// 清空剪貼簿後依序寫回備份的格式。回傳成功寫回的格式數。
fn restore_clipboard(snap: &ClipboardSnapshot) -> usize {
    let Some(_guard) = ClipboardGuard::open() else {
        return 0;
    };
    if unsafe { EmptyClipboard() }.is_err() {
        return 0;
    }
    let mut restored = 0;
    for (fmt, bytes) in &snap.formats {
        unsafe {
            let Ok(hglobal) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else {
                continue;
            };
            let ptr = GlobalLock(hglobal);
            if ptr.is_null() {
                let _ = GlobalFree(hglobal);
                continue;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
            let _ = GlobalUnlock(hglobal);
            // 成功後記憶體歸系統;失敗要自己釋放。
            if SetClipboardData(*fmt, HANDLE(hglobal.0)).is_ok() {
                restored += 1;
            } else {
                let _ = GlobalFree(hglobal);
            }
        }
    }
    restored
}

/// 等目標程式貼完之後還原。還原前序號變了就放棄,不蓋掉使用者新複製的內容。
fn schedule_clipboard_restore(snap: ClipboardSnapshot, seq_after_write: u32) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(CLIPBOARD_RESTORE_DELAY_MS)).await;
        let current = unsafe { GetClipboardSequenceNumber() };
        let wanted = snap.formats.len();
        let (ok, restored, reason) = if current != seq_after_write {
            (false, 0, "clipboard_changed")
        } else {
            let n = restore_clipboard(&snap);
            (n > 0, n, if n > 0 { "restored" } else { "restore_failed" })
        };
        tracing::info!(ok, restored, wanted, reason, "clipboard restore after paste");
        mori_core::event_log::append(serde_json::json!({
            "kind": "clipboard_restore",
            "ok": ok,
            "reason": reason,
            "formats_wanted": wanted,
            "formats_restored": restored,
        }));
    });
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

#[cfg(test)]
mod tests {
    use super::is_hglobal_format;

    #[test]
    fn text_and_dib_are_backed_up() {
        assert!(is_hglobal_format(1)); // CF_TEXT
        assert!(is_hglobal_format(8)); // CF_DIB
        assert!(is_hglobal_format(13)); // CF_UNICODETEXT
        assert!(is_hglobal_format(15)); // CF_HDROP(檔案清單)
        assert!(is_hglobal_format(0xC123)); // 註冊格式(例如 HTML Format)
    }

    #[test]
    fn gdi_and_private_formats_are_skipped() {
        for f in [2u32, 3, 9, 14, 0x80, 0x82, 0x8E, 0x200, 0x2FF, 0x300, 0x3FF] {
            assert!(!is_hglobal_format(f), "format {f:#x} should be skipped");
        }
    }
}
