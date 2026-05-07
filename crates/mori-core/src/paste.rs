//! Paste-back primitive for "select + voice + replace" workflows.
//!
//! Phase 4C 主軸:使用者反白 → 講話 → Mori 處理 → 結果**回填到原本反白**。
//! 同 Mode 一樣用 trait 抽象,讓 mori-core 的 skill 不依賴 Tauri / 平台。
//! mori-tauri 在 Linux 上實作為 `wl-copy <text> && ydotool key Ctrl+V`。
//!
//! 沒實作的平台(macOS / Windows)先回 `Err`,UI 顯示「請手動 Cmd/Ctrl+V
//! 貼上」即可。

use async_trait::async_trait;

#[async_trait]
pub trait PasteController: Send + Sync {
    /// 把 `text` 寫進剪貼簿,然後模擬 paste 鍵(Linux 上是 Ctrl+V)送到
    /// 當下 focused 的視窗。**這個 trait 不負責「保存原視窗 focus」** —
    /// 呼叫者要保證在呼叫前 focused 的就是要被貼回去的那一個視窗
    /// (例如 Mori 不偷焦點的設計就是為了維持這點)。
    async fn paste_back(&self, text: &str) -> anyhow::Result<()>;
}
