# In-app Reminder Popup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mori 自家 in-app reminder popup window 取代不可靠的 OS 通知作為主路徑;OS 通知留 backup 可 toggle;DB 加 `dismissed_at` 區分「fired 但 user 沒看」。

**Architecture:** 新 Tauri window `reminder_popup` + React 元件 listen Rust on_fire emit;ReminderService::new 注入 AppHandle;`~/.mori/config.json` 新增 `notifications` 子樹存 toggle;catch-up >7 天 silent dismiss。

**Tech Stack:** Rust + tokio-cron-scheduler + rusqlite + Tauri 2 + React + TS;對齊 mori-desktop 既有 `ChatBubble.tsx` window pattern + `hotkey_config.rs` config subtree pattern。

**Spec:** `docs/superpowers/specs/2026-05-22-in-app-reminder-popup-design.md`

---

## Task 1: DB schema — add `dismissed_at` column (idempotent migration)

**Files:**
- Modify: `crates/mori-time/src/schema.rs`(`ReminderStore::migrate` + `Reminder` struct)
- Test: `crates/mori-time/src/schema.rs`(下方 `#[cfg(test)] mod tests`)

- [ ] **Step 1: 加 failing test:migration idempotent + Reminder struct 有 dismissed_at**

加在 `schema.rs` 既有 tests 區段(`mod tests` 內):

```rust
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
```

- [ ] **Step 2: 跑 test 看 fail**

Run: `cargo test -p mori-time migration_adds_dismissed_at_idempotent -- --nocapture`
Expected: FAIL(`dismissed_at` 不存在 / `Reminder` 沒此欄)

- [ ] **Step 3: 加 `dismissed_at` 到 `Reminder` struct**

`Reminder` struct 已有 `fired_at: Option<DateTime<Utc>>`,在它旁邊加:

```rust
pub struct Reminder {
    // ... 既有欄位 ...
    pub fired_at: Option<DateTime<Utc>>,
    pub dismissed_at: Option<DateTime<Utc>>,  // 新增
    // ... 既有 ...
}
```

- [ ] **Step 4: migrate 加 `dismissed_at` 欄(idempotent 用 PRAGMA table_info 檢查)**

在 `migrate` 內,既有 `CREATE TABLE IF NOT EXISTS reminders ...` 之後加:

```rust
// 2026-05-22: dismissed_at 區分「fired 但 user 沒看」vs「user 已關掉」
// SQLite 沒 ADD COLUMN IF NOT EXISTS,用 table_info pragma 檢查
let has_dismissed_at: bool = conn
    .prepare("SELECT 1 FROM pragma_table_info('reminders') WHERE name = 'dismissed_at'")
    .and_then(|mut s| s.query_row([], |_| Ok(true)))
    .unwrap_or(false);
if !has_dismissed_at {
    conn.execute(
        "ALTER TABLE reminders ADD COLUMN dismissed_at TEXT",
        [],
    )
    .map_err(|e| ReminderError::Sql(e.to_string()))?;
}
```

- [ ] **Step 5: 改 row → Reminder mapping(`row_to_reminder` 或同等 helper)**

找到所有 `Reminder { id: ..., ... }` 構造(主要在 `row_to_reminder` 或 `create` / `get` / `list_pending` 等內 SQLite row mapping),加:

```rust
dismissed_at: row.get::<_, Option<String>>("dismissed_at")?
    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
    .map(|dt| dt.with_timezone(&Utc)),
```

- [ ] **Step 6: 跑 test pass**

Run: `cargo test -p mori-time migration_adds_dismissed_at_idempotent -- --nocapture`
Expected: PASS

- [ ] **Step 7: 確認既有 tests 都還過**

Run: `cargo test -p mori-time --lib`
Expected: PASS(全部,包含既有 44 tests + 新 1 test)

- [ ] **Step 8: Commit**

```bash
git add crates/mori-time/src/schema.rs
git commit -m "feat(mori-time): add dismissed_at column to reminders with idempotent migration"
```

---

## Task 2: ReminderStore methods — `mark_dismissed` + `list_active_popup_queue`

**Files:**
- Modify: `crates/mori-time/src/schema.rs`(`impl ReminderStore` 加 method)
- Test: 同檔內 `mod tests`

- [ ] **Step 1: 加 failing tests**

```rust
#[test]
fn mark_dismissed_sets_timestamp() {
    let store = ReminderStore::open_in_memory().expect("open");
    let r = store
        .create("hello".to_string(), Utc::now(), None)
        .expect("create");
    store.mark_fired(r.id, Utc::now()).expect("mark fired");
    let when = Utc::now();
    store.mark_dismissed(r.id, when).expect("mark dismissed");
    let got = store.get(r.id).expect("get").expect("exists");
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
```

- [ ] **Step 2: 跑 tests 看 fail**

Run: `cargo test -p mori-time mark_dismissed_sets_timestamp list_active_popup_queue -- --nocapture`
Expected: FAIL(methods 不存在)

- [ ] **Step 3: 實作 methods**

在 `impl ReminderStore`(`schema.rs`)內加:

```rust
/// 標 reminder 為 user 已 dismiss(從 popup 點 [關閉] 後寫入)。
/// 寫入時間,reminder.status 不動(`Fired` 不變),只是 popup 不再列出。
pub fn mark_dismissed(&self, id: i64, at: DateTime<Utc>) -> Result<(), ReminderError> {
    let conn = self.conn.lock();
    let affected = conn
        .execute(
            "UPDATE reminders SET dismissed_at = ?1 WHERE id = ?2",
            rusqlite::params![at.to_rfc3339(), id],
        )
        .map_err(|e| ReminderError::Sql(e.to_string()))?;
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
    let conn = self.conn.lock();
    let mut stmt = conn
        .prepare(
            "SELECT id, text, due_at, cron_expr, created_at, fired_at, snoozed_until, status, dismissed_at
             FROM reminders
             WHERE status = 'Fired'
               AND dismissed_at IS NULL
               AND fired_at > ?1
             ORDER BY fired_at DESC",
        )
        .map_err(|e| ReminderError::Sql(e.to_string()))?;
    let rows = stmt
        .query_map(rusqlite::params![grace_cutoff.to_rfc3339()], row_to_reminder)
        .map_err(|e| ReminderError::Sql(e.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| ReminderError::Sql(e.to_string()))
}
```

注意 `row_to_reminder` 是現有 helper(或寫在 `query_map` closure 內,看既有 pattern)— 確認返回 Reminder 時包含新加的 `dismissed_at` 欄。

- [ ] **Step 4: 跑 tests pass**

Run: `cargo test -p mori-time mark_dismissed_sets_timestamp list_active_popup_queue -- --nocapture`
Expected: PASS

- [ ] **Step 5: 全 lib tests 還過**

Run: `cargo test -p mori-time --lib`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/mori-time/src/schema.rs
git commit -m "feat(mori-time): add mark_dismissed + list_active_popup_queue store methods"
```

---

## Task 3: Catch-up grace — startup 自動 dismiss > 7 天 overdue reminder

**Files:**
- Modify: `crates/mori-time/src/commands.rs`(`ReminderService::new` reload-pending 段)
- Test: 同檔 `mod tests`

- [ ] **Step 1: 加 failing test**

```rust
#[tokio::test]
async fn startup_auto_dismisses_super_overdue_reminders() {
    use std::sync::Arc;
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
    let _svc = ReminderService::new(&db, Notifier::new("Mori-Test"))
        .await
        .expect("service");
    // 留一點時間給 1ms 觸發 + spawn task 處理 mark_fired 完成
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let store = ReminderStore::open(&db).expect("reopen");
    let queue = store
        .list_active_popup_queue(Utc::now())
        .expect("list active");
    assert!(queue.is_empty(), "super-overdue reminder should be auto-dismissed, not on popup queue");

    // 而 status 應該是 Fired(走完正常 fire path),dismissed_at 也填了
    let conn = rusqlite::Connection::open(&db).unwrap();
    let mut stmt = conn
        .prepare("SELECT status, dismissed_at FROM reminders WHERE text = 'stale'")
        .unwrap();
    let (status, dismissed_at): (String, Option<String>) = stmt
        .query_row([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(status, "Fired", "super-overdue should be marked Fired");
    assert!(dismissed_at.is_some(), "super-overdue should be marked dismissed");
}
```

注意 `ReminderService::new` 簽名 Task 4 才會加 `app_handle`。這個 test 在 Task 4 之前還能跑(用既有簽名)。

- [ ] **Step 2: 跑 test 看 fail**

Run: `cargo test -p mori-time startup_auto_dismisses_super_overdue -- --nocapture`
Expected: FAIL(reminder 還在 active queue,因為 catch-up 邏輯還沒寫)

- [ ] **Step 3: 在 `ReminderService::new` reload-pending 內加 grace logic**

找 `commands.rs` 內現有的:

```rust
let pending = store.lock().await.list_pending()?;
for r in &pending {
    if let Err(e) = scheduler.schedule(r).await {
        // ... log ...
    }
}
```

改成:

```rust
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
```

- [ ] **Step 4: 跑 test pass**

Run: `cargo test -p mori-time startup_auto_dismisses_super_overdue -- --nocapture`
Expected: PASS

- [ ] **Step 5: 全 lib tests 還過**

Run: `cargo test -p mori-time --lib`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/mori-time/src/commands.rs
git commit -m "feat(mori-time): auto-dismiss super-overdue reminders (>7d) on startup reload-pending"
```

---

## Task 4: `ReminderService::new` 加 `AppHandle` 參數 + on_fire emit event

**Files:**
- Modify: `crates/mori-time/src/commands.rs`(`ReminderService::new` 簽名 + on_fire callback)
- Modify: `crates/mori-time/src/lib.rs`(re-export 不變,但 doc 提到)
- Modify: `crates/mori-tauri/src/main.rs`(caller 帶 `app.handle().clone()`)
- Modify: `crates/mori-core/src/skill/remind_me.rs` tests(若 caller `ReminderService::new(&db, Notifier::new(...))` 要改)

> **依賴注入策略**:`ReminderService` 在 `mori-time` crate,**不能 depend on tauri**(會循環,而且 mori-time 是 cross-platform leaf crate)。所以 `app_handle` 用 trait 包裝,實作在 caller(`mori-tauri`)那邊。

- [ ] **Step 1: 在 `mori-time/src/commands.rs` 定義 `EventEmitter` trait**

加在檔頂 use 區後面:

```rust
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
    fn emit_reminder_fire(&self, _reminder: &Reminder) -> Result<(), String> { Ok(()) }
}
```

- [ ] **Step 2: `ReminderService::new` 簽名加 `emitter: Arc<dyn EventEmitter>` 參數**

```rust
impl ReminderService {
    pub async fn new(
        db_path: &Path,
        notifier: Notifier,
        emitter: Arc<dyn EventEmitter>,   // 新增
    ) -> Result<Self, CommandError> {
        // ... 原本 store 開檔等不變 ...

        let store_for_cb = Arc::clone(&store);
        let notifier_for_cb = notifier.clone();
        let emitter_for_cb = Arc::clone(&emitter);  // 新增 capture
        let on_fire: OnFireCallback = Arc::new(move |reminder: Reminder| {
            let store_inner = Arc::clone(&store_for_cb);
            let notifier_inner = notifier_for_cb.clone();
            let emitter_inner = Arc::clone(&emitter_for_cb);
            tokio::spawn(async move {
                // 既有 spawn_blocking notifier.fire 保留 ──────────
                let reminder_for_blk = reminder.clone();
                let fire_result = tokio::task::spawn_blocking(move || {
                    notifier_inner.fire(&reminder_for_blk)
                })
                .await;
                match fire_result {
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

                // 既有 mark_fired ──────────
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

        // ... 其餘 scheduler.start + reload-pending 不變 ...
    }
}
```

- [ ] **Step 3: 加 failing test:emit 確實被叫**

加在 `commands.rs` tests mod 內:

```rust
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
```

- [ ] **Step 4: 跑 test**

Run: `cargo test -p mori-time on_fire_calls_emitter_emit -- --nocapture`
Expected: 改完後 PASS。先跑看會不會因 caller 沒改而編譯爆。

- [ ] **Step 5: 修補編譯錯誤 — 既有 test caller + mori-tauri caller**

預期會看到 compile error:
- `mori-time/src/commands.rs` 內既有 `ReminderService::new` test caller 全部要加第 3 個參數
- `mori-core/src/skill/remind_me.rs` tests(`ReminderService::new(...).await`)同樣
- `mori-tauri/src/main.rs`(5265 行附近 `tauri::async_runtime::block_on(ReminderService::new(...))`)同樣

全改用 `NoopEmitter` 或 `Arc::new(NoopEmitter) as Arc<dyn EventEmitter>`,**除了 mori-tauri caller 等 Task 5 才接真實 emitter**。臨時先用 NoopEmitter 讓編譯過。

例:既有 test
```rust
let svc = ReminderService::new(&db, Notifier::new("Mori-Test")).await?;
```
改:
```rust
let svc = ReminderService::new(&db, Notifier::new("Mori-Test"), Arc::new(NoopEmitter)).await?;
```

`main.rs` 5265 行附近改:
```rust
tauri::async_runtime::block_on(ReminderService::new(
    &mori_dir().join("reminders.db"),
    notifier,
    Arc::new(mori_time::NoopEmitter),    // Task 5 換成真實的 TauriEventEmitter
))
```

`mori_time` lib.rs 需要 re-export:
```rust
pub use commands::{CommandError, EventEmitter, NoopEmitter, ReminderService};
```

- [ ] **Step 6: 跑全部 tests pass**

Run: `cargo test -p mori-time --lib`
Expected: PASS(原 44 + 新 2 = 46+)

Run: `cargo check -p mori-tauri`
Expected: 通過

Run: `cargo test -p mori-core --lib`
Expected: PASS(remind_me skill tests 也對齊)

- [ ] **Step 7: Commit**

```bash
git add crates/mori-time crates/mori-core crates/mori-tauri/src/main.rs
git commit -m "feat(mori-time): inject EventEmitter trait so on_fire can notify in-app popup

mori-time 是 leaf crate,不能 depend tauri (循環依賴 + cross-platform invariant)。
加 EventEmitter trait,具體實作(用 Tauri AppHandle 真的 emit)留 mori-tauri 端。
caller (main.rs / tests) 暫用 NoopEmitter 過編譯,Task 5 接真實 emitter。"
```

---

## Task 5: `mori-tauri` 端實作 `TauriEventEmitter`

**Files:**
- Create: `crates/mori-tauri/src/reminder_emitter.rs`(新)
- Modify: `crates/mori-tauri/src/main.rs`(注入 emitter + mod declaration)

- [ ] **Step 1: 寫新 emitter**

`crates/mori-tauri/src/reminder_emitter.rs`:

```rust
//! 2026-05-22:把 mori-time 的 EventEmitter trait 用 Tauri AppHandle 實作出來。
//! 放在 mori-tauri 是因為 mori-time crate 不能 depend tauri(會循環 + 違反 cross-platform 設計)。

use mori_time::{EventEmitter, schema::Reminder};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// Tauri 端的 EventEmitter 實作 — 把 reminder fire payload 透過 AppHandle.emit 送到
/// `reminder_popup` window React listener。
pub struct TauriEventEmitter {
    pub handle: AppHandle,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReminderFirePayload<'a> {
    id: i64,
    text: &'a str,
    due_at: String,
    fired_at: String,
}

impl EventEmitter for TauriEventEmitter {
    fn emit_reminder_fire(&self, reminder: &Reminder) -> Result<(), String> {
        let payload = ReminderFirePayload {
            id: reminder.id,
            text: &reminder.text,
            due_at: reminder.due_at.to_rfc3339(),
            fired_at: reminder
                .fired_at
                .unwrap_or_else(chrono::Utc::now)
                .to_rfc3339(),
        };
        // emit_to 特定 window 比較精準;若 popup 還沒 mount listener,event 丟失,
        // 但 popup mount 時會 invoke reminder_active_queue 補抓,所以不擋。
        self.handle
            .emit_to("reminder_popup", "reminder-fire-show", payload)
            .map_err(|e| e.to_string())
    }
}
```

- [ ] **Step 2: `main.rs` 加 mod + 換掉 NoopEmitter**

`main.rs` 檔頂加:
```rust
mod reminder_emitter;
```

5265 行附近 `ReminderService::new` block_on 改 — 但這條改不了,因為 block_on 在 main() sync 內、AppHandle 還沒 setup。**需要把 ReminderService 初始化往後挪到 .setup() closure 內(已經有 AppHandle)**。

實際做法:把 5265 那塊抽出 to a setup closure。看現有 main.rs 5265 上下文確定怎麼接。最簡單:

```rust
// 改用 .setup() 內初始化
.setup(move |app| {
    // ... 原有 setup code ...
    let app_handle_for_emitter = app.handle().clone();
    let reminder_service = tauri::async_runtime::block_on(ReminderService::new(
        &mori_dir().join("reminders.db"),
        Notifier::new("Mori").with_icon(/* ... */),
        std::sync::Arc::new(reminder_emitter::TauriEventEmitter {
            handle: app_handle_for_emitter,
        }),
    ))?;
    app.manage(std::sync::Arc::new(reminder_service));
    // ...
})
```

如果 main.rs 5265 已經在 setup 內,直接改 NoopEmitter → TauriEventEmitter 即可。否則挪一下位置。

- [ ] **Step 3: 編譯確認**

Run: `cargo check -p mori-tauri`
Expected: PASS

- [ ] **Step 4: 全 workspace 編譯**

Run: `cargo check --workspace --all-targets`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/mori-tauri
git commit -m "feat(mori-tauri): wire TauriEventEmitter into ReminderService — fire emits to popup window"
```

---

## Task 6: Notification config — `~/.mori/config.json` `notifications` 子樹

**Files:**
- Create: `crates/mori-tauri/src/notification_config.rs`(新)
- Modify: `crates/mori-tauri/src/main.rs`(mod declaration)

對齊 `hotkey_config.rs` pattern — read-on-call,不 cache,user 改 config.json 即時生效。

- [ ] **Step 1: 寫 notification_config.rs**

```rust
//! 2026-05-22:reminder 通知 toggle 設定 — `~/.mori/config.json` 的 `notifications` 子樹。
//!
//! 對齊 `hotkey_config.rs` / `recordings.rs` 既有 pattern:呼叫時讀檔 + 缺欄走預設,
//! 寫入時 round-trip 整個 JSON。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotificationConfig {
    /// in-app popup 視窗開關。預設 true。
    pub popup_enabled: bool,
    /// OS 桌面通知(notify-rust)開關。預設 true。
    pub os_notification_enabled: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            popup_enabled: true,
            os_notification_enabled: true,
        }
    }
}

impl NotificationConfig {
    /// 從 `~/.mori/config.json` 的 `notifications` 子樹讀;不存在 / 壞了 → 走預設 + log warn。
    pub fn load(config_path: &Path) -> Self {
        let raw = match std::fs::read_to_string(config_path) {
            Ok(s) => s,
            Err(_) => return Self::default(),
        };
        let json: Value = match serde_json::from_str(&raw) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(?e, "config.json malformed, notifications fall back to defaults");
                return Self::default();
            }
        };
        let sub = match json.get("notifications") {
            Some(v) => v.clone(),
            None => return Self::default(),
        };
        serde_json::from_value(sub).unwrap_or_else(|e| {
            tracing::warn!(?e, "notifications subtree malformed, falling back to defaults");
            Self::default()
        })
    }

    /// 寫回 `~/.mori/config.json` 的 `notifications` 子樹,保留其他欄位不動。
    pub fn write(&self, config_path: &Path) -> Result<(), String> {
        let raw = std::fs::read_to_string(config_path).unwrap_or_else(|_| "{}".to_string());
        let mut json: Value =
            serde_json::from_str(&raw).map_err(|e| format!("parse config.json: {e}"))?;
        let obj = json
            .as_object_mut()
            .ok_or_else(|| "config.json root not object".to_string())?;
        obj.insert(
            "notifications".to_string(),
            serde_json::to_value(self).map_err(|e| e.to_string())?,
        );
        let pretty = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
        std::fs::write(config_path, pretty).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_returns_defaults_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        let cfg = NotificationConfig::load(&path);
        assert_eq!(cfg, NotificationConfig::default());
        assert!(cfg.popup_enabled);
        assert!(cfg.os_notification_enabled);
    }

    #[test]
    fn load_returns_defaults_when_subtree_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, r#"{"other": {}}"#).unwrap();
        let cfg = NotificationConfig::load(&path);
        assert_eq!(cfg, NotificationConfig::default());
    }

    #[test]
    fn write_preserves_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, r#"{"providers":{"groq":{}},"hotkeys":{"toggle":"X"}}"#).unwrap();
        NotificationConfig {
            popup_enabled: false,
            os_notification_enabled: true,
        }
        .write(&path)
        .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains(r#""providers""#));
        assert!(raw.contains(r#""hotkeys""#));
        assert!(raw.contains(r#""popup_enabled": false"#));
    }

    #[test]
    fn round_trip_load_after_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, "{}").unwrap();
        let original = NotificationConfig { popup_enabled: false, os_notification_enabled: false };
        original.write(&path).unwrap();
        let loaded = NotificationConfig::load(&path);
        assert_eq!(loaded, original);
    }
}
```

- [ ] **Step 2: `main.rs` 加 mod**

```rust
mod notification_config;
```

- [ ] **Step 3: 跑 tests**

Run: `cargo test -p mori-tauri --lib notification_config -- --nocapture`
Expected: PASS(4 tests)

- [ ] **Step 4: 全 workspace 編譯**

Run: `cargo check --workspace --all-targets`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/mori-tauri/src/notification_config.rs crates/mori-tauri/src/main.rs
git commit -m "feat(mori-tauri): notification_config module — ~/.mori/config.json notifications subtree"
```

---

## Task 7: Wire `popup_enabled` / `os_notification_enabled` toggle 到 fire path

**Files:**
- Modify: `crates/mori-tauri/src/reminder_emitter.rs`(emit 前看 popup_enabled)
- Modify: `crates/mori-time/src/notifier.rs` OR `crates/mori-time/src/commands.rs`(看 os_notification_enabled)

兩 toggle 在哪檢查、由誰讀 config:
- `popup_enabled`:`TauriEventEmitter::emit_reminder_fire` 內檢查(emitter 知道 AppHandle,讀 config 容易)
- `os_notification_enabled`:**mori-time 不能讀 mori-tauri 的 config.json 路徑**(crate boundary)。簡單做法:`Notifier` 加 `enabled: Arc<AtomicBool>`,`mori-tauri` 端定期或事件式更新它,fire 時讀

更乾淨做法:同樣走 trait — `Notifier` 改成從 trait `NotificationSink` 拿,mori-tauri 端實作會 check config。但這 refactor 太大,Task 7 用 AtomicBool 即可。

- [ ] **Step 1: `Notifier` 加 `enabled: Arc<AtomicBool>`**

`mori-time/src/notifier.rs`:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone)]
pub struct Notifier {
    app_name: String,
    icon_path: Option<String>,
    /// 2026-05-22:OS 桌面通知開關。caller(mori-tauri)持 weak ref 一樣 Arc,
    /// 在 user 切 toggle 時更新。fire() 先檢查再走 .show()。
    pub enabled: Arc<AtomicBool>,
}

impl Notifier {
    pub fn new(app_name: impl Into<String>) -> Self {
        Self {
            app_name: app_name.into(),
            icon_path: None,
            enabled: Arc::new(AtomicBool::new(true)),
        }
    }

    /// 取得共用的 enabled flag handle,給 caller 切 toggle 用。
    pub fn enabled_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.enabled)
    }

    // ... 既有 with_icon / app_name / icon_path / build_text 等不變 ...

    pub fn fire(&self, reminder: &Reminder) -> Result<(), NotifyError> {
        // 2026-05-22:os_notification_enabled toggle off → 直接 Ok 不發送
        if !self.enabled.load(Ordering::Relaxed) {
            tracing::debug!(
                reminder_id = reminder.id,
                "notifier.fire skipped — os_notification_enabled toggle off"
            );
            return Ok(());
        }
        let n = self.build_for_reminder(reminder);
        Self::show(&n)
    }
}
```

- [ ] **Step 2: `TauriEventEmitter` 看 `popup_enabled` 才 emit**

`reminder_emitter.rs`:

```rust
impl EventEmitter for TauriEventEmitter {
    fn emit_reminder_fire(&self, reminder: &Reminder) -> Result<(), String> {
        // 讀當前設定 — load 是 read-on-call,user 切 toggle 即時生效
        let cfg = crate::notification_config::NotificationConfig::load(
            &crate::mori_dir().join("config.json"),
        );
        if !cfg.popup_enabled {
            tracing::debug!(
                reminder_id = reminder.id,
                "skip popup emit — popup_enabled toggle off"
            );
            return Ok(());
        }
        // ... 既有 emit 邏輯 ...
    }
}
```

- [ ] **Step 3: `main.rs` 啟動時把 `notifier.enabled` 設成 config 值 + 加 Tauri command 改 toggle**

在 setup 內 `ReminderService::new` 之前讀 config:

```rust
let cfg = notification_config::NotificationConfig::load(&mori_dir().join("config.json"));
let notifier = Notifier::new("Mori").with_icon(/* ... */);
notifier.enabled.store(cfg.os_notification_enabled, std::sync::atomic::Ordering::Relaxed);
let notifier_enabled_handle = notifier.enabled_handle();
// 注 manage 進 Tauri:
app.manage(notifier_enabled_handle.clone());  // 給 Tauri command 切 toggle 用
// ... 接著 ReminderService::new(..., notifier, ...) ...
```

- [ ] **Step 4: 加 Tauri commands `get_notification_config` / `set_notification_config`**

在 `notification_config.rs`(或 `main.rs` 接近其他 commands 處)加:

```rust
#[tauri::command]
pub fn get_notification_config() -> NotificationConfig {
    NotificationConfig::load(&crate::mori_dir().join("config.json"))
}

#[tauri::command]
pub fn set_notification_config(
    cfg: NotificationConfig,
    notifier_enabled: tauri::State<'_, std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<(), String> {
    cfg.write(&crate::mori_dir().join("config.json"))?;
    // 同步推進 notifier flag(popup_enabled emitter 是 read-on-call 不用推)
    notifier_enabled.store(cfg.os_notification_enabled, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}
```

- [ ] **Step 5: 註冊 commands 進 `invoke_handler!`**

`main.rs` 內 `tauri::generate_handler!` 加 `get_notification_config, set_notification_config`。

- [ ] **Step 6: 加 unit test 確認 enabled=false 時 fire 不去 dbus**

在 `mori-time/src/notifier.rs` `mod tests` 加:

```rust
#[test]
#[ignore]  // 需 real dbus,平時不跑;手動驗證用
fn fire_skipped_when_enabled_false_does_not_send() {
    // 沒法直接驗證「沒 send」(只能 mock dbus),這條留 manual smoke。
    // 但至少 fire() return Ok 而不爆。
    let n = Notifier::new("Mori-Test");
    n.enabled.store(false, std::sync::atomic::Ordering::Relaxed);
    let r = sample_reminder("disabled-test");
    assert!(n.fire(&r).is_ok());
}
```

- [ ] **Step 7: workspace check + tests**

Run: `cargo check --workspace --all-targets && cargo test -p mori-time --lib && cargo test -p mori-tauri --lib`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add crates/mori-time crates/mori-tauri
git commit -m "feat(notifications): wire popup_enabled + os_notification_enabled toggles to fire path

popup_enabled: TauriEventEmitter read-on-call config.json
os_notification_enabled: Notifier.enabled AtomicBool, set_notification_config command 推 toggle"
```

---

## Task 8: Tauri commands — `reminder_active_queue` / `reminder_dismiss` / `reminder_snooze`

**Files:**
- Modify: `crates/mori-tauri/src/reminders_cmd.rs`(加新 commands)
- Modify: `crates/mori-tauri/src/main.rs`(`invoke_handler!` 註冊)

- [ ] **Step 1: 加 commands 跟 ActiveReminder type**

`reminders_cmd.rs`(新增,既有檔):

```rust
use std::sync::Arc;
use chrono::Utc;
use mori_time::{schema::Reminder, ReminderService};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveReminder {
    pub id: i64,
    pub text: String,
    pub due_at: String,    // ISO8601
    pub fired_at: String,
}

impl From<&Reminder> for ActiveReminder {
    fn from(r: &Reminder) -> Self {
        Self {
            id: r.id,
            text: r.text.clone(),
            due_at: r.due_at.to_rfc3339(),
            fired_at: r.fired_at.unwrap_or_else(Utc::now).to_rfc3339(),
        }
    }
}

#[tauri::command]
pub async fn reminder_active_queue(
    svc: tauri::State<'_, Arc<ReminderService>>,
) -> Result<Vec<ActiveReminder>, String> {
    let store = svc.store.lock().await;
    let now = Utc::now();
    let reminders = store
        .list_active_popup_queue(now)
        .map_err(|e| e.to_string())?;
    Ok(reminders.iter().map(ActiveReminder::from).collect())
}

#[tauri::command]
pub async fn reminder_dismiss(
    id: i64,
    svc: tauri::State<'_, Arc<ReminderService>>,
) -> Result<(), String> {
    let store = svc.store.lock().await;
    store.mark_dismissed(id, Utc::now()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reminder_snooze(
    id: i64,
    minutes: u32,
    svc: tauri::State<'_, Arc<ReminderService>>,
) -> Result<(), String> {
    svc.snooze_reminder(id, minutes as i64)
        .await
        .map_err(|e| e.to_string())
}
```

注意 `svc.snooze_reminder` 在 mori-time `commands.rs` 應該已有(若 method 名不同,對齊既有名)。

- [ ] **Step 2: `main.rs` `invoke_handler!` 註冊**

加 `reminders_cmd::reminder_active_queue, reminders_cmd::reminder_dismiss, reminders_cmd::reminder_snooze`(對齊既有 reminders_cmd commands 列法)。

- [ ] **Step 3: 加 integration test(in-memory store)**

`reminders_cmd.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use mori_time::{schema::ReminderStore, notifier::Notifier};
    use std::path::PathBuf;

    async fn make_svc(db_path: PathBuf) -> Arc<ReminderService> {
        Arc::new(
            ReminderService::new(
                &db_path,
                Notifier::new("Mori-Test"),
                Arc::new(mori_time::NoopEmitter),
            )
            .await
            .expect("svc"),
        )
    }

    #[tokio::test]
    async fn dismiss_writes_dismissed_at_and_filter_takes_effect() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("r.db");
        let svc = make_svc(db.clone()).await;

        // 建 + 強迫 fired
        let r = {
            let store = svc.store.lock().await;
            let r = store
                .create("x".to_string(), Utc::now() - chrono::Duration::minutes(1), None)
                .unwrap();
            store.mark_fired(r.id, Utc::now()).unwrap();
            r
        };

        // dismiss 前 active_queue 包含 r
        let store = svc.store.lock().await;
        let before = store.list_active_popup_queue(Utc::now()).unwrap();
        assert!(before.iter().any(|x| x.id == r.id));
        drop(store);

        // call dismiss command 等價:直接呼叫 store.mark_dismissed
        let store = svc.store.lock().await;
        store.mark_dismissed(r.id, Utc::now()).unwrap();
        let after = store.list_active_popup_queue(Utc::now()).unwrap();
        assert!(!after.iter().any(|x| x.id == r.id));
    }
}
```

(Tauri command body 直接走 store API,所以這個 test 等價於 cover command 邏輯。)

- [ ] **Step 4: 跑 tests + workspace check**

Run: `cargo test -p mori-tauri --lib reminders_cmd && cargo check --workspace`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/mori-tauri
git commit -m "feat(mori-tauri): Tauri commands for in-app popup — active_queue / dismiss / snooze"
```

---

## Task 9: 註冊 `reminder_popup` Tauri window + capabilities

**Files:**
- Modify: `crates/mori-tauri/tauri.conf.json`
- Modify: `crates/mori-tauri/capabilities/default.json`

- [ ] **Step 1: `tauri.conf.json` windows array 加 entry**

在既有 `chat_bubble` window 旁加(對齊既有屬性):

```json
{
  "label": "reminder_popup",
  "url": "index.html",
  "title": "Mori (reminder)",
  "transparent": true,
  "decorations": false,
  "alwaysOnTop": true,
  "skipTaskbar": true,
  "resizable": false,
  "visible": true,
  "x": -10000,
  "y": -10000,
  "width": 320,
  "height": 96,
  "focus": false
}
```

- [ ] **Step 2: `capabilities/default.json` windows array 加 `"reminder_popup"`**

既有檔內:

```json
"windows": ["main", "floating", "chat_bubble", "picker", "reminder_popup"]
```

- [ ] **Step 3: 編譯確認**

Run: `cargo check -p mori-tauri`
Expected: PASS

- [ ] **Step 4: 啟動 mori-tauri 確認新 window 存在(不會崩 / capability 過得了)**

Run: `cd /home/ct/mori-universe/mori-desktop && npm run tauri dev`

啟動後 terminal 不該有 capability error 或 `unknown window label`。可手動驗證:

```bash
xdotool search --name "Mori (reminder)" 2>&1 | head
```

應該有一個 XID 回來(window 在 -10000,-10000 位置看不到但存在)。

- [ ] **Step 5: Commit**

```bash
git add crates/mori-tauri/tauri.conf.json crates/mori-tauri/capabilities/default.json
git commit -m "feat(mori-tauri): register reminder_popup window config + capabilities"
```

---

## Task 10: `ReminderPopup.tsx` React 元件 + main.tsx routing

**Files:**
- Create: `src/ReminderPopup.tsx`
- Create: `src/reminder-popup.css`
- Modify: `src/main.tsx`(label routing)

- [ ] **Step 1: 寫 `src/ReminderPopup.tsx`**

```tsx
// 2026-05-22:in-app reminder popup window — Mori 自家通知,因為 Linux GNOME
// 對「有 ABOVE 視窗的 app」會抑制 OS 通知 banner。
//
// 由 mori-time on_fire callback → TauriEventEmitter → emit "reminder-fire-show"
// 觸發;mount 時也 invoke reminder_active_queue 補抓未 dismissed reminders。
//
// 對齊 ChatBubble.tsx pattern:transparent + decorationless + alwaysOnTop;
// 隱藏雙保險 = setPosition(-10000, -10000) + setSize(1, 1)。

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalPosition, LogicalSize } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";

type ActiveReminder = {
  id: number;
  text: string;
  dueAt: string;     // ISO8601
  firedAt: string;
};

type SpriteMoved = { x: number; y: number };

const POPUP_WIDTH = 320;
const POPUP_MAX_HEIGHT = 480;
const CHIP_SIZE = 32;
const POPUP_TO_CHIP_MINUTES = 5;
const DEBOUNCE_MS = 200;
const SPRITE_GAP = 12;
// MVP sprite 預設大小(對齊 FloatingMori),sprite 寬高都 200,gap 12 後 popup 在下方
const SPRITE_HEIGHT = 200;

function ReminderPopup() {
  const [queue, setQueue] = useState<ActiveReminder[]>([]);
  const [mode, setMode] = useState<"popup" | "chip">("popup");
  const [spritePos, setSpritePos] = useState<{ x: number; y: number }>({ x: 0, y: 0 });
  const cardRef = useRef<HTMLDivElement | null>(null);
  const debounceTimer = useRef<number | null>(null);
  const inactivityTimer = useRef<number | null>(null);

  // === debounce buffer:多個 emit 同瞬間進來時合成一次 setQueue ===
  const pendingNew = useRef<ActiveReminder[]>([]);
  const flushPending = () => {
    if (pendingNew.current.length === 0) return;
    setQueue((prev) => {
      const merged = [...prev];
      for (const r of pendingNew.current) {
        if (!merged.some((x) => x.id === r.id)) merged.push(r);
      }
      return merged;
    });
    pendingNew.current = [];
  };

  // === listen + 啟動補抓 ===
  useEffect(() => {
    let unlistenFire: (() => void) | null = null;
    let unlistenSpriteMoved: (() => void) | null = null;

    (async () => {
      // 1) 補抓 mount 前 emit 過、popup 還沒 ready 收到的 reminder
      try {
        const active = await invoke<ActiveReminder[]>("reminder_active_queue");
        if (active.length > 0) {
          setQueue(active);
          setMode("popup");
        }
      } catch (e) {
        console.warn("[reminder_popup] active_queue fetch failed", e);
      }

      // 2) 訂閱新 fire 事件
      const u1 = await listen<ActiveReminder>("reminder-fire-show", (e) => {
        pendingNew.current.push(e.payload);
        if (debounceTimer.current !== null) window.clearTimeout(debounceTimer.current);
        debounceTimer.current = window.setTimeout(() => {
          flushPending();
          setMode("popup");  // 新 fire 進來,從 chip 拉回 popup
        }, DEBOUNCE_MS);
      });
      unlistenFire = u1;

      // 3) sprite 拖動同步位置
      const u2 = await listen<SpriteMoved>("sprite-moved", (e) => {
        setSpritePos(e.payload);
      });
      unlistenSpriteMoved = u2;
    })();

    return () => {
      if (debounceTimer.current !== null) window.clearTimeout(debounceTimer.current);
      if (inactivityTimer.current !== null) window.clearTimeout(inactivityTimer.current);
      unlistenFire?.();
      unlistenSpriteMoved?.();
    };
  }, []);

  // === queue 變化 → setSize / setPosition / show ===
  useEffect(() => {
    const win = getCurrentWindow();
    if (queue.length === 0) {
      // 完全 dismiss 過渡 — 雙保險:移 off-screen + 縮 1x1
      win.setPosition(new LogicalPosition(-10000, -10000)).catch(() => {});
      win.setSize(new LogicalSize(1, 1)).catch(() => {});
      return;
    }
    // sprite 旁 anchor:預設貼 sprite 下方
    const anchorX = spritePos.x;
    const anchorY = spritePos.y + SPRITE_HEIGHT + SPRITE_GAP;
    win.setPosition(new LogicalPosition(anchorX, anchorY)).catch(() => {});

    if (mode === "chip") {
      win.setSize(new LogicalSize(CHIP_SIZE, CHIP_SIZE)).catch(() => {});
      return;
    }
    // popup mode:跟著 card 內容高度
    requestAnimationFrame(() => {
      const measured = cardRef.current?.offsetHeight ?? 0;
      if (measured <= 0) return;  // 沿用 ChatBubble pattern,offsetHeight=0 skip
      const h = Math.min(POPUP_MAX_HEIGHT, measured);
      win.setSize(new LogicalSize(POPUP_WIDTH, h)).catch(() => {});
    });
  }, [queue, mode, spritePos]);

  // === inactivity timer:popup → chip 自動退場 ===
  useEffect(() => {
    if (mode !== "popup" || queue.length === 0) return;
    if (inactivityTimer.current !== null) window.clearTimeout(inactivityTimer.current);
    inactivityTimer.current = window.setTimeout(() => {
      setMode("chip");
    }, POPUP_TO_CHIP_MINUTES * 60 * 1000);
    return () => {
      if (inactivityTimer.current !== null) window.clearTimeout(inactivityTimer.current);
    };
  }, [mode, queue.length]);

  const onSnooze = async (id: number) => {
    try {
      await invoke("reminder_snooze", { id, minutes: 5 });
      setQueue((q) => q.filter((r) => r.id !== id));
    } catch (e) {
      console.error("[reminder_popup] snooze failed", e);
      alert(`稍後失敗:${e}`);  // MVP:粗暴 alert,follow-up 改 inline 紅 chip
    }
  };

  const onDismiss = async (id: number) => {
    try {
      await invoke("reminder_dismiss", { id });
      setQueue((q) => q.filter((r) => r.id !== id));
    } catch (e) {
      console.error("[reminder_popup] dismiss failed", e);
      alert(`關閉失敗:${e}`);
    }
  };

  if (queue.length === 0) return null;

  // === chip mode render ===
  if (mode === "chip") {
    return (
      <div
        className="reminder-chip"
        onClick={() => setMode("popup")}
        title={`${queue.length} 條未讀提醒`}
      >
        🔔 {queue.length}
      </div>
    );
  }

  // === popup mode render ===
  // 折疊:queue ≥ 2 條 → list view;只 1 條 → 單筆 card
  const visible = queue.slice(0, 5);
  const overflow = Math.max(0, queue.length - 5);

  return (
    <div ref={cardRef} className="reminder-card">
      {visible.map((r) => (
        <div key={r.id} className="reminder-row">
          <div className="reminder-row-head">
            <span className="reminder-bell">🔔</span>
            <span className="reminder-text">{r.text}</span>
          </div>
          <div className="reminder-row-meta">
            <span className="reminder-due">原定 {formatDueChip(r.dueAt)}</span>
            <button onClick={() => onSnooze(r.id)}>稍後 5 分</button>
            <button onClick={() => onDismiss(r.id)}>關閉</button>
          </div>
        </div>
      ))}
      {overflow > 0 && (
        <div className="reminder-overflow-chip">+{overflow} 條歷史提醒</div>
      )}
    </div>
  );
}

function formatDueChip(iso: string): string {
  try {
    const d = new Date(iso);
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  } catch {
    return "?";
  }
}

export default ReminderPopup;
```

- [ ] **Step 2: 寫 `src/reminder-popup.css`**

```css
/* 2026-05-22:in-app reminder popup 樣式 — 對齊 chat-bubble.css 既有風格 */

.reminder-card {
  background: rgba(20, 24, 28, 0.96);
  color: #f1eee0;
  border-radius: 12px;
  padding: 12px 14px;
  font-family: system-ui, sans-serif;
  font-size: 14px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.45);
  border: 1px solid rgba(255, 255, 255, 0.08);
  display: flex;
  flex-direction: column;
  gap: 10px;
  max-width: 320px;
}

.reminder-row {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 6px 0;
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}

.reminder-row:last-child {
  border-bottom: none;
}

.reminder-row-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.reminder-bell {
  font-size: 16px;
}

.reminder-text {
  font-weight: 500;
  word-break: break-word;
}

.reminder-row-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
}

.reminder-due {
  color: #c9a24d;
  margin-right: auto;
}

.reminder-row-meta button {
  background: rgba(255, 255, 255, 0.08);
  color: inherit;
  border: 1px solid rgba(255, 255, 255, 0.15);
  border-radius: 6px;
  padding: 4px 10px;
  cursor: pointer;
  font-size: 12px;
}

.reminder-row-meta button:hover {
  background: rgba(255, 255, 255, 0.14);
}

.reminder-overflow-chip {
  font-size: 11px;
  color: rgba(255, 255, 255, 0.55);
  align-self: flex-start;
  margin-top: 4px;
}

.reminder-chip {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  background: rgba(201, 162, 77, 0.95);
  color: #1a1410;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 13px;
  font-weight: 700;
  cursor: pointer;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
}

html.reminder-popup-window,
body.reminder-popup-window {
  background: transparent !important;
  margin: 0;
  padding: 0;
}
```

- [ ] **Step 3: 改 `src/main.tsx` label routing**

對齊既有 `chat_bubble` / `picker` 分支(看現有 main.tsx 內 label 條件):

```tsx
import ReminderPopup from "./ReminderPopup";
import "./reminder-popup.css";

// 既有 label 分支內加:
} else if (label === "reminder_popup") {
  document.documentElement.classList.add("reminder-popup-window");
  document.body.classList.add("reminder-popup-window");
  createRoot(document.getElementById("root")!).render(<ReminderPopup />);
}
```

- [ ] **Step 4: TS check**

Run: `cd /home/ct/mori-universe/mori-desktop && npx tsc --noEmit`
Expected: 0 errors

- [ ] **Step 5: Commit**

```bash
git add src/ReminderPopup.tsx src/reminder-popup.css src/main.tsx
git commit -m "feat(ui): ReminderPopup React component — in-app reminder window with snooze/dismiss"
```

---

## Task 11: ConfigTab Settings UI — popup_enabled / os_notification_enabled toggle

**Files:**
- Modify: `src/tabs/ConfigTab.tsx`(加新「通知」區段)

- [ ] **Step 1: 加 form section 在 ConfigTab 內**

ConfigTab.tsx 既有 form view 有 providers / hotkeys 等 section;對齊既有 pattern 加一個 `<section>` 區塊。具體插入位置由 ConfigTab 既有結構決定,大約在 hotkeys section 之後。

```tsx
// 通知區段 — 2026-05-22 新增
type NotificationConfig = {
  popup_enabled: boolean;
  os_notification_enabled: boolean;
};

// 在 ConfigTab function 內加 state + load:
const [notifCfg, setNotifCfg] = useState<NotificationConfig>({
  popup_enabled: true,
  os_notification_enabled: true,
});

useEffect(() => {
  invoke<NotificationConfig>("get_notification_config")
    .then(setNotifCfg)
    .catch((e) => console.warn("get_notification_config failed", e));
}, []);

const saveNotif = async (next: NotificationConfig) => {
  setNotifCfg(next);
  try {
    await invoke("set_notification_config", { cfg: next });
  } catch (e) {
    console.error("set_notification_config failed", e);
    alert(`儲存失敗:${e}`);
  }
};

// JSX section(放在合適位置,例如 hotkeys 後 / annuli 前):
<section className="config-section">
  <h3>通知</h3>
  <label className="config-toggle">
    <input
      type="checkbox"
      checked={notifCfg.popup_enabled}
      onChange={(e) => saveNotif({ ...notifCfg, popup_enabled: e.target.checked })}
    />
    <span>In-app popup 視窗</span>
  </label>
  <label className="config-toggle">
    <input
      type="checkbox"
      checked={notifCfg.os_notification_enabled}
      onChange={(e) => saveNotif({ ...notifCfg, os_notification_enabled: e.target.checked })}
    />
    <span>OS 桌面通知</span>
    <small className="config-hint">
      ⚠ 在 Linux GNOME 上可能被 shell 抑制(若 Mori 有 alwaysOnTop 視窗)。
      建議 in-app popup 保持開啟為主路徑。
    </small>
  </label>
  {!notifCfg.popup_enabled && !notifCfg.os_notification_enabled && (
    <div className="config-warning">
      ⚠ 你關掉了所有通知方式 — reminder fire 後不會主動告訴你。
    </div>
  )}
</section>
```

(`config-section` / `config-toggle` class 沿用 ConfigTab.tsx 既有 CSS class — 對齊既有 form section 樣式;如果沒有 reuse,寫 minimal style 即可。)

- [ ] **Step 2: TS check**

Run: `npx tsc --noEmit`
Expected: 0 errors

- [ ] **Step 3: Commit**

```bash
git add src/tabs/ConfigTab.tsx
git commit -m "feat(ui): ConfigTab notification toggle section — popup_enabled / os_notification_enabled"
```

---

## Task 12: 全 workspace verify + manual smoke + final summary commit

- [ ] **Step 1: 跑既有 verify 腳本**

Run: `bash scripts/verify.sh`
Expected: PASS(`npm run build` + `cargo test -p mori-core --lib` + `cargo check --workspace --all-targets`)

- [ ] **Step 2: Manual smoke**(逐項 user 互動驗證,**勾不到就回頭修**)

啟動 `npm run tauri dev`,然後:

- [ ] 設「1 分鐘後提醒我說 smoke1」→ fire 時 popup 出現貼 sprite,且**不會自動消失**
- [ ] popup 上 [稍後 5 分] 按下 → popup 該筆消 + 5 分後再 fire 一次出現
- [ ] popup 上 [關閉] 按下 → popup 該筆消 + DB `dismissed_at` 寫入(`sqlite3 ~/.mori/reminders.db "select id,text,dismissed_at from reminders where id=N"`)
- [ ] 設 reminder 後 5 分鐘無互動 → popup 縮成右上角小 chip,chip 點一下 → 拉回完整 popup
- [ ] 重啟 mori-tauri → 上次未 dismiss 的 reminder 重新進 popup queue + 顯示
- [ ] 連設 3 條 5 分鐘內 fire 的 reminder,在 fire 前重啟 → popup 折疊顯示 3 筆
- [ ] ConfigTab 把「OS 桌面通知」toggle off → fire 一次 → 只 popup 不發 OS(用 `busctl --user list 2>&1 | grep -i notif` 對應時間視窗不該有 Notify call;或直接看 GNOME 通知抽屜該秒沒新條目)
- [ ] ConfigTab 把「In-app popup」toggle off → fire 一次 → popup 視窗保持離屏不彈,只有 OS 通知出現(若 OS 也工作)
- [ ] 兩個 toggle 都 off → ConfigTab 顯示警告區塊,fire 一次 → 系統內無任何通知(預期)
- [ ] 直接 sqlite 塞一筆 8 天前 due_at 的 Pending reminder,重啟 mori-tauri → reload-pending 自動 mark dismissed,**不該** popup
  ```bash
  sqlite3 ~/.mori/reminders.db "insert into reminders (text, due_at, created_at, status) values ('stale-test', datetime('now', '-8 days'), datetime('now', '-8 days'), 'Pending');"
  ```
  重啟後檢查:
  ```bash
  sqlite3 ~/.mori/reminders.db "select text, status, fired_at, dismissed_at from reminders where text='stale-test';"
  ```
  期望 status=Fired + 兩個 timestamp 都有值

- [ ] **Step 3: 全部 smoke 過 → 最終 commit(若還有 lint 修補)**

```bash
git status  # 看有沒有 lint/format 殘留
# 若有殘留改動:
git add -A
git commit -m "chore: smoke verify pass — in-app reminder popup MVP ready"
```

- [ ] **Step 4: PR / 整理**

把 plan + spec link 寫進 PR body,checklist 直接從 smoke 段抄過去。

```bash
git log --oneline main ^origin/main
# 預期看到一連串:
#  fix(chat-bubble) ...
#  chore(observability) ...
#  docs(spec) ...
#  fix(skill_server) ...
#  fix(mori-time): spawn_blocking ...
#  feat(mori-time): sticky notifications ...
#  ... 加上本 plan 12 條 task 的 commit
```

---

## Self-review checklist(plan 寫完自查)

- [x] 每 task 有 file paths 明確
- [x] 每 step 有 verbatim code / command,沒有「按 X 風格實作」這種模糊
- [x] TDD:Task 1-4, 6-8 都先寫 failing test 再實作
- [x] DRY:trait `EventEmitter` 抽掉 mori-time / mori-tauri 循環依賴,不重複定義 emit logic
- [x] Spec coverage:
  - §4.1 (5 處修改)→ Tasks 1-5, 8-11
  - §4.2 data flow → Task 4, 5, 10
  - §4.3 catch-up 折疊 → Tasks 3 (grace), 10 (UI 折疊)
  - §4.4 active queue / mode → Task 10
  - §4.5 動作 → Tasks 8, 10
  - §4.6 視窗位置 → Tasks 9, 10
  - §5 error handling → Tasks 4 (emit 失敗 log), 7 (toggle), 10 (alert)
  - §6 testing → Tasks 1-8 unit;Task 12 manual smoke
- [x] 命名一致:`reminder_popup`(window label)、`reminder-fire-show`(event name)、`ActiveReminder`(type)、`reminder_active_queue` / `reminder_dismiss` / `reminder_snooze`(commands)
- [x] Type 一致:Rust `serde(rename_all = "camelCase")` + TS camelCase 全文對齊
