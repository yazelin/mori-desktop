//! BI-2:Permission Broker 的 mori-tauri 薄 shim。
//! 政策 / 評估 / audit 邏輯全在 mori_core::body;這裡只決定 audit 檔案位置、
//! 接上預設政策、產生 RFC3339 timestamp。

use mori_core::body::{broker_decide, default_policy, BrokerResponse, PermissionRequest};
use std::path::{Path, PathBuf};

/// `~/.mori/permission-audit.jsonl`。與 ~/.mori/body-parts 同根(crate::mori_dir())。
pub fn audit_path() -> PathBuf {
    crate::mori_dir().join("permission-audit.jsonl")
}

/// 對指定 audit 路徑評估 + 記錄(可測)。
pub fn decide_at(req: &PermissionRequest, audit_path: &Path) -> Result<BrokerResponse, String> {
    let now = chrono::Utc::now().to_rfc3339();
    broker_decide(req, &default_policy(), audit_path, &now)
}

/// 對真實 ~/.mori audit 路徑評估 + 記錄。
pub fn decide(req: &PermissionRequest) -> Result<BrokerResponse, String> {
    decide_at(req, &audit_path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mori_core::body::{Decision, RiskClass};

    fn req(risk: RiskClass) -> PermissionRequest {
        PermissionRequest {
            schema_version: 1,
            request_id: "r1".into(),
            session_id: None,
            source: "demo".into(),
            tool: "shell.exec".into(),
            risk,
            reason: None,
            scope: None,
        }
    }

    #[test]
    fn decide_at_uses_default_policy_and_writes_tempfile() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("permission-audit.jsonl");
        // exec.safe → allow;且 audit 檔被寫出。
        let resp = decide_at(&req(RiskClass::ExecSafe), &path).unwrap();
        assert_eq!(resp.decision, Decision::Allow);
        assert!(path.exists());
        // exec.destructive → deny。
        let resp = decide_at(&req(RiskClass::ExecDestructive), &path).unwrap();
        assert_eq!(resp.decision, Decision::Deny);
    }
}
