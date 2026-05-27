# BI-1 Body Registry (read-only) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mori Desktop 能掃描本機 body part manifest、解析驗證、用**唯讀** UI 列出每個 body part 的身分 / kind / capabilities / 狀態;並把 MoriPack Studio 回填成第一個 registered manifest。**完全不執行任何 write / exec / 高風險操作。**

**Architecture:** 在 BI-0 已建的 `mori_core::body` 模組下新增 `manifest`(semantic 型別 `BodyManifest` + 解析/驗證)與 `registry`(`scan_body_parts(base)` 掃 `~/.mori/body-parts/`)。`mori-tauri` 啟動時把 bundled 的 MoriPack Studio manifest 寫進 `~/.mori/body-parts/`(類比 character pack 的 `ensure_default`),加一個唯讀 `body_registry_list` command,前端加一個唯讀 `BodyTab`。

**Tech Stack:** Rust(mori-core serde 型別 + tempdir-testable 掃描;mori-tauri command + bundled manifest)、React/TS(新 sidebar tab)。

**依賴**:BI-0(`mori_core::body` 模組已存在;PR #129 合進 main 後再從乾淨 main 切 `feat/bi-1-body-registry`)。

**Spec sources:** `docs/body-interface-backlog.md`(BI-1 row + 完成判準)、`docs/mori-body-interface.md`(§Body Part Manifest L104-167、§Discovery L406-416、§Versioning L954-971)。

---

## 設計決策(已選預設,review 時可推翻)

| # | 決策 | 選的(預設)| 備選 / 備註 |
|---|---|---|---|
| M1 | **manifest v1 欄位範圍** | 只放 `schema_version / id / name / kind / description / entrypoints / interfaces / capabilities / permissions / data_policy`(對齊 backlog D1)| robot/ROS2/zenoh **不放專屬欄位**;未知 transport 用 `#[serde(other)]` 吃進 `Transport::Other`(schema 不禁止未來 binding,但 BI-1 不處理)|
| M2 | **「health」在 BI-1 的意義** | **靜態有效性**:manifest 能 parse + schema_version 支援 → `Valid`,否則 `UnsupportedSchema` / `ParseError`。**不做 live HTTP /health 探測** | live `/health` 探測等到有 body part service 在跑(BI-3 Agent Plus)再做;BI-1 沒有任何 service 在跑,且要守「不執行高風險操作」|
| M3 | **MoriPack 怎麼成為第一個 registered body part** | bundle 一份 `mori.moripack-studio` manifest,啟動時寫到 `~/.mori/body-parts/mori.moripack-studio/manifest.json`(類比 `character_pack::ensure_default`)| 之後第三方 body part 也是丟 manifest 進這資料夾被掃到 |
| M4 | **UI 形態** | 新 sidebar tab **`BodyTab`(身體)**,唯讀清單:name / kind / status badge / capabilities / interfaces / 來源路徑 | 不放任何「啟動/停止/授權」按鈕(那是 BI-2/BI-3)|
| M5 | **掃描位置** | `~/.mori/body-parts/<id>/manifest.json` 與 `~/.mori/body-parts/*.json` | repo-local `<repo>/.mori-body/manifest.json`(dev 用)**BI-1 先不掃**,等有需求 |

---

## File Structure

| 檔案 | 責任 | 動作 |
|---|---|---|
| `crates/mori-core/src/body/manifest.rs` | `BodyManifest` 型別 + enums + `parse_manifest` + `manifest_status` + 測試 | Create |
| `crates/mori-core/src/body/registry.rs` | `DiscoveredBodyPart` + `scan_body_parts(base: &Path)` + 測試 | Create |
| `crates/mori-core/src/body/mod.rs` | 加 `pub mod manifest; pub mod registry;` + re-export | Modify |
| `crates/mori-tauri/src/body_registry.rs` | `body_parts_dir()` + bundled MoriPack Studio manifest + `ensure_bundled_body_parts()` + 測試 | Create |
| `crates/mori-tauri/src/main.rs` | `body_registry_list` command + 註冊 + 啟動時呼叫 `ensure_bundled_body_parts()` | Modify |
| `src/tabs/BodyTab.tsx` | 唯讀 body part 清單 UI | Create |
| `src/`(sidebar 註冊處)| 加「身體 / Body」tab 入口 | Modify |
| `docs/body-interface-backlog.md` | BI-1 進度標記 | Modify |

---

## Task 1: `BodyManifest` 型別 + 解析/驗證

**Files:**
- Create: `crates/mori-core/src/body/manifest.rs`
- Modify: `crates/mori-core/src/body/mod.rs`

- [ ] **Step 1: 在 `body/manifest.rs` 寫 failing test**

```rust
//! Body Part Manifest — body part 對 Mori Desktop 自我描述的 semantic 契約。
//! 見 docs/mori-body-interface.md §Body Part Manifest。BI-1 v1 欄位刻意最小;
//! 未知 transport / 未知欄位都「能讀就降級讀、不 crash」(§Versioning)。

#[cfg(test)]
mod tests {
    use super::*;

    const MORIPACK_JSON: &str = r#"{
        "schema_version": 1,
        "id": "mori.moripack-studio",
        "name": "MoriPack Studio",
        "kind": "standalone_app",
        "capabilities": ["character_pack.edit", "character_pack.export"],
        "entrypoints": { "web": "https://mori-sprite-studio.vercel.app/" },
        "permissions": ["filesystem.read.character_pack"],
        "data_policy": { "owns_raw_data": false, "default_ingestion": "off" }
    }"#;

    #[test]
    fn parses_valid_moripack_manifest() {
        let m = parse_manifest(MORIPACK_JSON).expect("should parse");
        assert_eq!(m.id, "mori.moripack-studio");
        assert_eq!(m.kind, BodyKind::StandaloneApp);
        assert!(m.capabilities.contains(&"character_pack.export".to_string()));
        assert_eq!(m.entrypoints.web.as_deref(), Some("https://mori-sprite-studio.vercel.app/"));
        assert!(!m.data_policy.owns_raw_data);
        assert_eq!(manifest_status(&m), ManifestStatus::Valid);
    }

    #[test]
    fn unknown_transport_degrades_to_other_not_error() {
        // §Versioning:不懂的 transport 記錄但不 crash。
        let json = r#"{"schema_version":1,"id":"x","name":"X","kind":"local_service",
            "interfaces":[{"name":"events","transport":"zenoh","url":"z"}]}"#;
        let m = parse_manifest(json).expect("zenoh interface should still parse");
        assert_eq!(m.interfaces[0].transport, Transport::Other);
    }

    #[test]
    fn future_schema_version_is_unsupported_not_parse_error() {
        let json = r#"{"schema_version":99,"id":"x","name":"X","kind":"cli"}"#;
        let m = parse_manifest(json).expect("should still parse structurally");
        assert_eq!(manifest_status(&m), ManifestStatus::UnsupportedSchema(99));
    }

    #[test]
    fn missing_required_field_is_parse_error() {
        // 缺 id → serde 失敗 → Err
        let json = r#"{"schema_version":1,"name":"X","kind":"cli"}"#;
        assert!(parse_manifest(json).is_err());
    }
}
```

- [ ] **Step 2: 跑測試確認 fail**

Run: `cargo test -p mori-core --lib body::manifest`
Expected: 編譯失敗(型別 / 函式未定義)。

- [ ] **Step 3: 在 test mod 上方寫實作**

```rust
use serde::{Deserialize, Serialize};

/// 目前支援的 manifest schema major。
pub const SUPPORTED_MANIFEST_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub kind: BodyKind,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub entrypoints: Entrypoints,
    #[serde(default)]
    pub interfaces: Vec<Interface>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub data_policy: DataPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyKind {
    StandaloneApp,
    LocalService,
    Cli,
    Crate,
    Plugin,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entrypoints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_api: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interface {
    pub name: String,
    pub transport: Transport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// BI-1 只實作 http/sse/cli;其餘(zenoh/ros2/dds…)吃進 `Other`,
/// 不報錯也不處理 —— schema 不禁止未來 binding,但 BI-1 不動它們。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Http,
    Sse,
    Cli,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataPolicy {
    #[serde(default)]
    pub owns_raw_data: bool,
    #[serde(default = "default_ingestion")]
    pub default_ingestion: String,
}

impl Default for DataPolicy {
    fn default() -> Self {
        Self { owns_raw_data: false, default_ingestion: default_ingestion() }
    }
}

fn default_ingestion() -> String {
    "off".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "detail")]
pub enum ManifestStatus {
    Valid,
    UnsupportedSchema(u32),
}

/// 結構解析(serde)。缺 required 欄位 → Err。
pub fn parse_manifest(json: &str) -> Result<BodyManifest, String> {
    serde_json::from_str(json).map_err(|e| e.to_string())
}

/// 語意有效性:schema_version 是否支援。
pub fn manifest_status(m: &BodyManifest) -> ManifestStatus {
    if m.schema_version == SUPPORTED_MANIFEST_SCHEMA {
        ManifestStatus::Valid
    } else {
        ManifestStatus::UnsupportedSchema(m.schema_version)
    }
}
```

- [ ] **Step 4: 在 `body/mod.rs` 加 module + re-export**

```rust
pub mod manifest;
```
並把 re-export 行補上(跟現有 artifact re-export 同風格):
```rust
pub use manifest::{
    manifest_status, parse_manifest, BodyKind, BodyManifest, DataPolicy, Entrypoints, Interface,
    ManifestStatus, Transport, SUPPORTED_MANIFEST_SCHEMA,
};
```

- [ ] **Step 5: 跑測試確認 pass**

Run: `cargo test -p mori-core --lib body::manifest`
Expected: 4 test PASS。
Run: `cargo test -p mori-core --lib`(無回歸)。

- [ ] **Step 6: Commit**

```bash
git add crates/mori-core/src/body/manifest.rs crates/mori-core/src/body/mod.rs
git commit -m "feat(bi-1): BodyManifest type + parse/validate in mori-core

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Body registry 掃描器

**Files:**
- Create: `crates/mori-core/src/body/registry.rs`
- Modify: `crates/mori-core/src/body/mod.rs`(加 `pub mod registry;` + re-export)

- [ ] **Step 1: 在 `registry.rs` 寫 failing test**

```rust
//! Body Registry — 掃描本機 body part manifest 目錄,回報每個 body part 的
//! 身分與狀態。**唯讀**:只讀檔、不啟動、不執行任何東西。

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(dir: &std::path::Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    const VALID: &str = r#"{"schema_version":1,"id":"mori.demo","name":"Demo","kind":"cli"}"#;

    #[test]
    fn scans_subdir_manifest_and_flat_json() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "mori.demo/manifest.json", VALID);
        write(tmp.path(), "mori.flat.json", VALID);
        let found = scan_body_parts(tmp.path());
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|d| d.status == "valid"));
        assert!(found.iter().any(|d| d.manifest.as_ref().unwrap().id == "mori.demo"));
    }

    #[test]
    fn records_parse_error_without_crashing() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "broken/manifest.json", "{ not json");
        let found = scan_body_parts(tmp.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].status, "parse_error");
        assert!(found[0].manifest.is_none());
        assert!(found[0].detail.is_some());
    }

    #[test]
    fn flags_unsupported_schema() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "future/manifest.json",
            r#"{"schema_version":99,"id":"x","name":"X","kind":"cli"}"#);
        let found = scan_body_parts(tmp.path());
        assert_eq!(found[0].status, "unsupported_schema");
        assert!(found[0].manifest.is_some()); // 能讀就降級讀
    }

    #[test]
    fn missing_dir_returns_empty_not_error() {
        let tmp = TempDir::new().unwrap();
        let found = scan_body_parts(&tmp.path().join("nonexistent"));
        assert!(found.is_empty());
    }
}
```

- [ ] **Step 2: 跑測試確認 fail**

Run: `cargo test -p mori-core --lib body::registry`
Expected: 編譯失敗。

- [ ] **Step 3: 寫實作**

```rust
use crate::body::manifest::{manifest_status, parse_manifest, BodyManifest, ManifestStatus};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 掃到的一個 body part:來源路徑 + 狀態 + (可降級的)manifest。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredBodyPart {
    pub source: String,
    /// "valid" | "unsupported_schema" | "parse_error"
    pub status: String,
    pub detail: Option<String>,
    pub manifest: Option<BodyManifest>,
}

/// 掃 `base/<id>/manifest.json` 與 `base/*.json`。唯讀,不啟動任何東西。
/// 任何 IO / parse 失敗都記成一筆 `parse_error`,不中斷整個掃描、不 panic。
pub fn scan_body_parts(base: &Path) -> Vec<DiscoveredBodyPart> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(base) {
        Ok(e) => e,
        Err(_) => return out, // 目錄不存在 → 空
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let manifest_file = if path.is_dir() {
            let m = path.join("manifest.json");
            if m.is_file() {
                m
            } else {
                continue;
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            path.clone()
        } else {
            continue;
        };
        out.push(read_one(&manifest_file));
    }
    out.sort_by(|a, b| a.source.cmp(&b.source)); // 穩定順序
    out
}

fn read_one(manifest_file: &Path) -> DiscoveredBodyPart {
    let source = manifest_file.to_string_lossy().into_owned();
    let body = match std::fs::read_to_string(manifest_file) {
        Ok(b) => b,
        Err(e) => return parse_err(source, e.to_string()),
    };
    match parse_manifest(&body) {
        Ok(m) => {
            let status = match manifest_status(&m) {
                ManifestStatus::Valid => "valid",
                ManifestStatus::UnsupportedSchema(_) => "unsupported_schema",
            };
            DiscoveredBodyPart {
                source,
                status: status.to_string(),
                detail: None,
                manifest: Some(m),
            }
        }
        Err(e) => parse_err(source, e),
    }
}

fn parse_err(source: String, detail: String) -> DiscoveredBodyPart {
    DiscoveredBodyPart {
        source,
        status: "parse_error".to_string(),
        detail: Some(detail),
        manifest: None,
    }
}
```

- [ ] **Step 4: 在 `body/mod.rs` 加 `pub mod registry;` + re-export**

```rust
pub mod registry;
```
```rust
pub use registry::{scan_body_parts, DiscoveredBodyPart};
```

- [ ] **Step 5: 跑測試確認 pass**

Run: `cargo test -p mori-core --lib body::registry`
Expected: 4 test PASS。Run `cargo test -p mori-core --lib`(無回歸)。

- [ ] **Step 6: Commit**

```bash
git add crates/mori-core/src/body/registry.rs crates/mori-core/src/body/mod.rs
git commit -m "feat(bi-1): body registry scan_body_parts (read-only)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: bundle + 寫入 MoriPack Studio manifest

**Files:**
- Create: `crates/mori-tauri/src/body_registry.rs`
- Modify: `crates/mori-tauri/src/main.rs`(啟動時呼叫 `ensure_bundled_body_parts()` — 找現有 `character_pack::ensure_default()` 呼叫處旁邊加)

> 前置:先 `grep -n "ensure_default" crates/mori-tauri/src/main.rs` 找到 character pack 啟動初始化的呼叫點,把 `ensure_bundled_body_parts()` 加在同一區(同樣 best-effort,失敗只 log warn 不 panic)。並 `grep -n "fn mori_dir\|\.mori" crates/mori-tauri/src/character_pack.rs` 確認 `~/.mori` 根目錄 helper 的取法,`body_parts_dir()` 用同一個 helper。

- [ ] **Step 1: 寫 `body_registry.rs`(含 test)**

```rust
//! BI-1:body part manifest 目錄 + bundled 第一方 body part manifest。
//! MoriPack Studio 是第一個 registered body part(artifact-first 工具)。

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// bundled 第一方 body part manifest:MoriPack Studio。
/// 對齊 docs/moripack-integration.md §Phase 3 sample。
const MORIPACK_STUDIO_MANIFEST: &str = r#"{
  "schema_version": 1,
  "id": "mori.moripack-studio",
  "name": "MoriPack Studio",
  "kind": "standalone_app",
  "description": "Mori 角色包(.moripack.zip)的外部編輯器。",
  "entrypoints": { "web": "https://mori-sprite-studio.vercel.app/" },
  "capabilities": ["character_pack.edit", "character_pack.export"],
  "permissions": ["filesystem.read.character_pack", "filesystem.write.character_pack_export"],
  "data_policy": { "owns_raw_data": false, "default_ingestion": "off" }
}
"#;

/// `~/.mori/body-parts/`。沿用 character_pack 的 mori 根目錄 helper。
pub fn body_parts_dir() -> PathBuf {
    // 注意:實作時改用 main repo 既有的 mori_dir helper(character_pack.rs 用的同一個),
    // 不要自己重算 HOME。下面僅示意路徑形狀。
    crate::character_pack::characters_dir()
        .parent()
        .map(|p| p.join("body-parts"))
        .unwrap_or_else(|| PathBuf::from("body-parts"))
}

/// 啟動時確保 bundled 第一方 body part manifest 存在(不覆蓋既有 — user/第三方可能改過)。
pub fn ensure_bundled_body_parts() -> Result<()> {
    write_if_absent(
        &body_parts_dir().join("mori.moripack-studio").join("manifest.json"),
        MORIPACK_STUDIO_MANIFEST,
    )
}

fn write_if_absent(path: &Path, body: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_moripack_manifest_is_valid_and_parses() {
        // bundled manifest 本身必須是合法、可被 mori-core 解析的 v1 manifest。
        let m = mori_core::body::parse_manifest(MORIPACK_STUDIO_MANIFEST)
            .expect("bundled manifest must parse");
        assert_eq!(m.id, "mori.moripack-studio");
        assert_eq!(m.kind, mori_core::body::BodyKind::StandaloneApp);
        assert_eq!(
            mori_core::body::manifest_status(&m),
            mori_core::body::ManifestStatus::Valid
        );
    }

    #[test]
    fn write_if_absent_writes_then_skips() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().join("a/manifest.json");
        write_if_absent(&p, "first").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "first");
        write_if_absent(&p, "second").unwrap(); // 已存在 → 不覆蓋
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "first");
    }
}
```

> 實作備註:`body_parts_dir()` 上面是示意 —— 動手時請改用 `character_pack.rs` 內實際的 `~/.mori` 根目錄函式(例如 `mori_dir()` 若存在),`characters_dir()` 是否有 `pub` 的 parent helper 要先確認;目標是 `~/.mori/body-parts/`,跟 `~/.mori/characters/` 同層。

- [ ] **Step 2: 宣告 module + 啟動呼叫**

`main.rs` 頂部 module 區加 `mod body_registry;`。在 `character_pack::ensure_default()` 呼叫處旁加:
```rust
if let Err(e) = body_registry::ensure_bundled_body_parts() {
    tracing::warn!(error = %e, "ensure_bundled_body_parts failed (non-fatal)");
}
```

- [ ] **Step 3: 跑測試**

Run: `cargo test -p mori-tauri --bin mori-tauri body_registry`
Expected: 2 test PASS。Run `cargo check -p mori-tauri`。

- [ ] **Step 4: Commit**

```bash
git add crates/mori-tauri/src/body_registry.rs crates/mori-tauri/src/main.rs
git commit -m "feat(bi-1): bundle MoriPack Studio body manifest + write on startup

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: `body_registry_list` Tauri command

**Files:**
- Modify: `crates/mori-tauri/src/main.rs`(command + 註冊)

- [ ] **Step 1: 加 command**

加在 body / character 命令區:
```rust
/// BI-1:唯讀掃描 ~/.mori/body-parts/ 回傳 body part 清單。不啟動/不執行任何東西。
#[tauri::command]
fn body_registry_list() -> Result<Vec<mori_core::body::DiscoveredBodyPart>, String> {
    Ok(mori_core::body::scan_body_parts(&crate::body_registry::body_parts_dir()))
}
```

- [ ] **Step 2: 註冊**

`tauri::generate_handler![...]` 加一行 `body_registry_list,`。

- [ ] **Step 3: 編譯確認**

Run: `cargo check -p mori-tauri`
Expected: 乾淨。

- [ ] **Step 4: Commit**

```bash
git add crates/mori-tauri/src/main.rs
git commit -m "feat(bi-1): body_registry_list tauri command

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: 前端唯讀 `BodyTab`

**Files:**
- Create: `src/tabs/BodyTab.tsx`
- Modify: sidebar/tab 註冊處(先 `grep -rn "AnnuliTab\|SubTabId\|tabs/" src/` 找到既有 tab 註冊與 sidebar 清單,照同 pattern 加「身體 / Body」)

> 走 **integration + manual verify**(repo UI 慣例手測)。

- [ ] **Step 1: 寫 `BodyTab.tsx`**

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface BodyManifest {
  schema_version: number;
  id: string;
  name: string;
  kind: string;
  description?: string;
  capabilities?: string[];
  interfaces?: { name: string; transport: string }[];
  permissions?: string[];
}
interface DiscoveredBodyPart {
  source: string;
  status: string; // valid | unsupported_schema | parse_error
  detail: string | null;
  manifest: BodyManifest | null;
}

export function BodyTab() {
  const [parts, setParts] = useState<DiscoveredBodyPart[]>([]);
  const [err, setErr] = useState<string | null>(null);

  const refresh = async () => {
    try {
      setParts(await invoke<DiscoveredBodyPart[]>("body_registry_list"));
      setErr(null);
    } catch (e: any) {
      setErr(String(e));
    }
  };
  useEffect(() => { refresh(); }, []);

  return (
    <div style={{ padding: 16 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 12 }}>
        <h2 style={{ margin: 0 }}>身體部件</h2>
        <button className="mori-btn small ghost" onClick={refresh}>重新整理</button>
      </div>
      <p style={{ opacity: 0.7, fontSize: 12 }}>
        掃描 <code>~/.mori/body-parts/</code> 的 manifest(唯讀;不會啟動或執行任何部件)。
      </p>
      {err && <div style={{ color: "rgba(255,160,160,.95)", fontSize: 12 }}>❌ {err}</div>}
      {parts.length === 0 && !err && <div style={{ opacity: 0.6 }}>還沒有任何 body part。</div>}
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        {parts.map((p) => (
          <div key={p.source} style={{ border: "1px solid var(--c-border)", borderRadius: 8, padding: 10 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <strong>{p.manifest?.name ?? "(無法解析)"}</strong>
              <span style={{ fontSize: 11, opacity: 0.7 }}>{p.manifest?.kind}</span>
              <StatusBadge status={p.status} />
            </div>
            {p.manifest?.id && <div style={{ fontSize: 11, opacity: 0.6 }}>{p.manifest.id}</div>}
            {p.manifest?.capabilities?.length ? (
              <div style={{ fontSize: 12, marginTop: 4 }}>能力:{p.manifest.capabilities.join(", ")}</div>
            ) : null}
            {p.manifest?.interfaces?.length ? (
              <div style={{ fontSize: 12 }}>介面:{p.manifest.interfaces.map((i) => `${i.name}(${i.transport})`).join(", ")}</div>
            ) : null}
            {p.detail && <div style={{ fontSize: 12, color: "rgba(255,160,160,.95)" }}>{p.detail}</div>}
            <div style={{ fontSize: 10, opacity: 0.4, wordBreak: "break-all", marginTop: 4 }}>{p.source}</div>
          </div>
        ))}
      </div>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const map: Record<string, { t: string; c: string }> = {
    valid: { t: "✓ 正常", c: "rgba(140,220,160,.9)" },
    unsupported_schema: { t: "⚠ 版本不支援", c: "rgba(230,200,120,.9)" },
    parse_error: { t: "✗ 解析失敗", c: "rgba(255,160,160,.95)" },
  };
  const s = map[status] ?? { t: status, c: "var(--c-text-muted)" };
  return <span style={{ fontSize: 11, color: s.c }}>{s.t}</span>;
}
```

- [ ] **Step 2: 在 sidebar / tab 清單註冊「身體」tab**

照 `grep` 找到的既有 pattern(如 `AnnuliTab` 的註冊方式 + `SubTabId` / 主 tab 列表)加入 `BodyTab`。**只加入口,不改其他 tab。**

- [ ] **Step 3: build + 既有測試不破**

Run: `npm run build && npm test`
Expected: build 成功;既有 vitest 全綠。

- [ ] **Step 4: Manual verify(BI-1 e2e)**

```bash
npm run tauri dev
```
1. 啟動後 `~/.mori/body-parts/mori.moripack-studio/manifest.json` 應被建立(`ls` 確認)。
2. 點側欄「身體」tab → 應看到 **MoriPack Studio**(kind `standalone_app`,狀態 ✓ 正常,能力 `character_pack.edit, character_pack.export`)。
3. 手動丟一個壞 json 進 `~/.mori/body-parts/broken/manifest.json` → 重新整理 → 顯示「✗ 解析失敗」+ 不會 crash。
4. **確認此 tab 沒有任何「啟動/停止/授權/執行」按鈕**(BI-1 唯讀)。

- [ ] **Step 5: Commit**

```bash
git add src/tabs/BodyTab.tsx src/<sidebar-註冊檔>
git commit -m "feat(bi-1): read-only Body registry tab

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: 文件進度

**Files:**
- Modify: `docs/body-interface-backlog.md`

- [ ] **Step 1: 標 BI-1 done 判準**

把 §4「各 stage 的完成判準」的 BI-1 那條(若有)或在表後補:
```markdown
- **BI-1 done** ✅(YYYY-MM-DD,branch `feat/bi-1-body-registry`)= Desktop 掃 `~/.mori/body-parts/` 顯示 ≥1 個 manifest(MoriPack Studio),含 valid / unsupported_schema / parse_error 狀態;唯讀,無任何 write/exec 按鈕。
```

- [ ] **Step 2: Commit**

```bash
git add docs/body-interface-backlog.md
git commit -m "docs(bi-1): mark body registry progress

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage**(對 backlog BI-1 + body-interface §Manifest/Discovery/Versioning):
- manifest reader(讀 local manifest)→ Task 1+2 ✓
- 掃 `~/.mori/body-parts/*/manifest.json` + `*.json`(§Discovery)→ Task 2 ✓
- UI 顯示 body list + 狀態 → Task 5 ✓
- 「不執行任何高風險操作」→ 全程唯讀:scan 只讀檔、command 只回 list、UI 無動作按鈕、health=靜態有效性(M2)✓
- 把 MoriPack 回填成第一個 registered manifest → Task 3 ✓
- 版本降級(不懂的 capability/transport/schema 不 crash,§Versioning)→ `Transport::Other` + `UnsupportedSchema` 仍回 manifest ✓

**2. Placeholder scan:** 無 TBD;唯二「實作時確認」是 `body_parts_dir()` 要接 repo 既有的 `~/.mori` 根 helper、sidebar 註冊照既有 pattern —— 都標了 grep 前置步驟,非空白。

**3. Type consistency:** `DiscoveredBodyPart` 的欄位(source/status/detail/manifest)Rust ↔ TS interface 同名;`status` 字串值(valid/unsupported_schema/parse_error)三處一致(registry → command → BodyTab badge);command 名 `body_registry_list` Rust 定義/註冊/前端 invoke 三處一致;`BodyManifest` 欄位 Rust(serde snake_case / enum lowercase·snake_case)↔ TS interface 對齊。

**4. 範圍紀律複查:** 只引入 manifest 型別 + 唯讀掃描 + 唯讀 command + 唯讀 tab + 一份 bundled manifest。**無** permission broker(BI-2)、**無** live health 探測 / service 啟停(BI-3)、**無** robot/ROS2/zenoh 處理(只 `Other` 吃掉)。✓

---

## 全套驗證(收工前)
```bash
bash scripts/verify.sh
```

**Branch:** `feat/bi-1-body-registry`(等 #129 合進 main 後,從乾淨 main 切)
**Plan saved:** `docs/superpowers/plans/2026-05-28-bi-1-body-registry.md`
