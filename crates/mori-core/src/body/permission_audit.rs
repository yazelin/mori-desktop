//! Permission audit log — 每筆 broker 決策 append 一行 JSON 到 `~/.mori/permission-audit.jsonl`。
//! `broker_decide` = evaluate + 寫 audit 的組合器;audit 寫不下去 → Err(fail-safe:
//! 記不下來的授權不算數)。讀用 tail(最後 N 筆,新到舊)。

use crate::body::permission::{
    evaluate, BrokerResponse, Decision, PermissionRequest, PolicyTable, RiskClass,
};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

/// audit log 的一筆紀錄(request 快照 + 決策 + 時間)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionAuditEntry {
    pub timestamp: String,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub source: String,
    pub tool: String,
    pub risk: RiskClass,
    pub decision: Decision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl PermissionAuditEntry {
    fn from_decision(req: &PermissionRequest, decision: Decision, now: &str) -> Self {
        Self {
            timestamp: now.to_string(),
            request_id: req.request_id.clone(),
            session_id: req.session_id.clone(),
            source: req.source.clone(),
            tool: req.tool.clone(),
            risk: req.risk,
            decision,
            reason: req.reason.clone(),
        }
    }
}

/// 組合器:評估 + 寫 audit + 回 BrokerResponse。
/// `now` 由呼叫端給(RFC3339),讓測試可決定性。audit 寫不下去 → Err(fail-safe)。
pub fn broker_decide(
    req: &PermissionRequest,
    policy: &PolicyTable,
    audit_path: &Path,
    now: &str,
) -> Result<BrokerResponse, String> {
    let decision = evaluate(req, policy);
    let entry = PermissionAuditEntry::from_decision(req, decision, now);
    append_audit(audit_path, &entry)?;
    Ok(BrokerResponse {
        request_id: req.request_id.clone(),
        decision,
        lease: None, // P8:保留不實作
    })
}

/// append 一行 JSON 到 audit log(建父目錄、append-only)。
pub fn append_audit(path: &Path, entry: &PermissionAuditEntry) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(f, "{line}").map_err(|e| e.to_string())?;
    Ok(())
}

/// 讀最後 `limit` 筆,新到舊。檔案不存在 → 空;壞行跳過(不 fatal)。
pub fn read_audit_tail(path: &Path, limit: usize) -> Vec<PermissionAuditEntry> {
    let body = match std::fs::read_to_string(path) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let mut entries: Vec<PermissionAuditEntry> = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    entries.reverse(); // 新到舊
    entries.truncate(limit);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::permission::{default_policy, Decision, PermissionRequest, RiskClass};
    use tempfile::TempDir;

    fn req(risk: RiskClass, id: &str) -> PermissionRequest {
        PermissionRequest {
            schema_version: 1,
            request_id: id.into(),
            session_id: None,
            source: "agent.plus".into(),
            tool: "shell.exec".into(),
            risk,
            reason: Some("r".into()),
            scope: None,
        }
    }

    #[test]
    fn broker_decide_denies_destructive_and_writes_audit() {
        // backlog 完成判準:一條 fake 高風險請求被 broker 攔下並寫 audit log。
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("permission-audit.jsonl");
        let resp = broker_decide(
            &req(RiskClass::ExecDestructive, "r1"),
            &default_policy(),
            &path,
            "2026-05-28T10:00:00+08:00",
        )
        .expect("decide ok");
        assert_eq!(resp.decision, Decision::Deny);
        assert!(resp.lease.is_none());

        let tail = read_audit_tail(&path, 10);
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].request_id, "r1");
        assert_eq!(tail[0].decision, Decision::Deny);
        assert_eq!(tail[0].risk, RiskClass::ExecDestructive);
        assert_eq!(tail[0].timestamp, "2026-05-28T10:00:00+08:00");
    }

    #[test]
    fn tail_returns_newest_first_and_caps_limit() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("a.jsonl");
        for i in 0..5 {
            broker_decide(
                &req(RiskClass::ReadPublic, &format!("r{i}")),
                &default_policy(),
                &path,
                "2026-05-28T10:00:00+08:00",
            )
            .unwrap();
        }
        let tail = read_audit_tail(&path, 2);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].request_id, "r4"); // 新到舊
        assert_eq!(tail[1].request_id, "r3");
    }

    #[test]
    fn read_missing_file_returns_empty() {
        let tmp = TempDir::new().unwrap();
        assert!(read_audit_tail(&tmp.path().join("nope.jsonl"), 10).is_empty());
    }

    #[test]
    fn corrupt_line_is_skipped_not_fatal() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("a.jsonl");
        broker_decide(
            &req(RiskClass::ReadPublic, "good"),
            &default_policy(),
            &path,
            "2026-05-28T10:00:00+08:00",
        )
        .unwrap();
        // 手動插一行壞 JSON
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "{{ not json").unwrap();
        let tail = read_audit_tail(&path, 10);
        assert_eq!(tail.len(), 1); // 壞行被跳過
        assert_eq!(tail[0].request_id, "good");
    }
}
