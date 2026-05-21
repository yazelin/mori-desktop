//! mori-time — 時之鳥(本機 reminder + cron)
//!
//! 「時之鳥」是 Mori 在用戶世界裡的計時靈鳥 — 替用戶記住一次性或週期性的事項,
//! 到時鳴叫提醒。完全本地執行,vault-friendly,不依賴任何雲端排程服務。
//!
//! 5 sub-streams:
//! - K1(本 stream):schema + CRUD(this module: [`schema`])
//! - K2: [`scheduler`] — tokio-cron-scheduler 整合(背景觸發)
//! - K3: [`notifier`] — notify-rust 桌面通知
//! - K4: [`parser`] — chrono-english 自然語言時間解析
//! - K5: [`commands`] — Tauri 命令(remind_me / list_reminders / cancel_reminder / snooze_reminder)
//!
//! K1 ship 後其他 sub-streams 可並行接,不會 module conflict。

pub mod schema;
pub mod scheduler;
pub mod notifier;
pub mod parser;
pub mod commands;

pub use schema::{Reminder, ReminderError, ReminderStatus, ReminderStore};
pub use scheduler::{ReminderScheduler, SchedulerError};
pub use notifier::{Notifier, NotifyError};
pub use commands::{CommandError, ReminderService};
