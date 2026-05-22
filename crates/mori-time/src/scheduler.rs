//! K2 scheduler — tokio-cron-scheduler 整合
//!
//! [`ReminderScheduler`] 是一個薄包裝,把 [`crate::schema::ReminderStore`] 裡的
//! reminder 排程進 [`tokio_cron_scheduler::JobScheduler`]。觸發時呼叫 `on_fire`
//! callback,callback 內部由上層(K5)決定要做什麼(寫桌面通知 / mark_fired / re-load …)。
//!
//! 兩種 reminder:
//! - **一次性**(`cron_expr` 是 `None`):用 `Job::new_one_shot(due_at - now, …)`。
//!   過去時間視作「立刻」(1ms duration)。
//! - **週期性**(`cron_expr` 是 `Some(expr)`):用 `Job::new_cron_job(expr, …)`,
//!   `expr` 是 6-field cron(含秒)e.g. `0 0 8 * * *` = 每天早上 8:00。
//!
//! 取消:scheduler 內部用 uuid 認 job,我們維護一個 `reminder_id -> Uuid` 的對照表。
//!
//! ⚠️ tokio-cron-scheduler 的 one-shot 是每 500ms 檢查一次,所以最小觸發精度約 500ms。
//! 測試裡需要預留一點 margin。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::schema::{Reminder, ReminderStore};

/// scheduler 層錯誤類型。
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("scheduler init failed: {0}")]
    Init(String),
    #[error("schedule failed: {0}")]
    Schedule(String),
    #[error("store error: {0}")]
    Store(#[from] crate::schema::ReminderError),
}

/// 觸發 reminder 時呼叫的 callback。內部由 K5 wire 進 K3 notifier + store.mark_fired()。
pub type OnFireCallback = Arc<dyn Fn(Reminder) + Send + Sync>;

/// reminder 排程器 — 持有 tokio-cron-scheduler 的 `JobScheduler`,
/// 並維護 `reminder_id -> job_uuid` 對照表以便取消。
pub struct ReminderScheduler {
    scheduler: JobScheduler,
    #[allow(dead_code)] // K5 reload-from-store 時會用到
    store: Arc<Mutex<ReminderStore>>,
    on_fire: OnFireCallback,
    /// 對照 reminder.id -> 已加入 scheduler 的 Job(本身有 `.guid()`),給 `cancel()` 用。
    /// 存 Job 而不是 Uuid 是為了避免直接 import `uuid` crate(K2 不引新 deps;
    /// uuid 是 tokio-cron-scheduler 的 transitive dep,Rust 不允許這樣直接用)。
    jobs: Arc<Mutex<HashMap<i64, Job>>>,
}

impl ReminderScheduler {
    /// 建立新 scheduler(不會自動 start,要呼叫 [`start`])。
    pub async fn new(
        store: Arc<Mutex<ReminderStore>>,
        on_fire: OnFireCallback,
    ) -> Result<Self, SchedulerError> {
        let scheduler = JobScheduler::new()
            .await
            .map_err(|e| SchedulerError::Init(e.to_string()))?;
        Ok(Self {
            scheduler,
            store,
            on_fire,
            jobs: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// 啟動背景 scheduler tick。可重複呼叫(內部 idempotent)。
    pub async fn start(&self) -> Result<(), SchedulerError> {
        self.scheduler
            .start()
            .await
            .map_err(|e| SchedulerError::Schedule(e.to_string()))?;
        Ok(())
    }

    /// 把一個 reminder 排程進 scheduler。
    ///
    /// - `reminder.cron_expr` 是 `Some(expr)` → cron job(週期性)
    /// - 否則 → one-shot,以 `reminder.due_at - now()` 為 duration;
    ///   過去時間視作立刻觸發(1ms duration)。
    ///
    /// 同一個 reminder id 重複 schedule 會先 cancel 舊的、再排新的。
    pub async fn schedule(&self, reminder: &Reminder) -> Result<(), SchedulerError> {
        // 同 id 已排程過 → 先 cancel(idempotent re-schedule,K5 reload 後會用到)
        if self.jobs.lock().await.contains_key(&reminder.id) {
            self.cancel(reminder.id).await?;
        }

        let on_fire = Arc::clone(&self.on_fire);
        let reminder_for_cb = reminder.clone();

        let job = if let Some(expr) = reminder.cron_expr.as_deref() {
            Job::new_cron_job(expr, move |_uuid, _l| {
                (on_fire)(reminder_for_cb.clone());
            })
            .map_err(|e| SchedulerError::Schedule(format!("cron expr '{expr}': {e}")))?
        } else {
            let now = Utc::now();
            let delta = (reminder.due_at - now).num_milliseconds();
            // ≤0 → 立刻觸發(1ms 給 scheduler 一點 buffer)
            let dur = if delta <= 0 {
                Duration::from_millis(1)
            } else {
                Duration::from_millis(delta as u64)
            };
            Job::new_one_shot(dur, move |_uuid, _l| {
                (on_fire)(reminder_for_cb.clone());
            })
            .map_err(|e| SchedulerError::Schedule(format!("one-shot: {e}")))?
        };

        self.scheduler
            .add(job.clone())
            .await
            .map_err(|e| SchedulerError::Schedule(e.to_string()))?;
        self.jobs.lock().await.insert(reminder.id, job);
        Ok(())
    }

    /// 取消已排程的 reminder。對沒排程過 / 已 fire 過的 id 不會錯,直接 no-op。
    pub async fn cancel(&self, reminder_id: i64) -> Result<(), SchedulerError> {
        let job = self.jobs.lock().await.remove(&reminder_id);
        if let Some(job) = job {
            self.scheduler
                .remove(&job.guid())
                .await
                .map_err(|e| SchedulerError::Schedule(e.to_string()))?;
        }
        Ok(())
    }

    /// 關閉 scheduler。tokio-cron-scheduler 的 `shutdown` 需要 `&mut`,
    /// 但 `JobsSchedulerLocked` 內部是 Arc 共享,clone 一份來呼叫即可。
    pub async fn shutdown(&self) -> Result<(), SchedulerError> {
        let mut sched = self.scheduler.clone();
        sched
            .shutdown()
            .await
            .map_err(|e| SchedulerError::Schedule(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration as StdDuration;

    /// 收集 callback 觸發紀錄的 test fixture。
    type FireLog = Arc<StdMutex<Vec<Reminder>>>;

    fn fire_log() -> (FireLog, OnFireCallback) {
        let log: FireLog = Arc::new(StdMutex::new(Vec::new()));
        let log_for_cb = Arc::clone(&log);
        let cb: OnFireCallback = Arc::new(move |r: Reminder| {
            log_for_cb.lock().unwrap().push(r);
        });
        (log, cb)
    }

    async fn fresh_store() -> Arc<Mutex<ReminderStore>> {
        let store = ReminderStore::open_in_memory().expect("open in-memory store");
        Arc::new(Mutex::new(store))
    }

    #[tokio::test]
    async fn scheduler_starts_and_shuts_down_cleanly() {
        let store = fresh_store().await;
        let (_log, cb) = fire_log();
        let sched = ReminderScheduler::new(store, cb)
            .await
            .expect("new scheduler");
        sched.start().await.expect("start ok");
        sched.shutdown().await.expect("shutdown ok");
    }

    #[tokio::test]
    async fn schedule_one_shot_fires_after_due() {
        let store = fresh_store().await;
        let (log, cb) = fire_log();
        let sched = ReminderScheduler::new(Arc::clone(&store), cb)
            .await
            .expect("new scheduler");
        sched.start().await.expect("start ok");

        // due in 100ms;tokio-cron-scheduler one-shot tick = 500ms,
        // 所以實際觸發 ~500-1000ms 後,等 2s 留 margin。
        let due = Utc::now() + ChronoDuration::milliseconds(100);
        let r = store
            .lock()
            .await
            .create("ping".into(), due, None)
            .expect("create reminder");
        sched.schedule(&r).await.expect("schedule ok");

        tokio::time::sleep(StdDuration::from_millis(2000)).await;

        let fires = log.lock().unwrap();
        assert_eq!(fires.len(), 1, "expected exactly 1 fire, got {}", fires.len());
        assert_eq!(fires[0].id, r.id);
        assert_eq!(fires[0].text, "ping");

        drop(fires);
        sched.shutdown().await.expect("shutdown ok");
    }

    #[tokio::test]
    async fn schedule_cron_fires_repeatedly() {
        let store = fresh_store().await;
        let (log, cb) = fire_log();
        let sched = ReminderScheduler::new(Arc::clone(&store), cb)
            .await
            .expect("new scheduler");
        sched.start().await.expect("start ok");

        // 每秒一次(6-field cron with seconds)
        let due = Utc::now() + ChronoDuration::seconds(1);
        let r = store
            .lock()
            .await
            .create(
                "tick".into(),
                due,
                Some("*/1 * * * * *".to_string()),
            )
            .expect("create cron reminder");
        sched.schedule(&r).await.expect("schedule cron ok");

        // 等 5 秒,該觸發 ≥ 2 次。
        // 之前是 3.2s 期望 ≥2 次 — tokio-cron-scheduler 的 worker tick 是 500ms,
        // 第一次 fire 可能要等 0.5-1.5s 才落到 worker,慢 CI(尤其 cold cargo run)
        // 第二次 fire 卡到 3s 邊界很常見。延到 5s 留充裕 margin,assert 邏輯不變。
        tokio::time::sleep(StdDuration::from_secs(5)).await;

        let n_fires = log.lock().unwrap().len();
        assert!(
            n_fires >= 2,
            "expected ≥2 cron fires in 5s, got {n_fires}"
        );

        sched.shutdown().await.expect("shutdown ok");
    }

    #[tokio::test]
    async fn cancel_prevents_fire() {
        let store = fresh_store().await;
        let (log, cb) = fire_log();
        let sched = ReminderScheduler::new(Arc::clone(&store), cb)
            .await
            .expect("new scheduler");
        sched.start().await.expect("start ok");

        // due in 1s
        let due = Utc::now() + ChronoDuration::seconds(1);
        let r = store
            .lock()
            .await
            .create("ghost".into(), due, None)
            .expect("create reminder");
        sched.schedule(&r).await.expect("schedule ok");

        // 立刻 cancel
        sched.cancel(r.id).await.expect("cancel ok");

        // 等到 due 過去 + scheduler tick margin
        tokio::time::sleep(StdDuration::from_millis(2000)).await;

        let n_fires = log.lock().unwrap().len();
        assert_eq!(n_fires, 0, "expected 0 fires after cancel, got {n_fires}");

        // cancel 不存在的 id 應該 no-op,不錯
        sched.cancel(99999).await.expect("cancel unknown ok");

        sched.shutdown().await.expect("shutdown ok");
    }

    #[tokio::test]
    async fn schedule_past_reminder_fires_immediately() {
        let store = fresh_store().await;
        let (log, cb) = fire_log();
        let sched = ReminderScheduler::new(Arc::clone(&store), cb)
            .await
            .expect("new scheduler");
        sched.start().await.expect("start ok");

        // due 10 秒前(已經過去)
        let due = Utc::now() - ChronoDuration::seconds(10);
        let r = store
            .lock()
            .await
            .create("late".into(), due, None)
            .expect("create reminder");
        sched.schedule(&r).await.expect("schedule ok");

        // 過去 reminder 視作立刻觸發,但 one-shot 仍受 500ms tick 影響,等 1.5s
        tokio::time::sleep(StdDuration::from_millis(1500)).await;

        let n_fires = log.lock().unwrap().len();
        assert_eq!(n_fires, 1, "expected 1 fire for past-due reminder, got {n_fires}");

        sched.shutdown().await.expect("shutdown ok");
    }
}
