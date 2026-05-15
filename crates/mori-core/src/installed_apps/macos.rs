//! macOS 平台 installed apps scan — `.app` bundle 掃描。
//!
//! 來源:
//! - `/Applications/*.app`
//! - `~/Applications/*.app`
//! - `/System/Applications/*.app`(macOS Catalina+ 系統 app)
//!
//! Bundle 是目錄,`.app` 後綴是 macOS Finder 認的;display_name 取 file stem(不
//! parse Info.plist 內 `CFBundleDisplayName` — 多數 .app stem 就是顯示名,parse
//! plist 留 v0.5.1)。
//!
//! `last_used_at` 取 `.app` 目錄 mtime。沒對接 Spotlight 的 kMDItemLastUsedDate
//! (需 core-foundation crate,留 v0.5.1+)。

use std::path::PathBuf;

use chrono::{DateTime, Utc};

use super::InstalledApp;

pub fn scan() -> Vec<InstalledApp> {
    let mut roots: Vec<(PathBuf, &'static str)> = vec![
        (PathBuf::from("/Applications"), "applications_system"),
        (PathBuf::from("/System/Applications"), "applications_macos"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push((PathBuf::from(home).join("Applications"), "applications_user"));
    }

    let mut apps: Vec<InstalledApp> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (root, source) in roots {
        scan_app_dir(&root, source, &mut apps, &mut seen);
    }
    apps
}

fn scan_app_dir(
    dir: &std::path::Path,
    source: &'static str,
    out: &mut Vec<InstalledApp>,
    seen: &mut std::collections::HashSet<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        // .app 是目錄(bundle),我們把整個 bundle 當一個 entry
        if !ext.eq_ignore_ascii_case("app") {
            // 遞迴掃子目錄(例如 /Applications/Utilities/ 內也有 .app)
            if path.is_dir() {
                scan_app_dir(&path, source, out, seen);
            }
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if !seen.insert(stem.to_lowercase()) {
            continue;
        }
        let last_used_at = entry.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| {
            let d = t.duration_since(std::time::UNIX_EPOCH).ok()?;
            DateTime::<Utc>::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
        });
        out.push(InstalledApp {
            display_name: stem.to_string(),
            launch_target: path.to_string_lossy().into_owned(),
            source: source.to_string(),
            last_used_at,
        });
    }
}
