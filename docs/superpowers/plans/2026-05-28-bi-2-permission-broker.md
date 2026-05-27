# BI-2 Permission Broker (minimal) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mori Desktop 有一個最小但完整的 **Permission Broker**:body part / external agent 提出 `PermissionRequest`(帶 risk class)→ broker 依預設政策回 `allow / deny / ask` → 每筆決策寫進 append-only audit log。**刻意排在任何 `audio.capture` 部件之前**,讓 permission envelope 契約先在無真實高風險消費者的安全狀態下成熟。

**Architecture:** 在 `mori_core::body` 下新增 `permission`(envelope 型別 + `RiskClass` + `Decision` + `PolicyTable` + 純函式 `evaluate`)與 `permission_audit`(`PermissionAuditEntry` + append/tail JSONL + `broker_decide` 組合器)。所有政策與 audit 邏輯都在 mori-core、用 tempdir 純測。`mori-tauri` 只當薄 shim:`permission_broker.rs` 提供 `~/.mori/permission-audit.jsonl` 路徑 + `decide()`,三個唯讀/評估 command,前端新增唯讀 `PermissionsTab`(政策表 + audit log + 一組 **demo 按鈕**送假請求穿過 broker 以示範三路徑)。

**Tech Stack:** Rust(mori-core serde envelope + tempdir-testable 政策/audit;mori-tauri command + chrono timestamp)、React/TS(新 sidebar tab,沿用 BodyTab 風格 + i18n)。

**依賴**:BI-1(`mori_core::body` 模組、`crate::mori_dir()` helper、sidebar tab pattern 都已存在)。等 PR #130(BI-1)合進 main 後,從乾淨 main 切 `feat/bi-2-permission-broker`。

**Spec sources:**
- `docs/body-interface-backlog.md`(BI-2 row L94 + 完成判準 L105 + §1 範圍紀律 + §5 決定 B1)
- `docs/mori-body-interface.md`(§Permission Broker L566-647:permission classes / tool request envelope / broker 回覆;§Versioning L953-971「對高風險未知 permission 預設 deny」;§Testing L972-989「permission request tests」)

---

## 設計決策(已選預設,review 時可推翻)

| # | 決策 | 選的(預設)| 備選 / 備註 |
|---|---|---|---|
| P1 | **「ask」在 BI-2 = 決策結果,不做互動式解析 UI** | broker 依政策把請求**分類**成 allow/deny/ask,回給 caller + 寫 audit。`ask` 代表「需要人決定」,但**把 ask 變成 allow/deny 的 modal / pending store 不在 BI-2 做** | 理由:backlog §1 明令「在 0 個實作時不要把通用層蓋滿」。BI-2 沒有任何真實 requester(Agent Plus=BI-3、Meeting Recorder=BI-5)。互動式人類解析等**真的有 requester** 時再做。**這是本 plan 最關鍵的縮放決策,review 時最該挑戰它** |
| P2 | **決策只看 `risk` class** | `evaluate` 是 `risk → Decision` 的純查表;`scope / session_id / reason` 只寫進 audit,不影響 BI-2 的決策 | scoped allow / per-project policy / 信任記憶 = 之後。doc 的 `read.project = "ask / scoped allow"`,BI-2 取保守的 `ask` |
| P3 | **未知 / 未列 risk → Deny** | `RiskClass` 用 `#[serde(other)]` 把未知字串吃成 `Unknown`;policy table 不含 `Unknown` → 查不到 → `Deny` | 對齊 §Versioning L970「對高風險未知 permission 預設 deny」 |
| P4 | **政策表 = 程式內 hardcoded 預設** | `default_policy()` 回傳 10 個已知 class 的預設決策(對齊 doc 的 Permission classes 表)| user-editable policy file / 每 project override = 之後(YAGNI:BI-2 沒有 UI 要編輯它)|
| P5 | **audit log = append-only JSONL** | `~/.mori/permission-audit.jsonl`(與 `~/.mori/body-parts/` 同根,用 `crate::mori_dir()`),每筆一行 JSON | 跟 `~/.mori/logs/*.jsonl`(LogsTab)同風格;read 用 tail(讀最後 N 筆) |
| P6 | **audit 失敗 → fail-safe**:決策無法被記錄就**不放行** | `broker_decide` 回 `Result`;audit 寫入失敗 → `Err`,command 把 `Err` 回給前端,呼叫端應視同 deny | 安全敏感:一個記不下來的授權不算數。disk-full 等罕見,可接受擋下 |
| P7 | **UI = 新唯讀 top-level tab「權限 / Permissions」** | 沿用 BI-1 BodyTab pattern(最低風險);顯示政策表 + audit log + demo 按鈕 | 備選:塞進 ConfigTab Settings group(doc §940 把 Permissions 列為 settings group)。否決理由:ConfigTab 已 113KB;且 Mori Desktop §18 把 Permission broker 列為一級職責,值得獨立 surface。**review 可推翻** |
| P8 | **`lease` 欄位保留不實作** | `BrokerResponse.lease: Option<Lease>` 對齊 doc 的 broker 回覆 envelope,但 BI-2 永遠回 `None` | 對齊 backlog 判準 L41「沒有 ≥1 真實 desktop body part 會用到的欄位就不實作,最多 schema 保留」。lease 沒有消費者 |

---

## File Structure

| 檔案 | 責任 | 動作 |
|---|---|---|
| `crates/mori-core/src/body/permission.rs` | `RiskClass` / `Decision` / `PermissionRequest` / `BrokerResponse` / `Lease`(reserved)/ `PolicyRule` / `PolicyTable` + `default_policy()` + 純函式 `evaluate()` + 測試 | Create |
| `crates/mori-core/src/body/permission_audit.rs` | `PermissionAuditEntry` + `append_audit(path,&entry)` + `read_audit_tail(path,limit)` + `broker_decide(req,policy,audit_path,now)` 組合器 + 測試 | Create |
| `crates/mori-core/src/body/mod.rs` | 加 `pub mod permission; pub mod permission_audit;` + re-export | Modify |
| `crates/mori-tauri/src/permission_broker.rs` | `audit_path()` + `decide(req)` / `decide_at(req,path)` + 測試 | Create |
| `crates/mori-tauri/src/main.rs` | `permission_decide` / `permission_audit_list` / `permission_policy_list` 三 command + `mod permission_broker;` + 註冊 | Modify |
| `src/tabs/PermissionsTab.tsx` | 唯讀 UI:政策表 + audit log + demo 按鈕(送假請求穿過 broker)| Create |
| `src/shellTabs.ts` | `SHELL_TAB_IDS` 加 `"permissions"` | Modify |
| `src/MainShell.tsx` | import + TABS entry + render switch | Modify |
| `src/icons.tsx` | `IconPermissions`(盾牌)| Modify |
| `src/i18n/locales/zh-TW.json` / `en.json` | `sidebar.permissions(_sub)` + `permissions_tab.*` | Modify |
| `docs/body-interface-backlog.md` | BI-2 進度標記 | Modify |

---

## Task 1: 核心 — permission envelope + 政策 + `evaluate`

**Files:**
- Create: `crates/mori-core/src/body/permission.rs`
- Modify: `crates/mori-core/src/body/mod.rs`

- [ ] **Step 1: 在 `permission.rs` 寫 failing test**

```rust
//! Permission Broker — body part / external agent 提出 tool request,broker 依
//! risk class 給 allow/deny/ask。見 docs/mori-body-interface.md §Permission Broker。
//! BI-2 最小版:決策只看 risk class(純查表);未知 risk → deny(§Versioning)。

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
```

- [ ] **Step 2: 跑測試確認 fail**

Run: `cargo test -p mori-core --lib body::permission::`
Expected: 編譯失敗(型別 / 函式未定義)。

- [ ] **Step 3: 在 test mod 上方寫實作**

```rust
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub risk: RiskClass,
    pub default: Decision,
}

/// 政策表 — risk class → 預設決策。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyTable {
    pub rules: Vec<PolicyRule>,
}

/// BI-2 hardcoded 預設政策,對齊 docs/mori-body-interface.md §Permission classes 表。
/// `read.project` 的 doc 是「ask / scoped allow」→ 取保守的 ask。
pub fn default_policy() -> PolicyTable {
    use Decision::*;
    use RiskClass::*;
    let rule = |risk, default| PolicyRule { risk, default };
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
        .map(|r| r.default)
        .unwrap_or(Decision::Deny)
}
```

- [ ] **Step 4: 在 `body/mod.rs` 加 module + re-export**

在現有 `pub mod registry;` 後加:
```rust
pub mod permission;
```
在現有 `pub use registry::{...};` 後加:
```rust
pub use permission::{
    default_policy, evaluate, BrokerResponse, Decision, Lease, PermissionRequest, PolicyRule,
    PolicyTable, RiskClass, SUPPORTED_PERMISSION_SCHEMA,
};
```

- [ ] **Step 5: 跑測試確認 pass**

Run: `cargo test -p mori-core --lib body::permission::`
Expected: 6 test PASS。
Run: `cargo test -p mori-core --lib`(無回歸)。

- [ ] **Step 6: Commit**

```bash
git add crates/mori-core/src/body/permission.rs crates/mori-core/src/body/mod.rs
git commit -m "feat(bi-2): permission envelope + risk policy + evaluate (mori-core)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: 核心 — audit log + `broker_decide` 組合器

**Files:**
- Create: `crates/mori-core/src/body/permission_audit.rs`
- Modify: `crates/mori-core/src/body/mod.rs`(加 `pub mod permission_audit;` + re-export)

- [ ] **Step 1: 在 `permission_audit.rs` 寫 failing test**

```rust
//! Permission audit log — 每筆 broker 決策 append 一行 JSON 到 `~/.mori/permission-audit.jsonl`。
//! `broker_decide` = evaluate + 寫 audit 的組合器;audit 寫不下去 → Err(fail-safe:
//! 記不下來的授權不算數)。讀用 tail(最後 N 筆,新到舊)。

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
```

- [ ] **Step 2: 跑測試確認 fail**

Run: `cargo test -p mori-core --lib body::permission_audit::`
Expected: 編譯失敗。

- [ ] **Step 3: 寫實作**

```rust
use crate::body::permission::{
    evaluate, BrokerResponse, Decision, PermissionRequest, PolicyTable, RiskClass,
};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

/// audit log 的一筆紀錄(request 快照 + 決策 + 時間)。
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    if let Some(parent) = path.parent() {
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
```

- [ ] **Step 4: 在 `body/mod.rs` 加 module + re-export**

在 `pub mod permission;` 後加:
```rust
pub mod permission_audit;
```
在 permission 的 re-export 後加:
```rust
pub use permission_audit::{
    append_audit, broker_decide, read_audit_tail, PermissionAuditEntry,
};
```

- [ ] **Step 5: 跑測試確認 pass**

Run: `cargo test -p mori-core --lib body::permission_audit::`
Expected: 4 test PASS。
Run: `cargo test -p mori-core --lib`(無回歸)。

- [ ] **Step 6: Commit**

```bash
git add crates/mori-core/src/body/permission_audit.rs crates/mori-core/src/body/mod.rs
git commit -m "feat(bi-2): permission audit log + broker_decide composer (mori-core)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: mori-tauri — broker shim(audit 路徑 + decide)

**Files:**
- Create: `crates/mori-tauri/src/permission_broker.rs`

> 前置:`grep -n "fn mori_dir" crates/mori-tauri/src/*.rs` 確認 `crate::mori_dir()` 的取法(BI-1 `body_registry.rs` 已用 `crate::mori_dir().join("body-parts")`,這裡 `audit_path()` 用 `crate::mori_dir().join("permission-audit.jsonl")`,同一個 helper、同一個 `~/.mori` 根)。

- [ ] **Step 1: 寫 `permission_broker.rs`(含 test)**

```rust
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
    let now = chrono::Local::now().to_rfc3339();
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
```

- [ ] **Step 2: 宣告 module**

`main.rs` 頂部 module 區(`mod body_registry;` 旁,維持字母序)加:
```rust
mod permission_broker;
```

- [ ] **Step 3: 跑測試 + 編譯**

Run: `cargo test -p mori-tauri --bin mori-tauri permission_broker`
Expected: 1 test PASS。
Run: `cargo check -p mori-tauri`
Expected: 乾淨。

- [ ] **Step 4: Commit**

```bash
git add crates/mori-tauri/src/permission_broker.rs crates/mori-tauri/src/main.rs
git commit -m "feat(bi-2): permission_broker shim (audit path + decide)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: mori-tauri — 三個 Tauri command

**Files:**
- Modify: `crates/mori-tauri/src/main.rs`(command + 註冊)

> 前置:command 加在 `fn body_registry_list()`(main.rs:2216 附近)同區;註冊加在 `tauri::generate_handler![...]`(main.rs:6244 起,`body_registry_list,` 在 6321 附近)同清單。

- [ ] **Step 1: 加三個 command**

加在 `body_registry_list` 旁:
```rust
/// BI-2:評估一筆 permission request → allow/deny/ask,並寫 audit log。
/// audit 寫不下去 → Err(fail-safe:記不下來的授權不算數,呼叫端應視同 deny)。
#[tauri::command]
fn permission_decide(
    request: mori_core::body::PermissionRequest,
) -> Result<mori_core::body::BrokerResponse, String> {
    crate::permission_broker::decide(&request)
}

/// BI-2:讀 audit log 最後 `limit` 筆(新到舊),唯讀。
#[tauri::command]
fn permission_audit_list(limit: usize) -> Vec<mori_core::body::PermissionAuditEntry> {
    mori_core::body::read_audit_tail(&crate::permission_broker::audit_path(), limit)
}

/// BI-2:回傳目前的預設政策表(risk class → 預設決策),供 UI 顯示。唯讀。
#[tauri::command]
fn permission_policy_list() -> Vec<mori_core::body::PolicyRule> {
    mori_core::body::default_policy().rules
}
```

- [ ] **Step 2: 註冊**

在 `tauri::generate_handler![...]` 內 `body_registry_list,` 旁加三行:
```rust
            permission_decide,
            permission_audit_list,
            permission_policy_list,
```

- [ ] **Step 3: 編譯確認**

Run: `cargo check -p mori-tauri`
Expected: 乾淨。

- [ ] **Step 4: Commit**

```bash
git add crates/mori-tauri/src/main.rs
git commit -m "feat(bi-2): permission_decide / audit_list / policy_list commands

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: 前端唯讀 `PermissionsTab`(政策表 + audit log + demo)

**Files:**
- Create: `src/tabs/PermissionsTab.tsx`
- Modify: `src/shellTabs.ts`、`src/MainShell.tsx`、`src/icons.tsx`、`src/i18n/locales/zh-TW.json`、`src/i18n/locales/en.json`

> 走 integration + manual verify(repo UI 慣例)。沿用 `src/tabs/BodyTab.tsx` 的 inline-style + i18n + StatusBadge 風格。

- [ ] **Step 1: 寫 `PermissionsTab.tsx`**

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

type Decision = "allow" | "deny" | "ask";

interface PolicyRule {
  risk: string;
  default: Decision;
}
interface AuditEntry {
  timestamp: string;
  request_id: string;
  session_id?: string | null;
  source: string;
  tool: string;
  risk: string;
  decision: Decision;
  reason?: string | null;
}
interface BrokerResponse {
  request_id: string;
  decision: Decision;
}

// demo:三條 canned 請求,送過 broker 示範 allow/ask/deny 三路徑。
const DEMO_REQUESTS: { label: string; risk: string }[] = [
  { label: "read.public", risk: "read.public" },
  { label: "read.project", risk: "read.project" },
  { label: "exec.destructive", risk: "exec.destructive" },
];

export default function PermissionsTab() {
  const { t } = useTranslation();
  const [policy, setPolicy] = useState<PolicyRule[]>([]);
  const [audit, setAudit] = useState<AuditEntry[]>([]);
  const [lastDecision, setLastDecision] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const refresh = async () => {
    try {
      setPolicy(await invoke<PolicyRule[]>("permission_policy_list"));
      setAudit(await invoke<AuditEntry[]>("permission_audit_list", { limit: 50 }));
      setErr(null);
    } catch (e: any) {
      setErr(String(e));
    }
  };
  useEffect(() => { refresh(); }, []);

  const fireDemo = async (risk: string) => {
    try {
      const resp = await invoke<BrokerResponse>("permission_decide", {
        request: {
          schema_version: 1,
          request_id: `demo_${Date.now()}`,
          source: "permissions.tab.demo",
          tool: "demo.tool",
          risk,
          reason: "PermissionsTab demo",
        },
      });
      setLastDecision(`${risk} → ${resp.decision}`);
      await refresh();
    } catch (e: any) {
      setErr(String(e));
    }
  };

  return (
    <div style={{ padding: 16 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 12 }}>
        <h2 style={{ margin: 0 }}>{t("permissions_tab.title")}</h2>
        <button className="mori-btn small ghost" onClick={refresh}>{t("permissions_tab.refresh")}</button>
      </div>
      <p style={{ opacity: 0.7, fontSize: 12 }}>
        {t("permissions_tab.hint")} (<code>~/.mori/permission-audit.jsonl</code>)
      </p>
      {err && <div style={{ color: "rgba(255,160,160,.95)", fontSize: 12 }}>❌ {err}</div>}

      <h3 style={{ marginBottom: 6 }}>{t("permissions_tab.policy_title")}</h3>
      <div style={{ display: "flex", flexDirection: "column", gap: 4, marginBottom: 16 }}>
        {policy.map((r) => (
          <div key={r.risk} style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13 }}>
            <code style={{ minWidth: 150 }}>{r.risk}</code>
            <DecisionBadge decision={r.default} />
          </div>
        ))}
      </div>

      <h3 style={{ marginBottom: 6 }}>{t("permissions_tab.demo_title")}</h3>
      <p style={{ opacity: 0.6, fontSize: 11 }}>{t("permissions_tab.demo_hint")}</p>
      <div style={{ display: "flex", gap: 8, marginBottom: 6, flexWrap: "wrap" }}>
        {DEMO_REQUESTS.map((d) => (
          <button key={d.risk} className="mori-btn small" onClick={() => fireDemo(d.risk)}>
            {d.label}
          </button>
        ))}
      </div>
      {lastDecision && <div style={{ fontSize: 12, opacity: 0.8, marginBottom: 16 }}>{lastDecision}</div>}

      <h3 style={{ marginBottom: 6 }}>{t("permissions_tab.audit_title")}</h3>
      {audit.length === 0 && !err && <div style={{ opacity: 0.6 }}>{t("permissions_tab.audit_empty")}</div>}
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        {audit.map((a, i) => (
          <div key={`${a.request_id}_${i}`} style={{ border: "1px solid var(--c-border)", borderRadius: 8, padding: 10 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <DecisionBadge decision={a.decision} />
              <code style={{ fontSize: 12 }}>{a.risk}</code>
              <span style={{ fontSize: 12, opacity: 0.7 }}>{a.tool}</span>
            </div>
            <div style={{ fontSize: 11, opacity: 0.6 }}>{a.source} · {a.timestamp}</div>
            {a.reason && <div style={{ fontSize: 12, opacity: 0.8 }}>{a.reason}</div>}
          </div>
        ))}
      </div>
    </div>
  );
}

function DecisionBadge({ decision }: { decision: string }) {
  const { t } = useTranslation();
  const map: Record<string, { label: string; c: string }> = {
    allow: { label: t("permissions_tab.decision_allow"), c: "rgba(140,220,160,.9)" },
    ask: { label: t("permissions_tab.decision_ask"), c: "rgba(230,200,120,.9)" },
    deny: { label: t("permissions_tab.decision_deny"), c: "rgba(255,160,160,.95)" },
  };
  const s = map[decision] ?? { label: decision, c: "var(--c-text-muted)" };
  return <span style={{ fontSize: 11, color: s.c, fontWeight: 600 }}>{s.label}</span>;
}
```

- [ ] **Step 2: `shellTabs.ts` 加 tab id**

在 `SHELL_TAB_IDS` 陣列 `"body",` 後加一行:
```ts
  "permissions",
```

- [ ] **Step 3: `icons.tsx` 加 `IconPermissions`**

接在 `IconBody` 後加(沿用同檔 `base` spread 風格 —— 盾牌輪廓 + 中央勾):
```tsx
// 🛡 Permissions — 盾牌(權限 broker / audit)。
export function IconPermissions(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M12 3 l7 3 v5 c0 4.5 -3 7.5 -7 9 c-4 -1.5 -7 -4.5 -7 -9 V6 z" />
      <path d="M9 12 l2 2 l4 -4" />
    </svg>
  );
}
```

- [ ] **Step 4: `MainShell.tsx` 接線(三處)**

(a) import(跟 `import BodyTab from "./tabs/BodyTab";` 同區):
```tsx
import PermissionsTab from "./tabs/PermissionsTab";
```
(b) `IconPermissions` 加進 `icons.tsx` 的既有 import 清單(找 `IconBody` 所在的 `import { ... } from "./icons"`,把 `IconPermissions` 加進去)。
(c) `TABS` 陣列 `{ id: "body", ... }` 後加一行:
```tsx
  { id: "permissions", Icon: IconPermissions, key: "permissions" },
```
(d) render switch `{tab === "body" && <BodyTab />}` 後加一行:
```tsx
        {tab === "permissions" && <PermissionsTab />}
```

- [ ] **Step 5: i18n — `zh-TW.json`**

(a) sidebar 區 `"body_sub": "body part 清單",` 後加:
```json
    "permissions": "權限",
    "permissions_sub": "Permission broker / audit",
```
(b) `body_tab` block 結束的 `}` 後(模仿既有 `,"body_tab"` 逗號前綴風格)加:
```json
  ,"permissions_tab": {
    "title": "權限 broker",
    "refresh": "重新整理",
    "hint": "body part / agent 的工具請求經 broker 依風險分級裁決,每筆寫進 audit log(唯讀)",
    "policy_title": "預設政策",
    "demo_title": "示範 / 測試",
    "demo_hint": "送一條假請求穿過 broker(示範 allow / ask / deny 三路徑;會寫進下方 audit log)。",
    "audit_title": "稽核紀錄(audit log)",
    "audit_empty": "還沒有任何裁決紀錄。",
    "decision_allow": "✓ 允許",
    "decision_ask": "？ 詢問",
    "decision_deny": "✗ 拒絕"
  }
```

- [ ] **Step 6: i18n — `en.json`**

(a) sidebar 區 `"body_sub": "body parts registry",` 後加:
```json
    "permissions": "Permissions",
    "permissions_sub": "Permission broker / audit",
```
(b) `body_tab` block 後(同 `,"..."` 逗號前綴風格)加:
```json
  ,"permissions_tab": {
    "title": "Permission Broker",
    "refresh": "Refresh",
    "hint": "Tool requests from body parts / agents are brokered by risk class; every decision is written to an audit log (read-only)",
    "policy_title": "Default policy",
    "demo_title": "Demo / test",
    "demo_hint": "Send a fake request through the broker (demonstrates allow / ask / deny; it is written to the audit log below).",
    "audit_title": "Audit log",
    "audit_empty": "No decisions yet.",
    "decision_allow": "✓ allow",
    "decision_ask": "？ ask",
    "decision_deny": "✗ deny"
  }
```

- [ ] **Step 7: build + 既有測試不破**

Run: `npm run build && npm test`
Expected: build 成功(TS 無型別錯);既有 vitest 全綠(含 `shellTabs.test.ts` —— 新增的 `"permissions"` 是合法 visible tab,不影響既有斷言)。

- [ ] **Step 8: Manual verify(BI-2 e2e — 對應 backlog 完成判準)**

```bash
npm run tauri dev
```
1. 點側欄「權限」tab → 看到「預設政策」表(10 條,`exec.destructive`/`write.private` 顯示 ✗ 拒絕,`read.public`/`exec.safe` 顯示 ✓ 允許,其餘 ？ 詢問)。
2. 按 demo 的 **`exec.destructive`** 按鈕 → 上方顯示 `exec.destructive → deny`,且下方 audit log 立刻出現一筆 ✗ 拒絕紀錄(**= 完成判準「一條 fake 高風險請求被 broker 攔下並寫 audit log」**)。
3. 按 **`read.public`** → `allow`;按 **`read.project`** → `ask`(三路徑齊)。
4. `cat ~/.mori/permission-audit.jsonl` → 每行一筆 JSON,含 timestamp / risk / decision。
5. 重啟 app → audit log 仍在(append-only 持久)。

- [ ] **Step 9: Commit**

```bash
git add src/tabs/PermissionsTab.tsx src/shellTabs.ts src/MainShell.tsx src/icons.tsx \
  src/i18n/locales/zh-TW.json src/i18n/locales/en.json
git commit -m "feat(bi-2): read-only Permissions tab (policy + audit + demo)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: 文件進度

**Files:**
- Modify: `docs/body-interface-backlog.md`

- [ ] **Step 1: 標 BI-2 done**

把 §4「各 stage 的完成判準」的 BI-2 那條改成 done(對齊 BI-0/BI-1 的寫法):
```markdown
- **BI-2 done** ✅(2026-05-28,branch `feat/bi-2-permission-broker`)= permission envelope(`PermissionRequest`/`BrokerResponse`)+ 10-class 預設政策 + 純函式 `evaluate`(未知 risk → deny)+ append-only audit log(`~/.mori/permission-audit.jsonl`)+ 唯讀 Permissions tab(政策表 / audit / demo 三路徑)。一條 fake `exec.destructive` 被 broker 攔下並寫 audit。**未做**(刻意):互動式 ask 解析 UI、user-editable policy、lease(等真實 requester BI-3/BI-5)。
```

- [ ] **Step 2: Commit**

```bash
git add docs/body-interface-backlog.md
git commit -m "docs(bi-2): mark Permission Broker done

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage**(對 backlog BI-2 row + 判準 + body-interface §Permission Broker / §Versioning / §Testing):
- permission request schema → `PermissionRequest`(對齊 doc §Tool request envelope)Task 1 ✓
- broker 回覆 schema → `BrokerResponse`(decision + reserved lease,對齊 doc §broker 回覆)Task 1 ✓
- allow / deny / ask 三路徑 → `evaluate` + `default_policy`,三路徑各有 test(Task 1)+ UI demo 三按鈕(Task 5)✓
- audit log → `append_audit` / `read_audit_tail` / `broker_decide`,tempdir 測 + e2e 寫檔 Task 2/5 ✓
- 「一條 fake 高風險請求被 broker 攔下並寫 audit」→ Task 2 test `broker_decide_denies_destructive_and_writes_audit` + Task 5 manual verify #2 ✓
- 對高風險未知 permission 預設 deny(§Versioning)→ `RiskClass::Unknown` + `evaluate` unwrap_or(Deny),test `unknown_risk_defaults_to_deny` ✓
- permission request tests(§Testing)→ Task 1/2 單元測試 ✓
- 排在 audio.capture 之前(決定 B1)→ 本 plan 不引入任何 audio 部件;`audio.capture` 只在政策表內當一條 `ask` 規則存在 ✓

**2. Placeholder scan:** 無 TBD。唯一「實作時確認」是 Task 3/4 的 `crate::mori_dir()` 與 command 註冊位置 —— 都標了 grep 前置 + 引用了 BI-1 `body_registry.rs` 的既有同款用法,非空白。

**3. Type consistency:**
- `Decision`(allow/deny/ask,serde lowercase)Rust ↔ TS `Decision` 字面值三處一致(evaluate → audit → BrokerResponse → command → TS badge)✓
- `RiskClass` serde 字串(`read.public` 等 dotted rename + `Unknown` via `#[serde(other)]`)↔ TS 用 string,policy/audit 顯示原字串 ✓
- `PermissionRequest` 欄位(schema_version/request_id/session_id/source/tool/risk/reason/scope)Rust serde ↔ TS demo 送出物件對齊;Tauri command 參數名 `request`(Rust `request:`)↔ 前端 `invoke("permission_decide", { request: {...} })` 一致 ✓
- command 名三處一致:`permission_decide` / `permission_audit_list`(參數 `limit` ↔ 前端 `{ limit: 50 }`)/ `permission_policy_list`(定義 → 註冊 → 前端 invoke)✓
- `PolicyRule { risk, default }` Rust ↔ TS interface 同名(`default` 是 TS 合法 property name,非保留字衝突)✓
- `PermissionAuditEntry` 欄位 Rust ↔ TS `AuditEntry` interface 同名 ✓

**4. 範圍紀律複查(backlog §1):** 只引入 envelope 型別 + 純查表政策 + audit log + 唯讀 tab + demo 按鈕。**無** 互動式 ask 解析(P1)、**無** user-editable policy(P4)、**無** lease 計算(P8)、**無** sandbox / shell shim / external-agent adapter(那是真有 requester 時的 BI-3+)、**無** 任何 audio 部件。`lease` / 未知 transport 只在 schema 保留。✓

---

## 全套驗證(收工前)
```bash
bash scripts/verify.sh
```
預設含 `npm run build`、`cargo test -p mori-core --lib`、`cargo check --workspace --all-targets`。額外手跑一次完整 broker 測試:
```bash
cargo test -p mori-core --lib body::permission
cargo test -p mori-tauri --bin mori-tauri permission_broker
```

**Branch:** `feat/bi-2-permission-broker`(等 #130 / BI-1 合進 main 後,從乾淨 main 切)
**Plan saved:** `docs/superpowers/plans/2026-05-28-bi-2-permission-broker.md`
