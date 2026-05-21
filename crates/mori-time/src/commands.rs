//! K5 commands — 待實作(Tauri command 包裝)
//!
//! TODO: 由 K5 stream impl。本 stub 確保 K1 ship 後 `lib.rs` 不 break。
//!
//! 預期 Tauri commands(by K5):
//! - `remind_me(text: String, when: String) -> Result<Reminder, String>`
//! - `list_reminders() -> Result<Vec<Reminder>, String>`
//! - `cancel_reminder(id: i64) -> Result<(), String>`
//! - `snooze_reminder(id: i64, minutes: i64) -> Result<(), String>`
//!
//! K5 會在 `crates/mori-tauri/src/lib.rs` 把這些 register 進 Tauri invoke handler。
