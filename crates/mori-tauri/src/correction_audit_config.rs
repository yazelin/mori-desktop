//! 2026-05-22:語音校正 audit 設定 — `~/.mori/config.json` 的 `correction_audit` 子樹。
//!
//! 對齊 `notification_config.rs` / `hotkey_config.rs` 既有 pattern:呼叫時讀檔 + 缺欄走預設,
//! 寫入時 round-trip 整個 JSON。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CorrectionAuditConfig {
    /// 對話結束後跑 LLM audit。預設 true。
    pub enabled: bool,
    /// LLM provider。預設 "groq"。
    pub provider: String,
    /// model。預設 "openai/gpt-oss-120b"(便宜)。
    pub model: String,
}

impl Default for CorrectionAuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: "groq".into(),
            model: "openai/gpt-oss-120b".into(),
        }
    }
}

impl CorrectionAuditConfig {
    pub fn load(config_path: &Path) -> Self {
        let raw = match std::fs::read_to_string(config_path) {
            Ok(s) => s,
            Err(_) => return Self::default(),
        };
        let json: Value = match serde_json::from_str(&raw) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(?e, "config.json malformed, correction_audit fall back to defaults");
                return Self::default();
            }
        };
        let sub = match json.get("correction_audit") {
            Some(v) => v.clone(),
            None => return Self::default(),
        };
        serde_json::from_value(sub).unwrap_or_else(|e| {
            tracing::warn!(?e, "correction_audit subtree malformed, falling back to defaults");
            Self::default()
        })
    }

    pub fn write(&self, config_path: &Path) -> Result<(), String> {
        let raw = std::fs::read_to_string(config_path).unwrap_or_else(|_| "{}".to_string());
        let mut json: Value =
            serde_json::from_str(&raw).map_err(|e| format!("parse config.json: {e}"))?;
        let obj = json
            .as_object_mut()
            .ok_or_else(|| "config.json root not object".to_string())?;
        obj.insert(
            "correction_audit".to_string(),
            serde_json::to_value(self).map_err(|e| e.to_string())?,
        );
        let pretty = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
        std::fs::write(config_path, pretty).map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn get_correction_audit_config() -> CorrectionAuditConfig {
    CorrectionAuditConfig::load(&crate::mori_dir().join("config.json"))
}

#[tauri::command]
pub fn set_correction_audit_config(cfg: CorrectionAuditConfig) -> Result<(), String> {
    cfg.write(&crate::mori_dir().join("config.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_returns_defaults_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        let cfg = CorrectionAuditConfig::load(&path);
        assert_eq!(cfg, CorrectionAuditConfig::default());
        assert!(cfg.enabled);
        assert_eq!(cfg.provider, "groq");
        assert_eq!(cfg.model, "openai/gpt-oss-120b");
    }

    #[test]
    fn write_preserves_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, r#"{"providers":{"groq":{}},"hotkeys":{"toggle":"X"}}"#).unwrap();
        CorrectionAuditConfig {
            enabled: false,
            provider: "groq".into(),
            model: "x".into(),
        }
        .write(&path)
        .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains(r#""providers""#));
        assert!(raw.contains(r#""hotkeys""#));
        assert!(raw.contains(r#""enabled": false"#));
    }

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, "{}").unwrap();
        let original = CorrectionAuditConfig {
            enabled: false,
            provider: "anthropic".into(),
            model: "claude-haiku".into(),
        };
        original.write(&path).unwrap();
        let loaded = CorrectionAuditConfig::load(&path);
        assert_eq!(loaded, original);
    }
}
