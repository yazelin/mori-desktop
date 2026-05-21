//! K5 commands — 對外 API + 整合 K1-K4。
//!
//! 本模組是「時之鳥」對外的單一入口:
//! - K1 [`ReminderStore`]:SQLite 持久層
//! - K2 [`ReminderScheduler`]:tokio-cron-scheduler 背景觸發
//! - K3 [`Notifier`]:桌面通知
//! - K4 [`parser::parse`]:自然語言時間解析
//!
//! [`ReminderService`] 把這 4 條黏成一個 thread-safe service:
//! - mori-tauri AppState 在啟動時建一個 [`Arc<ReminderService>`],註冊進 Tauri Manager
//! - LLM Skill / Tauri command / 內部呼叫者一律從 AppState 拿 Arc clone
//! - on-fire callback 內部把 K3 notifier + store.mark_fired() 串好;一次性 reminder
//!   觸發後 store 自動標 Fired,週期性(cron)reminder 保留 Pending 持續觸發
//!
//! ## 為什麼用 catch_unwind 包 callback?
//!
//! K2 reviewer 建議:`on_fire` callback 跑在 tokio-cron-scheduler 的 worker task,
//! 如果裡面 panic 會把整個 scheduler task 拖死,導致後續所有 reminder 都不響。
//! 我們用 [`std::panic::catch_unwind`] 包 K3 / store call 那段,panic 只 log warn,
//! 不傳染。`AssertUnwindSafe` 是因為 Notifier / store reference 都不是 UnwindSafe
//! (rusqlite Connection 帶內部 mutable state),但我們不在 panic 後繼續用它們
//! (callback 結束就放回 Arc,沒有 partially-mutated state 殘留)。

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::Mutex;

use crate::notifier::Notifier;
use crate::parser;
use crate::scheduler::{OnFireCallback, ReminderScheduler, SchedulerError};
use crate::schema::{Reminder, ReminderError, ReminderStore};

/// 對外錯誤類型 — 包 K4 parse / K1 store / K2 scheduler 三條 error。
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("parse time: {0}")]
    ParseTime(#[from] parser::ParseError),
    #[error("store: {0}")]
    Store(#[from] ReminderError),
    #[error("scheduler: {0}")]
    Schedule(#[from] SchedulerError),
}

/// 整合 K1-K4 的主入口 service。
///
/// 啟動流程([`ReminderService::new`]):
/// 1. open / create SQLite store
/// 2. build on-fire callback(closure capturing notifier + store handle)
/// 3. create scheduler with callback、start scheduler background task
/// 4. load store.list_pending() → 全 schedule 進來
///
/// AppState 應該持有 `Arc<ReminderService>`,所有 method 取 `&self`,可跨 task / thread 共用。
pub struct ReminderService {
    /// SQLite store,wrap 進 tokio Mutex 因為 K1 是同步 API,但我們從 async context call。
    pub store: Arc<Mutex<ReminderStore>>,
    /// K2 scheduler。Arc 因為 callback 內部要再 clone 一份 store ref;scheduler 自身
    /// `&self` 即可 schedule / cancel,不需 Mutex。
    pub scheduler: Arc<ReminderScheduler>,
}

impl ReminderService {
    /// 建 service:open store → build callback → start scheduler → reload pending。
    ///
    /// `db_path` 通常是 `~/.mori/reminders.db`(對齊既有 mori_dir pattern)。
    /// `notifier` 由 caller 預先建好(`Notifier::new("Mori")`),這樣 caller 可
    /// `.with_icon()` 自訂。
    pub async fn new(db_path: &Path, notifier: Notifier) -> Result<Self, CommandError> {
        // 1) Open store
        let store = ReminderStore::open(db_path)?;
        let store = Arc::new(Mutex::new(store));

        // 2) Build on-fire callback。
        //    必須 clone 出 Arc 給 closure capture(原 store / notifier 留給 service struct)。
        //    closure 跑在 tokio-cron-scheduler 的 worker thread,**不在 tokio runtime**
        //    (scheduler 用 std thread driver),所以不能在這裡面 `await`。我們改用
        //    阻塞 lock(store mutex 是 tokio::sync::Mutex,沒有 blocking API)→ 走
        //    `tokio::runtime::Handle::try_current` + `block_in_place`?
        //
        //    實際上 K2 scheduler 的 callback signature 是同步 `Fn(Reminder)`(見
        //    `OnFireCallback = Arc<dyn Fn(Reminder) + Send + Sync>`),所以不能 await。
        //    解法:用 `std::sync::Mutex` 對 store?不行,K1 設計是 tokio Mutex。
        //
        //    最務實:這裡 spawn 一個短命 tokio task 處理「mark_fired + notify」。
        //    `tokio::spawn` 需要 runtime handle;tokio-cron-scheduler 內部本身用 tokio
        //    spawn 跑 jobs,所以 callback 一定在 tokio runtime 內 — `Handle::current()`
        //    能拿到。我們用這個。
        let store_for_cb = Arc::clone(&store);
        let notifier_for_cb = notifier.clone();
        let on_fire: OnFireCallback = Arc::new(move |reminder: Reminder| {
            let store_inner = Arc::clone(&store_for_cb);
            let notifier_inner = notifier_for_cb.clone();
            // reminder.id 在 panic-log path 也要,先抓出來避免 move ownership 衝突
            let reminder_id = reminder.id;
            // 包 catch_unwind:不讓 panic 拖死整個 scheduler task。
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                tokio::spawn(async move {
                    // K3:發桌面通知。失敗 log warn,不 throw。
                    if let Err(e) = notifier_inner.fire(&reminder) {
                        tracing::warn!(
                            reminder_id = reminder.id,
                            error = %e,
                            "notifier.fire failed (reminder still mark_fired)",
                        );
                    }
                    // 一次性 reminder:fire 後 store 標 Fired。週期性(cron)保留 Pending。
                    if reminder.cron_expr.is_none() {
                        let store = store_inner.lock().await;
                        if let Err(e) = store.mark_fired(reminder.id, Utc::now()) {
                            tracing::warn!(
                                reminder_id = reminder.id,
                                error = %e,
                                "mark_fired failed",
                            );
                        }
                    }
                });
            }));
            if let Err(panic) = result {
                tracing::warn!(
                    reminder_id,
                    panic = ?panic,
                    "on_fire callback panicked (suppressed to keep scheduler alive)",
                );
            }
        });

        // 3) Create scheduler + start
        let scheduler = ReminderScheduler::new(Arc::clone(&store), on_fire).await?;
        scheduler.start().await?;
        let scheduler = Arc::new(scheduler);

        // 4) Reload pending — restart 後把 DB 裡未完成的 reminder 重新 schedule。
        //    用 due_at vs now 判斷:過去的 one-shot 立刻觸發(scheduler 內部處理)。
        let pending = store.lock().await.list_pending()?;
        for r in &pending {
            if let Err(e) = scheduler.schedule(r).await {
                tracing::warn!(
                    reminder_id = r.id,
                    error = %e,
                    "failed to schedule pending reminder on startup",
                );
            }
        }
        tracing::info!(
            pending_count = pending.len(),
            "ReminderService started, pending reminders rescheduled",
        );

        Ok(Self { store, scheduler })
    }

    /// 設一個一次性 reminder。`when_expr` 走 K4 parser(中/英文 NL)。
    ///
    /// 流程:parse NL → store.create → scheduler.schedule。
    pub async fn remind_me(
        &self,
        text: String,
        when_expr: String,
    ) -> Result<Reminder, CommandError> {
        let due_at = parser::parse(&when_expr)?;
        let reminder = {
            let store = self.store.lock().await;
            store.create(text, due_at, None)?
        };
        self.scheduler.schedule(&reminder).await?;
        Ok(reminder)
    }

    /// 設一個週期性 reminder(cron expression)。
    ///
    /// `cron_expr` 必須是 6-field cron(含秒),例:`"0 0 8 * * *"` = 每天 08:00。
    /// `due_at` 設成 now + 1 分鐘(symbolic placeholder — scheduler 內部走 cron,
    /// 不靠 `due_at`;但 store schema 要求非 null)。
    pub async fn remind_me_cron(
        &self,
        text: String,
        cron_expr: String,
    ) -> Result<Reminder, CommandError> {
        let placeholder_due = Utc::now() + chrono::Duration::minutes(1);
        let reminder = {
            let store = self.store.lock().await;
            store.create(text, placeholder_due, Some(cron_expr))?
        };
        self.scheduler.schedule(&reminder).await?;
        Ok(reminder)
    }

    /// 列出未完成 reminder(Pending + Snoozed)。
    pub async fn list_reminders(&self) -> Result<Vec<Reminder>, CommandError> {
        let store = self.store.lock().await;
        Ok(store.list_pending()?)
    }

    /// 取消 reminder。同時取消 scheduler 內的 job + 把 store 標 Cancelled。
    pub async fn cancel_reminder(&self, id: i64) -> Result<(), CommandError> {
        self.scheduler.cancel(id).await?;
        let store = self.store.lock().await;
        store.cancel(id)?;
        Ok(())
    }

    /// 把現有 reminder snooze 到指定時間。`when_expr` 同 [`remind_me`] 走 NL parser。
    ///
    /// 內部:parse → store.snooze(更新 status + snoozed_until)→ scheduler 重 schedule。
    /// 注意:K2 schedule 是用 reminder.due_at 算 one-shot duration,所以我們在
    /// schedule 前把 reminder.due_at 更新成新 until,讓 scheduler 用新時間。
    pub async fn snooze_reminder(
        &self,
        id: i64,
        when_expr: String,
    ) -> Result<(), CommandError> {
        let until = parser::parse(&when_expr)?;
        // 先 cancel scheduler 內舊 job(避免兩個 timer 並存)
        self.scheduler.cancel(id).await?;
        // 更新 store
        let updated = {
            let store = self.store.lock().await;
            store.snooze(id, until)?;
            // 拿出更新後完整 reminder(包含新 snoozed_until + Snoozed status)。
            // K2 scheduler 用的是 due_at,不是 snoozed_until — 但 snooze 行為是「延後觸發」,
            // 我們在 schedule call 前把 reminder.due_at 改成 `until` 來達到延後效果。
            let mut r = store.get(id)?;
            r.due_at = until;
            r
        };
        self.scheduler.schedule(&updated).await?;
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Integration-style tests — 真的用 ReminderStore + ReminderScheduler。
    //! Notifier 用 `Notifier::new("Mori")`;`.fire()` 在 CI 沒 dbus 會 Err,
    //! 但 service 不卡 fire error(只 log),test 仍能驗證「mark_fired」是否走完。
    //!
    //! 一次性 reminder 的 fire 時序:scheduler tick 500ms + service spawn task,
    //! 所以 due-after-100ms 的 reminder 要等 ~1.5s 才看到 store 變 Fired。
    //!
    //! 全用 tempfile DB(`ReminderStore::open(path)`),避免 in-memory store 在
    //! `new()` migrate 後不能跨 Arc<Mutex>(其實 sqlite in-memory 也行,但 `new()`
    //! 收 path,跑得最像 prod path)。
    //!
    //! `service_loads_pending_on_startup` 對齊 P0 行為:重啟後 pending reminder
    //! 不會掉。建一條、close service、新開 service、確認新 service 的 list 含這條。
    //! 在 0.5s 內檢查就行(scheduler 重新 schedule 過,fire timing 跟 startup 距離無關)。
    use super::*;
    use crate::schema::ReminderStatus;
    use chrono::Duration;
    use std::time::Duration as StdDuration;
    use tempfile::TempDir;

    fn fresh_notifier() -> Notifier {
        Notifier::new("Mori-Test")
    }

    async fn service_in(dir: &TempDir) -> ReminderService {
        let db = dir.path().join("reminders.db");
        ReminderService::new(&db, fresh_notifier())
            .await
            .expect("new service")
    }

    #[tokio::test]
    async fn service_new_creates_db_and_starts_scheduler() {
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        // DB 檔案被建出來
        assert!(dir.path().join("reminders.db").exists());
        // list_reminders 不 panic 且回空
        let pending = svc.list_reminders().await.expect("list ok");
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn remind_me_creates_and_schedules_reminder() {
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        let r = svc
            .remind_me("喝水".into(), "1 hour".into())
            .await
            .expect("remind_me ok");
        assert!(r.id > 0);
        assert_eq!(r.text, "喝水");
        assert_eq!(r.status, ReminderStatus::Pending);
        // list 應包含這條
        let pending = svc.list_reminders().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, r.id);
    }

    #[tokio::test]
    async fn remind_me_parses_nl_time_chinese() {
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        // 30 分鐘後 should parse via K4 Chinese fallback
        let r = svc
            .remind_me("散步".into(), "30 分鐘後".into())
            .await
            .expect("remind_me parses 中文");
        // due_at should be ~30 min from now;允許 ±2 分鐘漂移
        let now = Utc::now();
        let diff = (r.due_at - now).num_seconds();
        assert!(
            (1680..=1920).contains(&diff),
            "expected ~1800s in future, got {diff}s",
        );
    }

    #[tokio::test]
    async fn remind_me_returns_parse_error_for_unrecognized() {
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        let err = svc
            .remind_me("X".into(), "qwerty foobar".into())
            .await
            .expect_err("garbage time should fail");
        assert!(
            matches!(err, CommandError::ParseTime(_)),
            "expected ParseTime, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn list_reminders_returns_pending_only() {
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        let r1 = svc
            .remind_me("a".into(), "1 hour".into())
            .await
            .unwrap();
        let r2 = svc
            .remind_me("b".into(), "2 hours".into())
            .await
            .unwrap();
        // cancel r2 → list 只剩 r1
        svc.cancel_reminder(r2.id).await.unwrap();
        let pending = svc.list_reminders().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, r1.id);
    }

    #[tokio::test]
    async fn cancel_reminder_cancels_in_store_and_scheduler() {
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        let r = svc
            .remind_me("ghost".into(), "30 minutes".into())
            .await
            .unwrap();
        svc.cancel_reminder(r.id).await.expect("cancel ok");
        // store status → Cancelled
        let after = svc.store.lock().await.get(r.id).unwrap();
        assert_eq!(after.status, ReminderStatus::Cancelled);
        // list_pending 不再包含
        let pending = svc.list_reminders().await.unwrap();
        assert!(pending.iter().all(|x| x.id != r.id));
    }

    #[tokio::test]
    async fn snooze_reminder_updates_store_and_reschedules() {
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        let r = svc
            .remind_me("snz".into(), "1 hour".into())
            .await
            .unwrap();
        svc.snooze_reminder(r.id, "2 hours".into())
            .await
            .expect("snooze ok");
        let after = svc.store.lock().await.get(r.id).unwrap();
        assert_eq!(after.status, ReminderStatus::Snoozed);
        assert!(after.snoozed_until.is_some());
        // snoozed reminder 仍在 list_pending(因為 list_pending 含 Snoozed)
        let pending = svc.list_reminders().await.unwrap();
        assert!(pending.iter().any(|x| x.id == r.id));
    }

    #[tokio::test]
    async fn service_loads_pending_on_startup() {
        // 第一次:建 reminder + close service(只是 drop)
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("reminders.db");
        let r_id = {
            let svc = ReminderService::new(&db, fresh_notifier()).await.unwrap();
            let r = svc
                .remind_me("persistent".into(), "1 hour".into())
                .await
                .unwrap();
            r.id
        };
        // 第二次:重新開 service,pending 應該被 reload
        let svc2 = ReminderService::new(&db, fresh_notifier()).await.unwrap();
        let pending = svc2.list_reminders().await.unwrap();
        assert_eq!(pending.len(), 1, "expected 1 reloaded pending, got {}", pending.len());
        assert_eq!(pending[0].id, r_id);
        assert_eq!(pending[0].text, "persistent");
    }

    #[tokio::test]
    async fn one_shot_reminder_fires_and_marks_store() {
        // 端對端 happy path:設 due 在 200ms 之後,等 ~2s 看 store 變 Fired。
        // notifier.fire 在 CI 沒 dbus 會 Err,但 service callback 內部 log warn
        // 不擋 mark_fired。
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        // 用 fixed due_at instead of NL — 200ms in future
        let due = Utc::now() + Duration::milliseconds(200);
        let r = {
            let store = svc.store.lock().await;
            store.create("ping".into(), due, None).unwrap()
        };
        svc.scheduler.schedule(&r).await.unwrap();
        // 等 scheduler tick + service spawn task
        tokio::time::sleep(StdDuration::from_millis(2000)).await;
        let after = svc.store.lock().await.get(r.id).unwrap();
        assert_eq!(
            after.status,
            ReminderStatus::Fired,
            "expected Fired after due passed, got {:?}",
            after.status,
        );
        assert!(after.fired_at.is_some());
    }
}
