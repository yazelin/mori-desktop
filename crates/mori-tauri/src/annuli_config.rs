//! annuli 連線設定 — `~/.mori/config.json` 的 `annuli` 子樹。
//!
//! 結構(JSON):
//!
//! ```json
//! {
//!   "annuli": {
//!     "enabled": true,
//!     "endpoint": "http://localhost:5000",
//!     "spirit_name": "mori",
//!     "user_id": "yazelin",
//!     "soul_token": "<random hex>",
//!     "basic_auth": { "user": "ct", "pass": "..." }
//!   }
//! }
//! ```
//!
//! 全欄位可省略。預設 `enabled=false` —— 沒明確設定就走 LocalMarkdownMemoryStore
//! 那條 Wave 2 fallback path,不破現有行為。

use mori_core::annuli::AnnuliClientConfig;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BasicAuth {
    pub user: String,
    pub pass: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnnuliConfig {
    /// 預設 `false`。設 `true` 才會切到 AnnuliMemoryStore;否則走 LocalMarkdown。
    pub enabled: bool,
    /// e.g., `"http://localhost:5000"`。**沒 trailing slash**。
    pub endpoint: String,
    /// vault spirit name(例 `"mori"`、`"jinn"`)。
    pub spirit_name: String,
    /// vault stable user identity。空 → 啟動時從 `<vault>/identity/user_id` 讀;
    /// 還沒有 → 走 fallback(目前 mori-desktop 用 OS user 占位,Wave 4 完整後改 prompt)。
    pub user_id: String,
    /// 可選的 `X-Soul-Token`(僅 `PUT /soul` + `POST /memory/section` 需要)。
    pub soul_token: String,
    /// 可選 basic auth(`ANNULI_ADMIN_USER` / `ANNULI_ADMIN_PASS`)。
    pub basic_auth: Option<BasicAuth>,
    /// request timeout(秒,預設 10)。
    pub timeout_secs: u64,
}

impl Default for AnnuliConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: String::new(),
            spirit_name: String::new(),
            user_id: String::new(),
            soul_token: String::new(),
            basic_auth: None,
            timeout_secs: 10,
        }
    }
}

impl AnnuliConfig {
    /// 從 `~/.mori/config.json` 讀 `annuli` 子樹。檔案 / 子樹缺 → 預設(`enabled=false`)。
    pub fn load(config_path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(config_path) else {
            return Self::default();
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            tracing::warn!(
                ?config_path,
                "config.json malformed, annuli config 走預設(enabled=false)",
            );
            return Self::default();
        };
        let Some(annuli) = json.get("annuli") else {
            return Self::default();
        };
        match serde_json::from_value::<Self>(annuli.clone()) {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::warn!(error = %e, "annuli config 段 parse 失敗,走預設");
                Self::default()
            }
        }
    }

    /// 轉成 `AnnuliClientConfig`,給 `AnnuliClient::new` 用。
    ///
    /// 注意:不 `Default::default` `AnnuliConfig` 直接 unwrap — caller 應該先檢
    /// `enabled == true`,且 endpoint / spirit_name / user_id 至少有值。
    pub fn to_client_config(&self) -> AnnuliClientConfig {
        AnnuliClientConfig {
            endpoint: self.endpoint.clone(),
            spirit_name: self.spirit_name.clone(),
            user_id: self.user_id.clone(),
            soul_token: if self.soul_token.is_empty() {
                None
            } else {
                Some(self.soul_token.clone())
            },
            basic_auth: self
                .basic_auth
                .as_ref()
                .map(|ba| (ba.user.clone(), ba.pass.clone())),
            timeout: Duration::from_secs(self.timeout_secs.max(1)),
        }
    }

    /// `true` 若 `enabled && endpoint!="" && spirit_name!="" && user_id!=""`。
    /// 任一 required 欄位空 → false(避免半設定狀態啟動爆 panic)。
    pub fn is_ready(&self) -> bool {
        self.enabled
            && !self.endpoint.is_empty()
            && !self.spirit_name.is_empty()
            && !self.user_id.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_config(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn missing_file_returns_default() {
        let nope = Path::new("/tmp/this-file-definitely-does-not-exist-xyz.json");
        let cfg = AnnuliConfig::load(nope);
        assert!(!cfg.enabled);
        assert!(!cfg.is_ready());
    }

    #[test]
    fn malformed_json_returns_default() {
        let f = write_config("not json at all {");
        let cfg = AnnuliConfig::load(f.path());
        assert!(!cfg.enabled);
    }

    #[test]
    fn missing_annuli_subtree_returns_default() {
        let f = write_config(r#"{ "hotkeys": { "toggle": "Ctrl+Alt+Space" } }"#);
        let cfg = AnnuliConfig::load(f.path());
        assert!(!cfg.enabled);
    }

    #[test]
    fn minimal_enabled_config_parses() {
        let f = write_config(
            r#"{ "annuli": { "enabled": true, "endpoint": "http://localhost:5000", "spirit_name": "mori", "user_id": "yazelin" } }"#,
        );
        let cfg = AnnuliConfig::load(f.path());
        assert!(cfg.enabled);
        assert_eq!(cfg.endpoint, "http://localhost:5000");
        assert_eq!(cfg.spirit_name, "mori");
        assert_eq!(cfg.user_id, "yazelin");
        assert_eq!(cfg.timeout_secs, 10); // 預設
        assert!(cfg.is_ready());
    }

    #[test]
    fn full_config_with_soul_token_and_basic_auth() {
        let f = write_config(
            r#"{
              "annuli": {
                "enabled": true,
                "endpoint": "http://localhost:5000",
                "spirit_name": "mori",
                "user_id": "yazelin",
                "soul_token": "abc123",
                "basic_auth": { "user": "ct", "pass": "secret" },
                "timeout_secs": 30
              }
            }"#,
        );
        let cfg = AnnuliConfig::load(f.path());
        assert_eq!(cfg.soul_token, "abc123");
        assert_eq!(cfg.basic_auth.as_ref().unwrap().user, "ct");
        assert_eq!(cfg.basic_auth.as_ref().unwrap().pass, "secret");
        assert_eq!(cfg.timeout_secs, 30);
    }

    #[test]
    fn is_ready_false_if_any_required_empty() {
        let mut cfg = AnnuliConfig {
            enabled: true,
            endpoint: "http://localhost:5000".into(),
            spirit_name: "mori".into(),
            user_id: "yazelin".into(),
            ..Default::default()
        };
        assert!(cfg.is_ready());

        cfg.spirit_name = String::new();
        assert!(!cfg.is_ready());

        cfg.spirit_name = "mori".into();
        cfg.user_id = String::new();
        assert!(!cfg.is_ready());

        cfg.user_id = "yazelin".into();
        cfg.endpoint = String::new();
        assert!(!cfg.is_ready());

        cfg.endpoint = "http://localhost:5000".into();
        cfg.enabled = false;
        assert!(!cfg.is_ready());
    }

    #[test]
    fn to_client_config_empty_token_becomes_none() {
        let cfg = AnnuliConfig {
            enabled: true,
            endpoint: "http://localhost:5000".into(),
            spirit_name: "mori".into(),
            user_id: "yazelin".into(),
            soul_token: String::new(),
            basic_auth: None,
            timeout_secs: 10,
        };
        let client_cfg = cfg.to_client_config();
        assert!(client_cfg.soul_token.is_none());
        assert!(client_cfg.basic_auth.is_none());
    }

    #[test]
    fn to_client_config_passes_basic_auth_tuple() {
        let cfg = AnnuliConfig {
            enabled: true,
            endpoint: "http://localhost:5000".into(),
            spirit_name: "mori".into(),
            user_id: "yazelin".into(),
            soul_token: "token".into(),
            basic_auth: Some(BasicAuth {
                user: "u".into(),
                pass: "p".into(),
            }),
            timeout_secs: 10,
        };
        let client_cfg = cfg.to_client_config();
        assert_eq!(client_cfg.soul_token.as_deref(), Some("token"));
        assert_eq!(
            client_cfg.basic_auth.as_ref(),
            Some(&("u".to_string(), "p".to_string()))
        );
    }
}
