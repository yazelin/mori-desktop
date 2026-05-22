//! Tauri commands bridging [`mori_time::ReminderService`] 到 IPC / LLM tool。
//!
//! 「時之鳥」整合層 — mori-time::ReminderService 是純 lib,這層把它包成
//! `#[tauri::command]`,讓:
//! - 前端 JS / UI 透過 `invoke("remind_me_cmd", { text, when })` 設提醒、列、取消、snooze
//! - LLM 透過 system prompt 內的 `remind_me` 工具描述也能叫到(skill dispatch 走的是
//!   [`mori_core::skill::RemindMeSkill`] — 同 service Arc,不同入口)
//!
//! 4 條 commands 命名對齊既有 `_cmd` 風格(`transcribe_*_cmd` / `read_file_text_cmd`),
//! 區分「Tauri 入口」vs「底層 lib 函式」。
//!
//! 失敗一律收成 `String` — Tauri IPC 需要 `Serialize` error,且前端 / LLM 端拿到
//! 文字訊息比 typed enum 更可讀。內部完整型別在 [`mori_time::CommandError`],
//! 走 `Display`(`#[error(..)]`)轉字串。
//!
//! ## 為何抽 `do_*` helper?
//!
//! `#[tauri::command]` 的 fn 簽章帶 `tauri::State<'_, _>`,在 unit test 內 mock 出
//! 一個合法的 `State` 很麻煩(需要 `Manager` 或 mock 整個 Tauri runtime)。對齊
//! `file_loader_cmd::read_file_text_cmd` 的測試風格(那邊 fn 沒 State 所以可直接 call),
//! 這裡把實質邏輯拆成不帶 State 的 `do_*` 內部 helper,Tauri command 純 wrap。
//! `do_*` 可在 plain `#[tokio::test]` 內直接 call,不需要 Tauri runtime。

use std::sync::Arc;

use chrono::Utc;
use mori_time::{Reminder, ReminderService};
use serde::Serialize;
use tauri::Manager;

// ─────────────────────────────────────────────────────────────────────
// Popup-queue 型別 — 給前端 reminder popup 用的精簡 view
// ─────────────────────────────────────────────────────────────────────

/// 前端 popup 收到的 reminder 快照。camelCase → TS 端 `dueAt` / `firedAt`。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveReminder {
    pub id: i64,
    pub text: String,
    pub due_at: String,   // ISO8601 RFC3339
    pub fired_at: String, // ISO8601 RFC3339(若 fired_at 為 None,填 Utc::now())
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

// ─────────────────────────────────────────────────────────────────────
// 內部 helpers — 無 Tauri 依賴,可直接 unit test
// ─────────────────────────────────────────────────────────────────────

/// `remind_me_cmd` 的核心邏輯,不帶 `tauri::State`。
pub(crate) async fn do_remind_me(
    service: &ReminderService,
    text: String,
    when: String,
) -> Result<Reminder, String> {
    service.remind_me(text, when).await.map_err(|e| e.to_string())
}

/// `list_reminders_cmd` 的核心邏輯。
pub(crate) async fn do_list_reminders(
    service: &ReminderService,
) -> Result<Vec<Reminder>, String> {
    service.list_reminders().await.map_err(|e| e.to_string())
}

/// `cancel_reminder_cmd` 的核心邏輯。
pub(crate) async fn do_cancel_reminder(
    service: &ReminderService,
    id: i64,
) -> Result<(), String> {
    service.cancel_reminder(id).await.map_err(|e| e.to_string())
}

/// `snooze_reminder_cmd` 的核心邏輯。
pub(crate) async fn do_snooze_reminder(
    service: &ReminderService,
    id: i64,
    when: String,
) -> Result<(), String> {
    service.snooze_reminder(id, when).await.map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────────────────────────────
// Tauri commands — 純 wrap helpers
// ─────────────────────────────────────────────────────────────────────

/// `remind_me(text, when)` — 設一次性 reminder。`when` 走 K4 NL parser。
#[tauri::command]
pub async fn remind_me_cmd(
    state: tauri::State<'_, Arc<ReminderService>>,
    text: String,
    when: String,
) -> Result<Reminder, String> {
    do_remind_me(state.inner(), text, when).await
}

/// `list_reminders()` — 列 pending + snoozed,由 due_at ASC 排。
#[tauri::command]
pub async fn list_reminders_cmd(
    state: tauri::State<'_, Arc<ReminderService>>,
) -> Result<Vec<Reminder>, String> {
    do_list_reminders(state.inner()).await
}

/// `cancel_reminder(id)` — 取消(scheduler 停 + store 標 Cancelled)。
#[tauri::command]
pub async fn cancel_reminder_cmd(
    state: tauri::State<'_, Arc<ReminderService>>,
    id: i64,
) -> Result<(), String> {
    do_cancel_reminder(state.inner(), id).await
}

/// `snooze_reminder(id, when)` — 暫緩到 `when`(NL 時間)。
#[tauri::command]
pub async fn snooze_reminder_cmd(
    state: tauri::State<'_, Arc<ReminderService>>,
    id: i64,
    when: String,
) -> Result<(), String> {
    do_snooze_reminder(state.inner(), id, when).await
}

// ─────────────────────────────────────────────────────────────────────
// Popup-queue commands — in-app reminder popup 系列
// ─────────────────────────────────────────────────────────────────────

/// `reminder_active_queue()` — 回傳已 fired 但尚未 dismissed 的 reminders
/// (由 `ReminderStore::list_active_popup_queue` 過濾)。
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

/// `reminder_dismiss(id)` — 標記 reminder 為 user 已 dismiss(寫 `dismissed_at`)。
#[tauri::command]
pub async fn reminder_dismiss(
    id: i64,
    svc: tauri::State<'_, Arc<ReminderService>>,
) -> Result<(), String> {
    let store = svc.store.lock().await;
    store.mark_dismissed(id, Utc::now()).map_err(|e| e.to_string())
}

/// `reminder_snooze(id, minutes)` — 暫緩 `minutes` 分鐘。
/// 內部轉換成 NL 字串並走 `ReminderService::snooze_reminder` NL parser 路徑。
#[tauri::command]
pub async fn reminder_snooze(
    id: i64,
    minutes: u32,
    svc: tauri::State<'_, Arc<ReminderService>>,
) -> Result<(), String> {
    svc.snooze_reminder(id, format!("{} minutes", minutes))
        .await
        .map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────────────────────────────
// Sprite position query — popup mount 時主動拿 sprite 位置
// ─────────────────────────────────────────────────────────────────────

/// `get_sprite_position()` — 回傳 floating sprite window 的目前邏輯座標。
///
/// ReminderPopup mount 時用這個補抓 sprite 位置(只在拖動後才 emit sprite-moved,
/// mount 時 spritePos 預設 (0,0),anchor 算成 (0, 212) → 不在任何 monitor 範圍)。
///
/// 失敗(floating window 不存在或 Tauri API 失敗)→ 回 Err,前端 fallback 用 (0,0)。
#[tauri::command]
pub fn get_sprite_position(
    app: tauri::AppHandle,
) -> Result<SpritePosition, String> {
    let win = app
        .get_webview_window("floating")
        .ok_or_else(|| "floating window not found".to_string())?;
    let phys = win.outer_position().map_err(|e| e.to_string())?;
    let scale = win.scale_factor().unwrap_or(1.0);
    Ok(SpritePosition {
        x: phys.x as f64 / scale,
        y: phys.y as f64 / scale,
    })
}

#[derive(Serialize)]
pub struct SpritePosition {
    pub x: f64,
    pub y: f64,
}

#[cfg(test)]
mod tests {
    //! 對齊 `file_loader_cmd::tests` 的風格 — Tauri State / runtime mock 麻煩,
    //! 我們直接 unit-test 內部 `do_*` helpers(`#[tauri::command]` wrapper 只是
    //! 註冊 macro,純 wrap 不影響 helper 行為)。
    //!
    //! 用 `tempfile::TempDir` + `ReminderService::new`(真的 sqlite,走 prod path);
    //! Notifier 在 CI 沒 dbus 會 fire-err,但 service callback 內部只 log warn,
    //! `do_*` helpers 不會受影響(我們只測 happy + error paths,不等 fire)。
    use super::*;
    use mori_time::{NoopEmitter, Notifier};
    use tempfile::TempDir;

    async fn make_test_service(dir: &TempDir) -> Arc<ReminderService> {
        let db = dir.path().join("test.db");
        let notifier = Notifier::new("MoriTauriTest");
        Arc::new(
            ReminderService::new(&db, notifier, Arc::new(NoopEmitter))
                .await
                .expect("new service"),
        )
    }

    #[tokio::test]
    async fn do_remind_me_creates_reminder() {
        let dir = TempDir::new().unwrap();
        let svc = make_test_service(&dir).await;
        let r = do_remind_me(&svc, "喝水".into(), "30 minutes".into())
            .await
            .expect("remind_me ok");
        assert!(r.id > 0);
        assert_eq!(r.text, "喝水");
    }

    #[tokio::test]
    async fn do_remind_me_returns_error_for_unparseable_when() {
        let dir = TempDir::new().unwrap();
        let svc = make_test_service(&dir).await;
        let err = do_remind_me(&svc, "test".into(), "qwerty foobar".into())
            .await
            .expect_err("garbage time should fail");
        // 錯誤是 String,只驗 Display 帶有 parse 字樣即可
        assert!(
            err.to_lowercase().contains("parse"),
            "expected parse error message, got: {err}",
        );
    }

    #[tokio::test]
    async fn do_list_reminders_returns_empty_initially() {
        let dir = TempDir::new().unwrap();
        let svc = make_test_service(&dir).await;
        let pending = do_list_reminders(&svc).await.expect("list ok");
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn do_list_reminders_returns_created() {
        let dir = TempDir::new().unwrap();
        let svc = make_test_service(&dir).await;
        let r = do_remind_me(&svc, "x".into(), "1 hour".into())
            .await
            .unwrap();
        let pending = do_list_reminders(&svc).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, r.id);
    }

    #[tokio::test]
    async fn do_cancel_reminder_cancels() {
        let dir = TempDir::new().unwrap();
        let svc = make_test_service(&dir).await;
        let r = do_remind_me(&svc, "bye".into(), "1 hour".into())
            .await
            .unwrap();
        do_cancel_reminder(&svc, r.id).await.expect("cancel ok");
        let pending = do_list_reminders(&svc).await.unwrap();
        assert!(pending.iter().all(|x| x.id != r.id));
    }

    #[tokio::test]
    async fn do_snooze_reminder_snoozes() {
        let dir = TempDir::new().unwrap();
        let svc = make_test_service(&dir).await;
        let r = do_remind_me(&svc, "snz".into(), "1 hour".into())
            .await
            .unwrap();
        do_snooze_reminder(&svc, r.id, "2 hours".into())
            .await
            .expect("snooze ok");
        // snooze 仍在 pending list(K1 list_pending 含 Snoozed)
        let pending = do_list_reminders(&svc).await.unwrap();
        assert!(pending.iter().any(|x| x.id == r.id));
    }

    #[tokio::test]
    async fn do_snooze_reminder_propagates_invalid_status() {
        // K5 snooze guard 應從 helper 透傳成 String error。
        let dir = TempDir::new().unwrap();
        let svc = make_test_service(&dir).await;
        let r = do_remind_me(&svc, "done".into(), "1 hour".into())
            .await
            .unwrap();
        // cancel → Cancelled
        do_cancel_reminder(&svc, r.id).await.unwrap();
        // snooze Cancelled 必須 err
        let err = do_snooze_reminder(&svc, r.id, "1 hour".into())
            .await
            .expect_err("snooze cancelled should fail");
        assert!(
            err.contains("can't be snoozed"),
            "expected guard message, got: {err}",
        );
    }

    // ── popup-queue command integration tests ────────────────────────

    #[tokio::test]
    async fn dismiss_writes_dismissed_at_and_filter_takes_effect() {
        let dir = TempDir::new().unwrap();
        let svc = make_test_service(&dir).await;

        // 建 reminder 並強迫 mark_fired(模擬排程觸發)
        let r = {
            let store = svc.store.lock().await;
            let r = store
                .create(
                    "popup-test".to_string(),
                    Utc::now() - chrono::Duration::minutes(1),
                    None,
                )
                .unwrap();
            store.mark_fired(r.id, Utc::now()).unwrap();
            r
        };

        // dismiss 前,active_queue 應包含 r
        {
            let store = svc.store.lock().await;
            let before = store.list_active_popup_queue(Utc::now()).unwrap();
            assert!(
                before.iter().any(|x| x.id == r.id),
                "fired reminder should appear in active queue before dismiss"
            );
        }

        // 呼叫 store.mark_dismissed — 等價 reminder_dismiss command 邏輯
        {
            let store = svc.store.lock().await;
            store.mark_dismissed(r.id, Utc::now()).unwrap();
        }

        // dismiss 後,active_queue 應不含 r
        let store = svc.store.lock().await;
        let after = store.list_active_popup_queue(Utc::now()).unwrap();
        assert!(
            !after.iter().any(|x| x.id == r.id),
            "dismissed reminder should NOT appear in active queue after dismiss"
        );
    }

    #[tokio::test]
    async fn active_reminder_from_converts_fields_correctly() {
        use mori_time::schema::Reminder;
        use chrono::TimeZone;

        let due = Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).unwrap();
        let fired = Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 5).unwrap();
        let r = Reminder {
            id: 42,
            text: "hello".to_string(),
            due_at: due,
            cron_expr: None,
            created_at: due,
            fired_at: Some(fired),
            snoozed_until: None,
            status: mori_time::ReminderStatus::Fired,
            dismissed_at: None,
        };
        let ar = ActiveReminder::from(&r);
        assert_eq!(ar.id, 42);
        assert_eq!(ar.text, "hello");
        assert_eq!(ar.due_at, due.to_rfc3339());
        assert_eq!(ar.fired_at, fired.to_rfc3339());
    }
}
