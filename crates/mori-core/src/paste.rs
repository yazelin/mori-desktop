//! Paste-back primitive for "select + voice + replace" workflows.
//!
//! Phase 4C 主軸:使用者反白 → 講話 → Mori 處理 → 結果**回填到原本反白**。
//! 同 Mode 一樣用 trait 抽象,讓 mori-core 的 skill 不依賴 Tauri / 平台。
//! mori-tauri 在 Linux 上實作為 `wl-copy <text> && ydotool key Ctrl+V`。
//!
//! 沒實作的平台(macOS / Windows)先回 `Err`,UI 顯示「請手動 Cmd/Ctrl+V
//! 貼上」即可。

use async_trait::async_trait;

/// `paste_back` 的兩段式結果。剪貼簿寫入幾乎不會失敗(整套基本前提);
/// 反而是模擬 paste 那步比較脆弱(ydotoold 沒跑、input group 沒生效、
/// 系統沒裝 ydotool 等)。把兩段拆開讓 UI / skill 可以給「**結果已放剪
/// 貼簿,請手動 Ctrl+V 貼上**」這種 graceful fallback,而不是 hard fail。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteResult {
    /// 剪貼簿寫成功 + paste-key 模擬成功 → 已貼回原視窗。
    Pasted,
    /// 剪貼簿寫成功,但 paste-key 模擬失敗。文字仍在剪貼簿裡,user 手動
    /// Ctrl+V 還是能補上。
    ClipboardOnly,
}

#[async_trait]
pub trait PasteController: Send + Sync {
    /// 把 `text` 寫進剪貼簿,然後嘗試模擬 paste 鍵(Linux 上是 Ctrl+V)
    /// 送到當下 focused 的視窗。**這個 trait 不負責「保存原視窗 focus」**
    /// — 呼叫者要保證在呼叫前 focused 的就是要被貼回去的視窗
    /// (例如 Mori 不偷焦點的設計就是為了維持這點)。
    ///
    /// `Err` 只用在「**剪貼簿都寫不進去**」這種根本性失敗。Paste-key
    /// 模擬失敗 → 回 `Ok(ClipboardOnly)`,讓上層決定怎麼跟 user 講。
    async fn paste_back(&self, text: &str) -> anyhow::Result<PasteResult>;
}
