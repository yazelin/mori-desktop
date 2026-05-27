//! Permission Broker — body part / external agent 提出 tool request,broker 依
//! risk class 給 allow/deny/ask。見 docs/mori-body-interface.md §Permission Broker。
//! BI-2 最小版:決策只看 risk class(純查表);未知 risk → deny(§Versioning)。

use serde::{Deserialize, Serialize};

/// 目前支援的 permission envelope schema major。
pub const SUPPORTED_PERMISSION_SCHEMA: u32 = 1;

/// 風險分級 — 對齊 docs/mori-body-interface.md §Permission classes 表。
/// 未知字串吃進 `Unknown`(不 crash),evaluate 時 → Deny。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RiskClass {
    #[serde(rename = "read.public")]
    ReadPublic,
    #[serde(rename = "read.project")]
    ReadProject,
    #[serde(rename = "read.private")]
    ReadPrivate,
    #[serde(rename = "write.project")]
    WriteProject,
    #[serde(rename = "write.private")]
    WritePrivate,
    #[serde(rename = "exec.safe")]
    ExecSafe,
    #[serde(rename = "exec.risky")]
    ExecRisky,
    #[serde(rename = "exec.destructive")]
    ExecDestructive,
    #[serde(rename = "audio.capture")]
    AudioCapture,
    #[serde(rename = "network.external")]
    NetworkExternal,
    #[serde(other)]
    Unknown,
}

impl Default for RiskClass {
    fn default() -> Self {
        // 缺 risk 的請求視為未知 → 之後 evaluate 會 deny(fail-safe)。
        RiskClass::Unknown
    }
}

/// broker 的決策。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Deny,
    Ask,
}

/// body part / external agent 的 tool request envelope。
/// 對齊 docs/mori-body-interface.md §Tool request envelope。多餘欄位(args 等)
/// 由 serde 忽略;缺的非必要欄位 degrade 成預設,不 crash。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    #[serde(default = "default_schema")]
    pub schema_version: u32,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// 哪個 body part / agent 提出(doc envelope 無此欄 → 預設空字串)。
    #[serde(default)]
    pub source: String,
    pub tool: String,
    #[serde(default)]
    pub risk: RiskClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// cwd / project 等;BI-2 不解讀,只透傳 audit。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<serde_json::Value>,
}

fn default_schema() -> u32 {
    SUPPORTED_PERMISSION_SCHEMA
}

/// broker 回覆 — 對齊 docs/mori-body-interface.md §broker 回覆。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerResponse {
    pub request_id: String,
    pub decision: Decision,
    /// 保留欄位(doc 的 lease)。BI-2 沒有 lease 消費者 → 永遠 None
    /// (見 backlog §1 範圍紀律:無真實消費者的欄位只保留不實作)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<Lease>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub expires_at: String,
    pub max_uses: u32,
}

/// 一條政策規則:某 risk class 的預設決策。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRule {
    pub risk: RiskClass,
    pub decision: Decision,
}

/// 政策表 — risk class → 預設決策。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyTable {
    pub rules: Vec<PolicyRule>,
}

/// BI-2 hardcoded 預設政策,對齊 docs/mori-body-interface.md §Permission classes 表。
/// `read.project` 的 doc 是「ask / scoped allow」→ 取保守的 ask。
pub fn default_policy() -> PolicyTable {
    use Decision::*;
    use RiskClass::*;
    let rule = |risk, decision| PolicyRule { risk, decision };
    PolicyTable {
        rules: vec![
            rule(ReadPublic, Allow),
            rule(ReadProject, Ask),
            rule(ReadPrivate, Ask),
            rule(WriteProject, Ask),
            rule(WritePrivate, Deny),
            rule(ExecSafe, Allow),
            rule(ExecRisky, Ask),
            rule(ExecDestructive, Deny),
            rule(AudioCapture, Ask),
            rule(NetworkExternal, Ask),
        ],
    }
}

/// 純函式:依政策表把 request 的 risk class 對到決策。
/// 未列 / 未知 class → Deny(§Versioning「對高風險未知 permission 預設 deny」)。
pub fn evaluate(req: &PermissionRequest, policy: &PolicyTable) -> Decision {
    policy
        .rules
        .iter()
        .find(|r| r.risk == req.risk)
        .map(|r| r.decision)
        .unwrap_or(Decision::Deny)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(risk: RiskClass) -> PermissionRequest {
        PermissionRequest {
            schema_version: 1,
            request_id: "toolreq_001".into(),
            session_id: Some("sess_abc".into()),
            source: "agent.plus".into(),
            tool: "shell.exec".into(),
            risk,
            reason: Some("test".into()),
            scope: None,
        }
    }

    #[test]
    fn allow_path_for_low_risk() {
        let p = default_policy();
        assert_eq!(evaluate(&req(RiskClass::ReadPublic), &p), Decision::Allow);
        assert_eq!(evaluate(&req(RiskClass::ExecSafe), &p), Decision::Allow);
    }

    #[test]
    fn deny_path_for_destructive_and_private_write() {
        let p = default_policy();
        assert_eq!(evaluate(&req(RiskClass::ExecDestructive), &p), Decision::Deny);
        assert_eq!(evaluate(&req(RiskClass::WritePrivate), &p), Decision::Deny);
    }

    #[test]
    fn ask_path_for_medium_risk() {
        let p = default_policy();
        assert_eq!(evaluate(&req(RiskClass::ReadProject), &p), Decision::Ask);
        assert_eq!(evaluate(&req(RiskClass::AudioCapture), &p), Decision::Ask);
    }

    #[test]
    fn unknown_risk_defaults_to_deny() {
        // §Versioning:對高風險未知 permission 預設 deny。
        let json = r#"{"request_id":"r","source":"x","tool":"t","risk":"quantum.teleport"}"#;
        let r: PermissionRequest = serde_json::from_str(json).expect("unknown risk still parses");
        assert_eq!(r.risk, RiskClass::Unknown);
        assert_eq!(evaluate(&r, &default_policy()), Decision::Deny);
    }

    #[test]
    fn request_parses_doc_envelope() {
        // docs/mori-body-interface.md §Tool request envelope 的形狀。
        let json = r#"{
            "request_id":"toolreq_001","session_id":"sess_abc","tool":"shell.exec",
            "args":{"command":["cargo","test"]},
            "scope":{"cwd":"/x","project":"mori-desktop"},
            "risk":"exec.safe","reason":"Run tests."
        }"#;
        let r: PermissionRequest = serde_json::from_str(json).expect("doc envelope parses");
        assert_eq!(r.risk, RiskClass::ExecSafe);
        assert_eq!(r.source, ""); // source 預設空(doc envelope 沒這欄,degrade 不 crash)
        assert_eq!(r.schema_version, 1); // 預設
    }

    #[test]
    fn default_policy_covers_all_ten_classes() {
        // 政策表必須涵蓋 doc Permission classes 表的 10 個 class(不含 Unknown)。
        assert_eq!(default_policy().rules.len(), 10);
    }
}
