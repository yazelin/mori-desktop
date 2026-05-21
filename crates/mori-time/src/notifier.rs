//! K3 notifier — 待實作(notify-rust 桌面通知)
//!
//! TODO: 由 K3 stream impl。本 stub 確保 K1 ship 後 `lib.rs` 不 break。
//!
//! 預期 API(by K3):
//! - `notify(reminder: &Reminder) -> Result<(), NotifyError>`
//! - 跨平台:Linux(libnotify)/ Windows(toast)/ macOS(NSUserNotification)
//! - 由 K2 scheduler 觸發
