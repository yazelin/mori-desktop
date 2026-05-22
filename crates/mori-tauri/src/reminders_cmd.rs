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

use mori_time::{Reminder, ReminderService};

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
}
