//! K2 scheduler — 待實作(tokio-cron-scheduler 整合)
//!
//! TODO: 由 K2 stream impl。本 stub 確保 K1 ship 後 `lib.rs` 不 break。
//!
//! 預期 API(by K2):
//! - `ReminderScheduler::new(store: Arc<ReminderStore>) -> Self`
//! - `start()` — 掃 store.list_pending() 排程進 tokio-cron-scheduler
//! - `reload()` — store CRUD 後 re-sync
//! - 到點觸發 → 呼叫 K3 notifier + store.mark_fired()
