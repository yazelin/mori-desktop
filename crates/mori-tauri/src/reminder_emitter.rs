//! 2026-05-22:把 mori-time 的 EventEmitter trait 用 Tauri AppHandle 實作出來。
//! 放在 mori-tauri 是因為 mori-time crate 不能 depend tauri(會循環 + 違反 cross-platform 設計)。

use mori_time::{EventEmitter, schema::Reminder};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Tauri 端的 EventEmitter 實作 — 把 reminder fire payload 透過 AppHandle.emit 送到
/// `reminder_popup` window React listener。
pub struct TauriEventEmitter {
    pub handle: AppHandle,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ReminderFirePayload<'a> {
    id: i64,
    text: &'a str,
    due_at: String,
    fired_at: String,
}

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

        tracing::info!(
            reminder_id = reminder.id,
            text = %reminder.text,
            "TauriEventEmitter: about to emit reminder-fire-show to popup window",
        );

        // 加在 emit_to 前:記錄 popup window 狀態,診斷 setPosition/setSize 是否生效
        use tauri::Manager;
        if let Some(popup_win) = self.handle.get_webview_window("reminder_popup") {
            let pos = popup_win.outer_position().ok();
            let size = popup_win.outer_size().ok();
            let visible = popup_win.is_visible().ok();
            tracing::info!(
                ?pos, ?size, ?visible,
                "popup window state pre-emit"
            );
        } else {
            tracing::warn!("reminder_popup window not registered with Manager!");
        }

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
        let result = self
            .handle
            .emit_to("reminder_popup", "reminder-fire-show", payload)
            .map_err(|e| e.to_string());
        tracing::info!(?result, "emit_to reminder-fire-show result");
        result
    }
}
