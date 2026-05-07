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

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use mori_core::paste::PasteController;

/// 最大允許的反白字數 — 太長就視為使用者選了整篇，不適合直接送 Whisper /
/// LLM tool args。1500 是經驗值(中文 ~2000 token,加上提示 + 結果輸出
/// 大概 5-6K total,留有餘地給 Groq gpt-oss-120b TPM)。
const MAX_SELECTION_CHARS: usize = 1500;

/// 讀 Wayland primary selection(滑鼠反白文字)。
/// 失敗時回 None — 反白為空、wl-paste 不在 PATH、Wayland session 沒有
/// primary 等情況都當「沒抓到」處理,**不要** fatal。
pub fn read_primary_selection() -> Option<String> {
    let output = Command::new("wl-paste")
        .args(["--primary", "--no-newline"])
        .output()
        .ok()?;

    if !output.status.success() {
        // 反白為空時 wl-paste 會 exit code != 0,完全正常,不要 warn。
        return None;
    }

    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Cap 過長的選取(整篇文章不該整個塞 LLM context)。
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

/// PasteController 的 Linux 實作:`wl-copy <text> && ydotool key Ctrl+V`。
pub struct LinuxPasteController;

#[async_trait]
impl PasteController for LinuxPasteController {
    async fn paste_back(&self, text: &str) -> Result<()> {
        // 1) 把結果寫到 Wayland 系統剪貼簿(透過 wl-copy)。我們用阻塞
        //    版本 — wl-copy 預設會 fork 一個 daemon 把資料留在剪貼簿,
        //    main process 立刻 return,所以 spawn → write stdin → drop
        //    OK。
        let mut child = Command::new("wl-copy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("spawn wl-copy (is wl-clipboard installed? run setup-wayland-input.sh)")?;

        {
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| anyhow!("wl-copy stdin"))?;
            stdin.write_all(text.as_bytes()).context("write to wl-copy stdin")?;
        }
        let status = child.wait().context("wl-copy wait")?;
        if !status.success() {
            anyhow::bail!("wl-copy exited non-zero ({status})");
        }

        // 2) 給合成器一拍呼吸時間 — 太快送 ydotool 偶爾會在 wl-copy 的
        //    daemon 還沒設好 selection 之前就觸發 paste,目標 app 抓到的
        //    是「上一個」剪貼簿值。50ms 通常夠。
        tokio::time::sleep(Duration::from_millis(80)).await;

        // 3) ydotool 用 Linux input-event keycode 送 Ctrl+V。
        //    29 = LEFT_CTRL, 47 = V。`:1` 表示按下,`:0` 表示放開。
        //    順序:Ctrl down → V down → V up → Ctrl up,完整 paste。
        let status = Command::new("ydotool")
            .args(["key", "29:1", "47:1", "47:0", "29:0"])
            .status()
            .context(
                "ydotool key send (is ydotoold daemon running? \
                 systemctl --user status ydotool)",
            )?;
        if !status.success() {
            anyhow::bail!("ydotool exited non-zero ({status})");
        }

        tracing::info!(
            chars = text.chars().count(),
            "paste-back: wl-copy + ydotool Ctrl+V dispatched",
        );
        Ok(())
    }
}
