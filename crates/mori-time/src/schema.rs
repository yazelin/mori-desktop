//! K1 — Reminder SQLite schema + CRUD
//!
//! [`ReminderStore`] 封裝一個 SQLite connection,提供:
//! - `migrate()` — 建表(idempotent)
//! - `create()` — 新增 reminder(回傳含 id 的 [`Reminder`])
//! - `list_pending()` / `list_all()` / `get()` — 讀
//! - `mark_fired()` / `snooze()` / `cancel()` — 狀態變更
//!
//! Note: K1 暫不負責背景排程,K2 會 wrap 本 store 在 `tokio-cron-scheduler` 內。

use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

/// 單筆 reminder。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reminder {
    pub id: i64,
    /// 要提醒的內容(短文字,可包含 emoji)。
    pub text: String,
    /// 下次觸發時間(UTC)。一次性 reminder fire 後就 stop;repeating 由 K2 scheduler 算下一次。
    pub due_at: DateTime<Utc>,
    /// `None` = 一次性;`Some(expr)` = 重複(K2 用 tokio-cron-scheduler 解析)。
    pub cron_expr: Option<String>,
    pub created_at: DateTime<Utc>,
    /// 上次實際觸發時間;`None` = 還沒響過。
    pub fired_at: Option<DateTime<Utc>>,
    /// 用戶明確關掉 popup 的時間;`None` = 還沒 dismiss 過。
    /// 區分「fired 但 user 沒看」vs「user 已關掉」。
    pub dismissed_at: Option<DateTime<Utc>>,
    /// 暫緩到何時(snooze 後 status = Snoozed,到時間才繼續排程)。
    pub snoozed_until: Option<DateTime<Utc>>,
    pub status: ReminderStatus,
}

/// Reminder 生命週期狀態。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReminderStatus {
    /// 排程中,等待 due_at。
    Pending,
    /// 已觸發(一次性 reminder 觸發後進此狀態)。
    Fired,
    /// 用戶 snooze 後暫緩。
    Snoozed,
    /// 用戶取消。
    Cancelled,
}

impl ReminderStatus {
    /// 序列化成資料庫存的小寫字串。
    fn as_db_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Fired => "fired",
            Self::Snoozed => "snoozed",
            Self::Cancelled => "cancelled",
        }
    }

    fn from_db_str(s: &str) -> Result<Self, ReminderError> {
        match s {
            "pending" => Ok(Self::Pending),
            "fired" => Ok(Self::Fired),
            "snoozed" => Ok(Self::Snoozed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(ReminderError::BadStatus(other.to_string())),
        }
    }
}

/// store 錯誤類型。
#[derive(Debug, thiserror::Error)]
pub enum ReminderError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("not found: id={0}")]
    NotFound(i64),
    #[error("unknown reminder status string: {0}")]
    BadStatus(String),
}

/// 包裝一個 SQLite connection,提供 reminder CRUD。
///
/// K2 scheduler 會持有 `Arc<Mutex<ReminderStore>>` 或自己拿 connection — 留給 K2 決定。
pub struct ReminderStore {
    conn: Connection,
}

impl ReminderStore {
    /// 開啟(或建立)指定路徑的 SQLite db。會 auto-run `migrate()`。
    pub fn open(db_path: &Path) -> Result<Self, ReminderError> {
        let conn = Connection::open(db_path)?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// 開一個記憶體 db(tests 用)。自動 migrate。
    pub fn open_in_memory() -> Result<Self, ReminderError> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// 建表。Idempotent — 重複跑不會錯。
    pub fn migrate(&self) -> Result<(), ReminderError> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS reminders (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                text            TEXT NOT NULL,
                due_at          TEXT NOT NULL,
                cron_expr       TEXT,
                created_at      TEXT NOT NULL,
                fired_at        TEXT,
                snoozed_until   TEXT,
                status          TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_reminders_status ON reminders(status);
            CREATE INDEX IF NOT EXISTS idx_reminders_due_at ON reminders(due_at);
            "#,
        )?;

        // 2026-05-22: dismissed_at 區分「fired 但 user 沒看」vs「user 已關掉」
        // SQLite 沒 ADD COLUMN IF NOT EXISTS,用 table_info pragma 檢查
        let has_dismissed_at: bool = self
            .conn
            .prepare("SELECT 1 FROM pragma_table_info('reminders') WHERE name = 'dismissed_at'")
            .and_then(|mut s| s.query_row([], |_| Ok(true)))
            .unwrap_or(false);
        if !has_dismissed_at {
            self.conn.execute(
                "ALTER TABLE reminders ADD COLUMN dismissed_at TEXT",
                [],
            )?;
        }

        Ok(())
    }

    /// 新增一個 pending reminder。`created_at` 自動填 now。
    pub fn create(
        &self,
        text: String,
        due_at: DateTime<Utc>,
        cron: Option<String>,
    ) -> Result<Reminder, ReminderError> {
        let now = Utc::now();
        let status = ReminderStatus::Pending;
        self.conn.execute(
            r#"INSERT INTO reminders (text, due_at, cron_expr, created_at, fired_at, snoozed_until, status)
               VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5)"#,
            params![
                text,
                due_at.to_rfc3339(),
                cron,
                now.to_rfc3339(),
                status.as_db_str(),
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        Ok(Reminder {
            id,
            text,
            due_at,
            cron_expr: cron,
            created_at: now,
            fired_at: None,
            dismissed_at: None,
            snoozed_until: None,
            status,
        })
    }

    /// 列出仍需排程的 reminder(status = Pending 或 Snoozed)。
    pub fn list_pending(&self) -> Result<Vec<Reminder>, ReminderError> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, text, due_at, cron_expr, created_at, fired_at, dismissed_at, snoozed_until, status
               FROM reminders
               WHERE status IN ('pending', 'snoozed')
               ORDER BY due_at ASC"#,
        )?;
        let rows = stmt.query_map([], row_to_reminder)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ReminderError::from)
            .and_then(|v| v.into_iter().collect::<Result<Vec<_>, _>>())
    }

    /// 列出所有 reminder(含 fired / cancelled),由舊到新。
    pub fn list_all(&self) -> Result<Vec<Reminder>, ReminderError> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, text, due_at, cron_expr, created_at, fired_at, dismissed_at, snoozed_until, status
               FROM reminders
               ORDER BY id ASC"#,
        )?;
        let rows = stmt.query_map([], row_to_reminder)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ReminderError::from)
            .and_then(|v| v.into_iter().collect::<Result<Vec<_>, _>>())
    }

    /// 拿單筆。找不到回 `NotFound(id)`。
    pub fn get(&self, id: i64) -> Result<Reminder, ReminderError> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, text, due_at, cron_expr, created_at, fired_at, dismissed_at, snoozed_until, status
               FROM reminders
               WHERE id = ?1"#,
        )?;
        let row = stmt
            .query_row(params![id], row_to_reminder)
            .optional()?;
        match row {
            Some(r) => r,
            None => Err(ReminderError::NotFound(id)),
        }
    }

    /// 標記 fired:寫入 `fired_at` 並把 status 設為 Fired。
    pub fn mark_fired(&self, id: i64, fired_at: DateTime<Utc>) -> Result<(), ReminderError> {
        let n = self.conn.execute(
            r#"UPDATE reminders
               SET fired_at = ?1, status = ?2
               WHERE id = ?3"#,
            params![fired_at.to_rfc3339(), ReminderStatus::Fired.as_db_str(), id],
        )?;
        if n == 0 {
            return Err(ReminderError::NotFound(id));
        }
        Ok(())
    }

    /// snooze 到指定時間。status -> Snoozed。
    ///
    /// **同時更新 `due_at = until`** — 這條 invariant 給 reload-on-startup 用:
    /// `ReminderService::new()` 會 `list_pending()` 後重新 schedule,scheduler 是用
    /// `due_at` 算 one-shot 觸發時間,不是 `snoozed_until`。若只更 `snoozed_until`,
    /// drop / restart 後重排會用舊 `due_at`(可能已過),導致 reminder 立刻觸發,
    /// 等於 snooze 沒效果。
    pub fn snooze(&self, id: i64, until: DateTime<Utc>) -> Result<(), ReminderError> {
        let n = self.conn.execute(
            r#"UPDATE reminders
               SET snoozed_until = ?1, due_at = ?1, status = ?2
               WHERE id = ?3"#,
            params![until.to_rfc3339(), ReminderStatus::Snoozed.as_db_str(), id],
        )?;
        if n == 0 {
            return Err(ReminderError::NotFound(id));
        }
        Ok(())
    }

    /// 取消 reminder。status -> Cancelled。
    pub fn cancel(&self, id: i64) -> Result<(), ReminderError> {
        let n = self.conn.execute(
            r#"UPDATE reminders
               SET status = ?1
               WHERE id = ?2"#,
            params![ReminderStatus::Cancelled.as_db_str(), id],
        )?;
        if n == 0 {
            return Err(ReminderError::NotFound(id));
        }
        Ok(())
    }

    /// 標 reminder 為 user 已 dismiss(從 popup 點 [關閉] 後寫入)。
    /// 寫入時間,reminder.status 不動(`Fired` 不變),只是 popup 不再列出。
    pub fn mark_dismissed(&self, id: i64, at: DateTime<Utc>) -> Result<(), ReminderError> {
        let affected = self.conn.execute(
            "UPDATE reminders SET dismissed_at = ?1 WHERE id = ?2",
            params![at.to_rfc3339(), id],
        )?;
        if affected == 0 {
            return Err(ReminderError::NotFound(id));
        }
        Ok(())
    }

    /// 給 in-app popup 用的 active queue:
    /// - status = Fired
    /// - dismissed_at IS NULL
    /// - fired_at > now - 7 days(grace,超過自動忽略,不再 popup)
    /// 排序 fired_at DESC(最新先)。
    pub fn list_active_popup_queue(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<Reminder>, ReminderError> {
        let grace_cutoff = now - chrono::Duration::days(7);
        let mut stmt = self.conn.prepare(
            "SELECT id, text, due_at, cron_expr, created_at, fired_at, dismissed_at, snoozed_until, status
             FROM reminders
             WHERE status = 'fired'
               AND dismissed_at IS NULL
               AND fired_at > ?1
             ORDER BY fired_at DESC",
        )?;
        let rows = stmt.query_map(params![grace_cutoff.to_rfc3339()], row_to_reminder)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ReminderError::from)
            .and_then(|v| v.into_iter().collect::<Result<Vec<_>, _>>())
    }
}

/// 把 sqlite row 拆成 `Reminder`。回傳 nested Result 因為 status 解析可能失敗。
///
/// 欄位順序(對應 SELECT 的 column 位置):
/// 0=id, 1=text, 2=due_at, 3=cron_expr, 4=created_at,
/// 5=fired_at, 6=dismissed_at, 7=snoozed_until, 8=status
fn row_to_reminder(row: &Row<'_>) -> rusqlite::Result<Result<Reminder, ReminderError>> {
    let id: i64 = row.get(0)?;
    let text: String = row.get(1)?;
    let due_at: String = row.get(2)?;
    let cron_expr: Option<String> = row.get(3)?;
    let created_at: String = row.get(4)?;
    let fired_at: Option<String> = row.get(5)?;
    let dismissed_at: Option<String> = row.get(6)?;
    let snoozed_until: Option<String> = row.get(7)?;
    let status: String = row.get(8)?;

    Ok((|| -> Result<Reminder, ReminderError> {
        Ok(Reminder {
            id,
            text,
            due_at: parse_rfc3339(&due_at)?,
            cron_expr,
            created_at: parse_rfc3339(&created_at)?,
            fired_at: fired_at.as_deref().map(parse_rfc3339).transpose()?,
            dismissed_at: dismissed_at.as_deref().map(parse_rfc3339).transpose()?,
            snoozed_until: snoozed_until.as_deref().map(parse_rfc3339).transpose()?,
            status: ReminderStatus::from_db_str(&status)?,
        })
    })())
}

fn parse_rfc3339(s: &str) -> Result<DateTime<Utc>, ReminderError> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| {
            // 包成 BadStatus 沿用 — 比較簡單。實際上應該分一個 BadTimestamp,但 K1 暫不需要。
            ReminderError::BadStatus(format!("bad rfc3339 timestamp '{s}': {e}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn store() -> ReminderStore {
        ReminderStore::open_in_memory().expect("open in-memory store")
    }

    #[test]
    fn migrate_creates_table_idempotent() {
        let s = store();
        // 第二次跑不應該錯
        s.migrate().expect("second migrate ok");
        s.migrate().expect("third migrate ok");
        // sanity: list_all 不會炸
        assert_eq!(s.list_all().unwrap().len(), 0);
    }

    #[test]
    fn create_returns_reminder_with_id() {
        let s = store();
        let due = Utc::now() + Duration::minutes(10);
        let r = s
            .create("喝水".to_string(), due, None)
            .expect("create ok");
        assert!(r.id > 0);
        assert_eq!(r.text, "喝水");
        assert_eq!(r.status, ReminderStatus::Pending);
        assert!(r.cron_expr.is_none());

        // 再 create 一筆 id 應該遞增
        let r2 = s
            .create("散步".to_string(), due + Duration::minutes(5), None)
            .expect("create2 ok");
        assert!(r2.id > r.id);
    }

    #[test]
    fn list_pending_filters_by_status() {
        let s = store();
        let due = Utc::now() + Duration::minutes(10);
        let r1 = s.create("a".into(), due, None).unwrap();
        let r2 = s.create("b".into(), due, None).unwrap();
        let r3 = s.create("c".into(), due, None).unwrap();

        // 把 r2 標 fired, r3 cancel,r1 留 pending
        s.mark_fired(r2.id, Utc::now()).unwrap();
        s.cancel(r3.id).unwrap();

        let pending = s.list_pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, r1.id);

        // list_all 應該全部三筆
        assert_eq!(s.list_all().unwrap().len(), 3);
    }

    #[test]
    fn list_pending_includes_snoozed() {
        let s = store();
        let due = Utc::now() + Duration::minutes(10);
        let r = s.create("snoozey".into(), due, None).unwrap();
        s.snooze(r.id, Utc::now() + Duration::minutes(30)).unwrap();
        let pending = s.list_pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].status, ReminderStatus::Snoozed);
    }

    #[test]
    fn get_returns_error_for_unknown_id() {
        let s = store();
        match s.get(999) {
            Err(ReminderError::NotFound(id)) => assert_eq!(id, 999),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn mark_fired_updates_status_and_timestamp() {
        let s = store();
        let due = Utc::now() + Duration::minutes(10);
        let r = s.create("test".into(), due, None).unwrap();
        assert!(r.fired_at.is_none());

        let fired_at = Utc::now();
        s.mark_fired(r.id, fired_at).unwrap();

        let after = s.get(r.id).unwrap();
        assert_eq!(after.status, ReminderStatus::Fired);
        assert!(after.fired_at.is_some());
        // 容忍 1 秒誤差(rfc3339 round-trip)
        let diff = (after.fired_at.unwrap() - fired_at).num_seconds().abs();
        assert!(diff <= 1, "fired_at round-trip drift {diff}s");

        // 不存在 id 應該回 NotFound
        match s.mark_fired(999, Utc::now()) {
            Err(ReminderError::NotFound(_)) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn snooze_updates_until_and_status() {
        let s = store();
        let due = Utc::now() + Duration::minutes(10);
        let r = s.create("snz".into(), due, None).unwrap();
        let until = Utc::now() + Duration::hours(1);
        s.snooze(r.id, until).unwrap();

        let after = s.get(r.id).unwrap();
        assert_eq!(after.status, ReminderStatus::Snoozed);
        assert!(after.snoozed_until.is_some());

        match s.snooze(999, until) {
            Err(ReminderError::NotFound(_)) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn snooze_updates_due_at_too() {
        // K5 fix:snooze 必須同更 due_at,reload-on-startup 才不會立刻觸發。
        let s = store();
        // due 在過去 — 模擬「reminder 本來該響了,user snooze」
        let past_due = Utc::now() - Duration::minutes(5);
        let r = s.create("late".into(), past_due, None).unwrap();

        let until = Utc::now() + Duration::hours(2);
        s.snooze(r.id, until).unwrap();

        let after = s.get(r.id).unwrap();
        assert_eq!(after.status, ReminderStatus::Snoozed);
        assert!(after.snoozed_until.is_some());
        // 關鍵 invariant:due_at 也被推到 until,不是停在 past_due
        let drift = (after.due_at - until).num_seconds().abs();
        assert!(drift <= 1, "due_at should track snoozed_until, drift={drift}s");

        // list_pending 拿出來的也得是新 due_at(防 SELECT 漏 column 等 dumb bug)
        let pending = s.list_pending().unwrap();
        let r2 = pending.iter().find(|x| x.id == r.id).expect("in pending");
        let drift2 = (r2.due_at - until).num_seconds().abs();
        assert!(drift2 <= 1, "list_pending due_at drift={drift2}s");
    }

    #[test]
    fn cancel_updates_status() {
        let s = store();
        let due = Utc::now() + Duration::minutes(10);
        let r = s.create("byebye".into(), due, None).unwrap();
        s.cancel(r.id).unwrap();

        let after = s.get(r.id).unwrap();
        assert_eq!(after.status, ReminderStatus::Cancelled);

        match s.cancel(999) {
            Err(ReminderError::NotFound(_)) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn create_with_cron_expr_persists() {
        let s = store();
        let due = Utc::now() + Duration::minutes(10);
        let r = s
            .create(
                "每天早上提醒喝水".into(),
                due,
                Some("0 0 8 * * *".to_string()),
            )
            .unwrap();
        let fetched = s.get(r.id).unwrap();
        assert_eq!(fetched.cron_expr.as_deref(), Some("0 0 8 * * *"));
    }

    #[test]
    fn migration_adds_dismissed_at_idempotent() {
        let store = ReminderStore::open_in_memory().expect("open");
        // 二次 migrate 不爆
        store.migrate().expect("migrate 2nd time");
        // 寫進去之後讀得到 None
        let r = store
            .create("test".to_string(), Utc::now(), None)
            .expect("create");
        assert!(r.dismissed_at.is_none(), "new reminder dismissed_at should be None");
    }

    #[test]
    fn mark_dismissed_sets_timestamp() {
        let store = ReminderStore::open_in_memory().expect("open");
        let r = store
            .create("hello".to_string(), Utc::now(), None)
            .expect("create");
        store.mark_fired(r.id, Utc::now()).expect("mark fired");
        let when = Utc::now();
        store.mark_dismissed(r.id, when).expect("mark dismissed");
        let got = store.get(r.id).expect("get");
        assert!(got.dismissed_at.is_some(), "dismissed_at should be Some after mark_dismissed");
    }

    #[test]
    fn list_active_popup_queue_filters_dismissed_and_super_overdue() {
        let store = ReminderStore::open_in_memory().expect("open");
        let now = Utc::now();
        let week_plus_one = now - chrono::Duration::days(8);
        let recent = now - chrono::Duration::minutes(5);

        // 一筆 7 天前 fired 沒 dismiss → 不算 active(超過 grace)
        let stale = store.create("stale".to_string(), week_plus_one, None).unwrap();
        store.mark_fired(stale.id, week_plus_one).unwrap();
        // 一筆 5 分鐘前 fired 沒 dismiss → active
        let active = store.create("active".to_string(), recent, None).unwrap();
        store.mark_fired(active.id, recent).unwrap();
        // 一筆 fired + dismissed → 不算 active
        let done = store.create("done".to_string(), recent, None).unwrap();
        store.mark_fired(done.id, recent).unwrap();
        store.mark_dismissed(done.id, now).unwrap();

        let queue = store.list_active_popup_queue(now).expect("list");
        let ids: Vec<i64> = queue.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![active.id], "only the recent unsmissed reminder should appear");
    }
}
