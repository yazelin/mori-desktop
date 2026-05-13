//! Linux primary selection + paste-back via xclip shell-out.
//!
//! ## 為什麼是 xclip 不是 arboard / wl-clipboard-rs
//!
//! Mori 強制 `GDK_BACKEND=x11`，跑在 XWayland 環境。XWayland 會把
//! Wayland 剪貼簿透明同步到 X11 selection。
//!
//! - `arboard` 3.6+ 透過 `wl-clipboard-rs` 直接打 Wayland zwlr_data_control
//!   協定 → GNOME portal 看到「不知道是誰在動 clipboard」→ 跳「未知
//!   wl-clipboard 要求剪貼簿存取」對話框。即使 register_host_app 也救不了
//!   （portal 無法把 Wayland-protocol-level 的 client 連回 app ID）。
//! - `xclip` 是純 X11 工具，走 X11 selection API（X server 自己的協定，
//!   走 XWayland）。portal 完全看不到，不會跳對話框。
//!
//! ## 流程
//!
//! - 讀反白：`xclip -selection primary -o`
//! - 寫剪貼簿：`xclip -selection clipboard -i`
//! - 送 paste 鍵:
//!   - **X11 session**(`XDG_SESSION_TYPE=x11`)→ `xdotool key ctrl+v`
//!     不需 daemon / 不需 group 權限,直接走 X server。
//!   - **Wayland session** → `ydotool key 29:1 47:1 47:0 29:0`
//!     需要 `ydotoold` daemon + user 在 `input` group。
//!
//! ## Setup
//!
//! - **Ubuntu 24.04 + X11**:`sudo apt install xclip xdotool` 就夠了,
//!   沒有 daemon 要設、沒有 group 要加。
//! - **Ubuntu 26.04 + Wayland**:`sudo bash setup-wayland-input.sh` from
//!   yazelin/ubuntu-26.04-setup 一次裝 `xclip` + `wl-clipboard` + `ydotool`
//!   + 加 `input` group + 啟 `ydotoold` user service。

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use mori_core::paste::{PasteController, PasteResult};
use tauri::AppHandle;

/// 最大允許的反白字數 — 太長就視為使用者選了整篇，不適合直接送 Whisper /
/// LLM tool args。1500 是經驗值(中文 ~2000 token,加上提示 + 結果輸出
/// 大概 5-6K total,留有餘地給 Groq gpt-oss-120b TPM)。
const MAX_SELECTION_CHARS: usize = 1500;

/// 讀 X11 PRIMARY selection（滑鼠反白文字）— shell-out 給 xclip。
///
/// xclip 是純 X11 工具，透過 XWayland 看到 Wayland 剪貼簿的同步版本，
/// 不碰 Wayland portal，不會跳對話框。失敗 / 反白為空 → 回 None。
pub fn read_primary_selection() -> Option<String> {
    let output = Command::new("xclip")
        .args(["-selection", "primary", "-o"])
        .output()
        .ok()?;
    if !output.status.success() {
        // 反白為空時 xclip 會 exit 1，正常忽略
        tracing::trace!(status = ?output.status, "xclip primary selection empty / unavailable");
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

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
    // 5F: 改用 xclip shell-out 後不再需要 AppHandle（不走 Tauri clipboard plugin）。
    // 保留空 struct 維持 trait object 介面。
    _private: (),
}

impl LinuxPasteController {
    pub fn new(_app: AppHandle) -> Self {
        Self { _private: () }
    }
}

/// Terminal app 用 Ctrl+Shift+V（Ctrl+V 在 terminal 是「送 literal ^V 字元」）。
/// 其他 app 都用 Ctrl+V。比對 process_name（lowercase）的子字串。
fn needs_shift_for_paste(process_name: &str) -> bool {
    let p = process_name.to_lowercase();
    [
        "gnome-terminal", "kgx", "ptyxis",     // GNOME 系列
        "kitty", "alacritty", "wezterm",       // 流行 terminal
        "foot", "tilix", "terminator", "xterm",
        "konsole", "urxvt", "rxvt",
    ]
    .iter()
    .any(|t| p.contains(t))
}

impl LinuxPasteController {
    /// 主要的 paste-back：
    /// 1. profile 設了 `paste_shortcut` → 完全照辦
    /// 2. 沒設 → 用 process name 偵測 terminal vs 一般 app
    /// 3. 偵測失敗（Wayland 原生視窗，xdotool 抓不到）→ fallback Ctrl+V
    pub async fn paste_back_for_process(
        &self,
        text: &str,
        process_name: &str,
        override_shortcut: Option<mori_core::voice_input_profile::PasteShortcut>,
    ) -> Result<PasteResult> {
        // 用 xclip 寫 X11 CLIPBOARD（純 X11，不碰 Wayland portal，不會跳對話框）。
        // XWayland 會把 X11 CLIPBOARD 同步到 Wayland clipboard，所有 app 都拿得到。
        let mut child = Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("spawn xclip — is xclip installed? run setup-wayland-input.sh")?;
        {
            let stdin = child.stdin.as_mut().context("get xclip stdin")?;
            stdin
                .write_all(text.as_bytes())
                .context("write text to xclip stdin")?;
        }
        // xclip fork 後變成 daemon hold selection，不 wait 它（會卡）。
        // 直接放任，selection ownership 會在下一次 xclip 寫入時被取代。
        drop(child);

        tokio::time::sleep(Duration::from_millis(80)).await;

        use mori_core::voice_input_profile::PasteShortcut;
        let use_shift_v = match override_shortcut {
            Some(PasteShortcut::CtrlShiftV) => true,
            Some(PasteShortcut::CtrlV) => false,
            None => needs_shift_for_paste(process_name),
        };
        let label = if use_shift_v { "Ctrl+Shift+V" } else { "Ctrl+V" };

        // X11 session 用 xdotool — 不需 ydotoold daemon、不需 user 在 input
        // group,直接走 X server,Ubuntu 24.04 + X11 開箱可用。
        // Wayland session 仍用 ydotool(XGrabKey 在 Wayland 被擋,paste key
        // 注入也是,只能走 uinput-level 的 ydotool)。
        let on_x11 = crate::x11_hotkey::is_x11_session();
        let outcome = if on_x11 {
            run_xdotool_paste(use_shift_v)
        } else {
            run_ydotool_paste(use_shift_v)
        };

        match outcome {
            Ok(()) => {
                tracing::info!(
                    chars = text.chars().count(),
                    target_process = %process_name,
                    paste_keys = label,
                    tool = if on_x11 { "xdotool" } else { "ydotool" },
                    "paste-back dispatched",
                );
                Ok(PasteResult::Pasted)
            }
            Err(e) => {
                tracing::warn!(
                    ?e,
                    tool = if on_x11 { "xdotool" } else { "ydotool" },
                    "paste-back tool failed — text in clipboard only.",
                );
                Ok(PasteResult::ClipboardOnly)
            }
        }
    }
}

/// X11:`xdotool key ctrl+v` / `ctrl+shift+v`。
/// xdotool 走 X server XTEST extension,無需 daemon 或 group 權限。
fn run_xdotool_paste(use_shift_v: bool) -> Result<()> {
    let key_spec = if use_shift_v { "ctrl+shift+v" } else { "ctrl+v" };
    let status = Command::new("xdotool")
        .args(["key", key_spec])
        .status()
        .context("spawn xdotool — is xdotool installed? sudo apt install xdotool")?;
    if !status.success() {
        anyhow::bail!("xdotool exited {status}");
    }
    Ok(())
}

/// Wayland:`ydotool key 29:1 47:1 47:0 29:0`(Linux keycode 序列)。
/// Linux keycodes:29=Ctrl, 42=Shift, 47=V。
/// 需 ydotoold daemon + user 在 input group。
fn run_ydotool_paste(use_shift_v: bool) -> Result<()> {
    let keys: &[&str] = if use_shift_v {
        &["29:1", "42:1", "47:1", "47:0", "42:0", "29:0"]
    } else {
        &["29:1", "47:1", "47:0", "29:0"]
    };
    let mut cmd = Command::new("ydotool");
    cmd.arg("key");
    for k in keys {
        cmd.arg(k);
    }
    let status = cmd
        .status()
        .context("spawn ydotool — is ydotoold daemon running + user in input group?")?;
    if !status.success() {
        anyhow::bail!("ydotool exited {status}");
    }
    Ok(())
}

#[async_trait]
impl PasteController for LinuxPasteController {
    /// trait 預設方法：不知道 process / 沒 profile override 時 fallback 用 Ctrl+V
    async fn paste_back(&self, text: &str) -> Result<PasteResult> {
        self.paste_back_for_process(text, "", None).await
    }
}

/// 啟動時的健康檢查 — 看必要工具在不在 PATH,缺什麼早警告。**不要**
/// fail app — 反白即改寫只是 phase 4C 的功能,沒它 Mori 還是能跑(語音、
/// 剪貼簿、記憶都不受影響)。只是讓 user 早點知道為何 paste-back 待會
/// 會 fallback 到 ClipboardOnly。
///
/// 依 session type 檢不同工具:
/// - X11(`XDG_SESSION_TYPE=x11`)→ `xclip` + `xdotool`(走 X server,
///   無需 daemon,Ubuntu 24.04 + X11 開箱可用)
/// - Wayland → `xclip` + `ydotool`(需 daemon + input group,setup-wayland-
///   input.sh 一次裝齊)
pub fn warn_if_setup_missing() {
    let on_x11 = crate::x11_hotkey::is_x11_session();
    let required: &[&str] = if on_x11 {
        &["xclip", "xdotool"]
    } else {
        &["xclip", "ydotool"]
    };
    for tool in required {
        let ok = Command::new("which")
            .arg(tool)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            let hint = if on_x11 {
                "sudo apt install xclip xdotool"
            } else {
                "跑 yazelin/ubuntu-26.04-setup 的 setup-wayland-input.sh 裝齊"
            };
            tracing::warn!(
                tool,
                hint,
                "selection / paste-back tool missing — phase 4C 功能會降級。",
            );
        }
    }
}
