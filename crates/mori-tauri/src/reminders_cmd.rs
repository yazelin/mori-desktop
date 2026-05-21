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

use std::sync::Arc;

use mori_time::{Reminder, ReminderService};

/// `remind_me(text, when)` — 設一次性 reminder。`when` 走 K4 NL parser。
#[tauri::command]
pub async fn remind_me_cmd(
    state: tauri::State<'_, Arc<ReminderService>>,
    text: String,
    when: String,
) -> Result<Reminder, String> {
    state
        .remind_me(text, when)
        .await
        .map_err(|e| e.to_string())
}

/// `list_reminders()` — 列 pending + snoozed,由 due_at ASC 排。
#[tauri::command]
pub async fn list_reminders_cmd(
    state: tauri::State<'_, Arc<ReminderService>>,
) -> Result<Vec<Reminder>, String> {
    state.list_reminders().await.map_err(|e| e.to_string())
}

/// `cancel_reminder(id)` — 取消(scheduler 停 + store 標 Cancelled)。
#[tauri::command]
pub async fn cancel_reminder_cmd(
    state: tauri::State<'_, Arc<ReminderService>>,
    id: i64,
) -> Result<(), String> {
    state.cancel_reminder(id).await.map_err(|e| e.to_string())
}

/// `snooze_reminder(id, when)` — 暫緩到 `when`(NL 時間)。
#[tauri::command]
pub async fn snooze_reminder_cmd(
    state: tauri::State<'_, Arc<ReminderService>>,
    id: i64,
    when: String,
) -> Result<(), String> {
    state
        .snooze_reminder(id, when)
        .await
        .map_err(|e| e.to_string())
}
