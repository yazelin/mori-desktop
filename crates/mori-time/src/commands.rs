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
//! ## on_fire panic 怎麼處理?
//!
//! `on_fire` callback 跑在 tokio-cron-scheduler 的 worker task。我們把實際工作
//! (notifier.fire + store.mark_fired)`tokio::spawn` 進一個獨立 task,**仰賴
//! tokio 的 task 隔離**:單一 spawned task 內 panic 只殺那條 task,不會 abort
//! tokio runtime,也不會傳染回 scheduler。
//!
//! 早前版本曾用 `std::panic::catch_unwind` 包 `tokio::spawn(...)`,但那個包法只
//! 罩 `tokio::spawn` 自己(基本不會 panic),罩不到 spawned async block 內的
//! `notifier.fire` / `store.mark_fired` — async work 被 spawn 出去後,catch_unwind
//! 已經 return 完了。砍掉那層誤導死防護。要看 panic 細節可從 tokio task panic
//! log 取(tracing-subscriber 預設會 emit)。

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::Mutex;

use crate::notifier::Notifier;
use crate::parser;
use crate::scheduler::{OnFireCallback, ReminderScheduler, SchedulerError};
use crate::schema::{Reminder, ReminderError, ReminderStore};

/// 2026-05-22:on_fire 觸發時對外 emit 通知事件。設計成 trait 是為了讓 mori-time
/// 不直接依賴 tauri(會循環依賴)— `mori-tauri` 那邊用 Tauri AppHandle 實作這個 trait,
/// 測試環境可以 mock。
pub trait EventEmitter: Send + Sync {
    /// Emit `reminder-fire-show` event 帶 payload。失敗回 Err(只 log warn,不擋 mark_fired)。
    fn emit_reminder_fire(&self, reminder: &Reminder) -> Result<(), String>;
}

/// no-op 實作,給沒 emit 需求的 caller(例如純 unit test)用。
pub struct NoopEmitter;
impl EventEmitter for NoopEmitter {
    fn emit_reminder_fire(&self, _reminder: &Reminder) -> Result<(), String> {
        Ok(())
    }
}

/// 對外錯誤類型 — 包 K4 parse / K1 store / K2 scheduler 三條 error。
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("parse time: {0}")]
    ParseTime(#[from] parser::ParseError),
    #[error("store: {0}")]
    Store(#[from] ReminderError),
    #[error("scheduler: {0}")]
    Schedule(#[from] SchedulerError),
    /// snooze 等狀態變更動作收到非 Pending / Snoozed reminder。
    /// `current` 是 `ReminderStatus` 的 Debug 字串(`"Fired"` / `"Cancelled"`),
    /// 對 user 而言夠可讀,也方便 LLM tool 回傳 / 前端顯示。
    #[error("reminder {id} can't be snoozed (status: {current})")]
    InvalidStatus { id: i64, current: String },
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
    pub async fn new(
        db_path: &Path,
        notifier: Notifier,
        emitter: Arc<dyn EventEmitter>,
    ) -> Result<Self, CommandError> {
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
        let emitter_for_cb = Arc::clone(&emitter); // 2026-05-22:capture for in-app popup emit
        let on_fire: OnFireCallback = Arc::new(move |reminder: Reminder| {
            let store_inner = Arc::clone(&store_for_cb);
            let notifier_inner = notifier_for_cb.clone();
            let emitter_inner = Arc::clone(&emitter_for_cb);
            // 仰賴 tokio task 隔離:spawned task 內 panic 只殺那條 task,不傳染
            // scheduler。詳見模組 doc。
            tokio::spawn(async move {
                // K3:發桌面通知。失敗 log warn,不 throw。
                //
                // **必須走 spawn_blocking**:`notify_rust::Notification::show()` 在 Linux
                // 內部跑 dbus async client → 自己 build 一個 tokio Runtime。如果直接在
                // 既有 tokio worker thread 內呼叫,會 panic
                // 「Cannot start a runtime from within a runtime」。
                // 觀察點:tokio-1.52.2 multi_thread/mod.rs:91。
                let reminder_for_blk = reminder.clone();
                let fire_result = tokio::task::spawn_blocking(move || {
                    notifier_inner.fire(&reminder_for_blk)
                })
                .await;
                match fire_result {
                    // 2026-05-22 debug:即使 Ok user 也回報沒看到通知,加 info log
                    // 證明 fire 路徑跑完 + 通知有 send 進 dbus(否則早就吐 Err)。
                    Ok(Ok(())) => tracing::info!(
                        reminder_id = reminder.id,
                        text = %reminder.text,
                        "notifier.fire returned Ok — notification submitted to dbus",
                    ),
                    Ok(Err(e)) => tracing::warn!(
                        reminder_id = reminder.id,
                        error = %e,
                        "notifier.fire failed (reminder still mark_fired)",
                    ),
                    Err(join_err) => tracing::warn!(
                        reminder_id = reminder.id,
                        error = %join_err,
                        "notifier.fire spawn_blocking join failed",
                    ),
                }

                // 2026-05-22:in-app popup emit。失敗只 warn,不擋 mark_fired。
                if let Err(e) = emitter_inner.emit_reminder_fire(&reminder) {
                    tracing::warn!(
                        reminder_id = reminder.id,
                        error = %e,
                        "emit reminder-fire-show failed (popup will catch up via active_queue query on next mount)",
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
        });

        // 3) Create scheduler + start
        let scheduler = ReminderScheduler::new(Arc::clone(&store), on_fire).await?;
        scheduler.start().await?;
        let scheduler = Arc::new(scheduler);

        // 4) Reload pending — restart 後把 DB 裡未完成的 reminder 重新 schedule。
        //    用 due_at vs now 判斷:過去的 one-shot 立刻觸發(scheduler 內部處理)。
        let pending = store.lock().await.list_pending()?;
        let now = Utc::now();
        let grace_cutoff = now - chrono::Duration::days(7);
        for r in &pending {
            // 2026-05-22:超過 7 天的 overdue one-shot reminder 自動標 dismissed,
            // 不再 fire,避免 user 久未開 app 後被 spam。cron 不適用(週期性永遠不算 overdue)。
            let is_super_overdue = r.cron_expr.is_none() && r.due_at < grace_cutoff;
            if is_super_overdue {
                let store_guard = store.lock().await;
                let when = Utc::now();
                if let Err(e) = store_guard.mark_fired(r.id, when) {
                    tracing::warn!(
                        reminder_id = r.id,
                        error = %e,
                        "failed to auto-mark super-overdue as fired"
                    );
                    continue;
                }
                if let Err(e) = store_guard.mark_dismissed(r.id, when) {
                    tracing::warn!(
                        reminder_id = r.id,
                        error = %e,
                        "failed to auto-mark super-overdue as dismissed"
                    );
                }
                tracing::info!(
                    reminder_id = r.id,
                    text = %r.text,
                    due_at = %r.due_at,
                    "auto-dismissed super-overdue reminder (> 7d) — skipping fire to avoid spam"
                );
                continue;
            }
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
    /// 內部:parse → store.snooze(更新 status + snoozed_until + **due_at**)→
    /// scheduler 重 schedule。K2 scheduler 是用 reminder.due_at 算 one-shot
    /// duration,store.snooze 已把 due_at 一起推到 until,reload-on-startup 也
    /// 不會用到舊 due_at 立刻觸發。
    pub async fn snooze_reminder(
        &self,
        id: i64,
        when_expr: String,
    ) -> Result<(), CommandError> {
        let until = parser::parse(&when_expr)?;

        // Guard:只允許對 Pending / Snoozed 的 reminder snooze。
        // 對 Fired(一次性已響)/ Cancelled(已取消)snooze 沒意義 —
        // K1 store.snooze 不認狀態、會悄悄改回 Snoozed,等於把過期 / 取消的鳥重新喚回,
        // 對 user 是 surprise update。早 fail 一條清楚的 InvalidStatus。
        {
            let store = self.store.lock().await;
            let current = store.get(id)?;
            use crate::schema::ReminderStatus::{Pending, Snoozed};
            if !matches!(current.status, Pending | Snoozed) {
                return Err(CommandError::InvalidStatus {
                    id,
                    current: format!("{:?}", current.status),
                });
            }
        }

        // 先 cancel scheduler 內舊 job(避免兩個 timer 並存)
        self.scheduler.cancel(id).await?;
        // 更新 store(snooze 內部會把 due_at 一起更新成 until)+ 拿回新 row
        let updated = {
            let store = self.store.lock().await;
            store.snooze(id, until)?;
            store.get(id)?
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
        ReminderService::new(&db, fresh_notifier(), Arc::new(NoopEmitter))
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
    async fn snooze_persists_due_at_for_reload() {
        // K5 fix:snooze 後 drop service、重開,reload 出來的 reminder 必須有「新」
        // due_at(=snooze until),不是原本 30 分鐘後那個。若 schema.snooze 沒同更
        // due_at,reload-on-startup 的重排會用過去的 due_at,reminder 立刻觸發 —
        // 等於 snooze 沒效果。
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("reminders.db");

        let (r_id, snooze_until) = {
            let svc = ReminderService::new(&db, fresh_notifier(), Arc::new(NoopEmitter)).await.unwrap();
            let r = svc
                .remind_me("snz-reload".into(), "30 minutes".into())
                .await
                .unwrap();
            // snooze 到 2 小時後
            svc.snooze_reminder(r.id, "2 hours".into()).await.unwrap();
            // 抓出來確認 store 端 due_at 已被推到 ~2h 後
            let after = svc.store.lock().await.get(r.id).unwrap();
            assert_eq!(after.status, ReminderStatus::Snoozed);
            // due_at 跟 snoozed_until 應該一致
            assert_eq!(after.due_at, after.snoozed_until.unwrap());
            (r.id, after.due_at)
        };

        // drop svc → 重開
        let svc2 = ReminderService::new(&db, fresh_notifier(), Arc::new(NoopEmitter)).await.unwrap();
        let pending = svc2.list_reminders().await.unwrap();
        let reloaded = pending.iter().find(|x| x.id == r_id).expect("reloaded");
        // reload 後 due_at 仍是 snooze until(~2h 後),不是原本 30min 後
        let drift = (reloaded.due_at - snooze_until).num_seconds().abs();
        assert!(drift <= 1, "reloaded due_at drift={drift}s");
        // 跟 now 比應該 ≈ 2h 後(1.9h ~ 2.1h)
        let from_now = (reloaded.due_at - Utc::now()).num_seconds();
        assert!(
            (6840..=7560).contains(&from_now),
            "reloaded due_at should be ~7200s in future, got {from_now}s",
        );
    }

    #[tokio::test]
    async fn service_loads_pending_on_startup() {
        // 第一次:建 reminder + close service(只是 drop)
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("reminders.db");
        let r_id = {
            let svc = ReminderService::new(&db, fresh_notifier(), Arc::new(NoopEmitter)).await.unwrap();
            let r = svc
                .remind_me("persistent".into(), "1 hour".into())
                .await
                .unwrap();
            r.id
        };
        // 第二次:重新開 service,pending 應該被 reload
        let svc2 = ReminderService::new(&db, fresh_notifier(), Arc::new(NoopEmitter)).await.unwrap();
        let pending = svc2.list_reminders().await.unwrap();
        assert_eq!(pending.len(), 1, "expected 1 reloaded pending, got {}", pending.len());
        assert_eq!(pending[0].id, r_id);
        assert_eq!(pending[0].text, "persistent");
    }

    #[tokio::test]
    async fn snooze_rejects_fired_reminder() {
        // 一條已 Fired 的 reminder 不該被 snooze 回 Snoozed 狀態 —
        // K5 snooze guard。在 store 端直接 mark_fired,然後試 snooze,期望 InvalidStatus。
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        let r = svc
            .remind_me("done".into(), "1 hour".into())
            .await
            .unwrap();
        // 直接 mark_fired(bypass scheduler — 我們只測 commands.rs guard)
        svc.store.lock().await.mark_fired(r.id, Utc::now()).unwrap();

        let err = svc
            .snooze_reminder(r.id, "30 minutes".into())
            .await
            .expect_err("snooze 已 fired reminder 必須回 InvalidStatus");
        match err {
            CommandError::InvalidStatus { id, current } => {
                assert_eq!(id, r.id);
                assert_eq!(current, "Fired");
            }
            other => panic!("expected InvalidStatus, got {other:?}"),
        }
        // 確認 store 端 status 沒被悄悄改回 Snoozed
        let after = svc.store.lock().await.get(r.id).unwrap();
        assert_eq!(after.status, ReminderStatus::Fired);
    }

    #[tokio::test]
    async fn snooze_rejects_cancelled_reminder() {
        // 同上,Cancelled 的 reminder 也擋。
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        let r = svc
            .remind_me("nope".into(), "1 hour".into())
            .await
            .unwrap();
        svc.cancel_reminder(r.id).await.unwrap();

        let err = svc
            .snooze_reminder(r.id, "30 minutes".into())
            .await
            .expect_err("snooze cancelled reminder 必須回 InvalidStatus");
        match err {
            CommandError::InvalidStatus { id, current } => {
                assert_eq!(id, r.id);
                assert_eq!(current, "Cancelled");
            }
            other => panic!("expected InvalidStatus, got {other:?}"),
        }
        let after = svc.store.lock().await.get(r.id).unwrap();
        assert_eq!(after.status, ReminderStatus::Cancelled);
    }

    #[tokio::test]
    async fn startup_auto_dismisses_super_overdue_reminders() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let db = dir.path().join("r.db");

        // 先用一個 service 寫一筆 8 天前 overdue + status=Pending
        {
            let store = ReminderStore::open(&db).expect("open");
            let past = Utc::now() - chrono::Duration::days(8);
            store.create("stale".to_string(), past, None).expect("create");
            // 不 mark_fired,留 Pending,模擬 user 一週多沒開 app
        }

        // 開 service → reload-pending 應該把 8 天前的標 dismissed_at + Fired
        let _svc = ReminderService::new(&db, Notifier::new("Mori-Test"), Arc::new(NoopEmitter))
            .await
            .expect("service");
        // 留一點時間給 1ms 觸發 + spawn task 處理 mark_fired 完成
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        let store = ReminderStore::open(&db).expect("reopen");
        let queue = store
            .list_active_popup_queue(Utc::now())
            .expect("list active");
        assert!(queue.is_empty(), "super-overdue reminder should be auto-dismissed, not on popup queue");

        // 而 status 應該是 fired(走完正常 fire path),dismissed_at 也填了
        let conn = rusqlite::Connection::open(&db).unwrap();
        let mut stmt = conn
            .prepare("SELECT status, dismissed_at FROM reminders WHERE text = 'stale'")
            .unwrap();
        let (status, dismissed_at): (String, Option<String>) = stmt
            .query_row([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(status, "fired", "super-overdue should be marked Fired");
        assert!(dismissed_at.is_some(), "super-overdue should be marked dismissed");
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

    #[tokio::test]
    async fn on_fire_calls_emitter_emit_reminder_fire() {
        use std::sync::Mutex as StdMutex;

        // 收集 emit call 的 mock emitter
        #[derive(Default)]
        struct CapturingEmitter {
            calls: StdMutex<Vec<i64>>,
        }
        impl EventEmitter for CapturingEmitter {
            fn emit_reminder_fire(&self, r: &Reminder) -> Result<(), String> {
                self.calls.lock().unwrap().push(r.id);
                Ok(())
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("r.db");
        let emitter = Arc::new(CapturingEmitter::default());

        let svc = ReminderService::new(
            &db,
            Notifier::new("Mori-Test"),
            emitter.clone() as Arc<dyn EventEmitter>,
        )
        .await
        .expect("svc");

        // 排一個 100ms 後 fire 的 reminder
        let when = Utc::now() + chrono::Duration::milliseconds(100);
        let r = svc.store.lock().await.create("emit-probe".to_string(), when, None).unwrap();
        svc.scheduler.schedule(&r).await.unwrap();

        // 等 fire + spawn task 完成
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;

        let calls = emitter.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "emit_reminder_fire should be called once");
        assert_eq!(calls[0], r.id);
    }
}
