//! 2026-05-23:Deterministic corrections substitute toggle — `~/.mori/config.json`
//! 的 `correction_substitute` 子樹。
//!
//! ON(預設):voice / agent pipeline 在 LLM cleanup 之後對 cleaned text 套 corrections.md
//! 字典條目(strict string replace)。100% reliability,但無上下文判斷。
//!
//! OFF:跳 deterministic substitute,完全靠 LLM cleanup 套字典(漏率高但 context-aware)。
//!
//! 對齊 `correction_audit_config` / `notification_config` 既有 pattern。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CorrectionSubstituteConfig {
    /// 對話結束後對 cleaned text 套 corrections.md 字典(strict string replace)。
    /// 預設 true(100% reliable,字典條目本身是 STT 諧音怪字 context-free 不會誤觸)。
    /// 設 false 跳過,完全靠 LLM cleanup 套字典(漏率高但 context-aware)。
    pub enabled: bool,
}

impl Default for CorrectionSubstituteConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl CorrectionSubstituteConfig {
    pub fn load(config_path: &Path) -> Self {
        let raw = match std::fs::read_to_string(config_path) {
            Ok(s) => s,
            Err(_) => return Self::default(),
        };
        let json: Value = match serde_json::from_str(&raw) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(?e, "config.json malformed, correction_substitute fall back to defaults");
                return Self::default();
            }
        };
        let sub = match json.get("correction_substitute") {
            Some(v) => v.clone(),
            None => return Self::default(),
        };
        serde_json::from_value(sub).unwrap_or_else(|e| {
            tracing::warn!(?e, "correction_substitute subtree malformed, falling back to defaults");
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
            "correction_substitute".to_string(),
            serde_json::to_value(self).map_err(|e| e.to_string())?,
        );
        let pretty = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
        std::fs::write(config_path, pretty).map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn get_correction_substitute_config() -> CorrectionSubstituteConfig {
    CorrectionSubstituteConfig::load(&crate::mori_dir().join("config.json"))
}

#[tauri::command]
pub fn set_correction_substitute_config(cfg: CorrectionSubstituteConfig) -> Result<(), String> {
    cfg.write(&crate::mori_dir().join("config.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_returns_defaults_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        let cfg = CorrectionSubstituteConfig::load(&path);
        assert_eq!(cfg, CorrectionSubstituteConfig::default());
        assert!(cfg.enabled);
    }

    #[test]
    fn write_preserves_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, r#"{"correction_audit":{"enabled":true},"hotkeys":{"toggle":"X"}}"#).unwrap();
        CorrectionSubstituteConfig { enabled: false }.write(&path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains(r#""correction_audit""#));
        assert!(raw.contains(r#""hotkeys""#));
        assert!(raw.contains(r#""enabled": false"#));
    }

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, "{}").unwrap();
        let original = CorrectionSubstituteConfig { enabled: false };
        original.write(&path).unwrap();
        let loaded = CorrectionSubstituteConfig::load(&path);
        assert_eq!(loaded, original);
    }
}
