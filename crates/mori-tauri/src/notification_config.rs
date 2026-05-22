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

/// 取得目前的通知 toggle 設定。
#[tauri::command]
pub fn get_notification_config() -> NotificationConfig {
    NotificationConfig::load(&crate::mori_dir().join("config.json"))
}

/// 寫入通知 toggle 設定,並同步推進 notifier os_notification_enabled flag。
///
/// popup_enabled 是 read-on-call(emitter 每次 fire 都讀 config.json),
/// os_notification_enabled 透過 AtomicBool State 即時推進 Notifier。
#[tauri::command]
pub fn set_notification_config(
    cfg: NotificationConfig,
    notifier_enabled: tauri::State<'_, std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<(), String> {
    cfg.write(&crate::mori_dir().join("config.json"))?;
    // 同步推進 notifier flag(popup_enabled emitter 是 read-on-call 不用推)
    notifier_enabled.store(
        cfg.os_notification_enabled,
        std::sync::atomic::Ordering::Relaxed,
    );
    Ok(())
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
