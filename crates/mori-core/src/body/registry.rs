//! Body Registry — 掃描本機 body part manifest 目錄,回報每個 body part 的
//! 身分與狀態。**唯讀**:只讀檔、不啟動、不執行任何東西。

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
