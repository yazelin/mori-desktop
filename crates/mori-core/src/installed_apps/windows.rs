//! Windows 平台 installed apps scan。
//!
//! v0.5.0 走「Start Menu `.lnk` 遞迴掃」路線。簡單跨機可重現,不用 Win32 binding。
//! 來源:
//! - `%ProgramData%\Microsoft\Windows\Start Menu\Programs\`(all-users)
//! - `%APPDATA%\Microsoft\Windows\Start Menu\Programs\`(user)
//! - `%PUBLIC%\Desktop`(common desktop)
//! - `%USERPROFILE%\Desktop`(user desktop)
//!
//! 不做的事(留 v0.5.1+):
//! - 不解析 `.lnk` 的 target(`launch_target` 留 .lnk path,ShellExecute 直接 open)
//! - 不讀 HKCU UserAssist registry(usage rank / freq 更準,但要 Win32 binding)
//! - 不掃 AppX / UWP / Microsoft Store apps
//!
//! `last_used_at` 取 `.lnk` 檔案 mtime 當代理(Start Menu 上 user 用過 = OS 通常會 touch
//! 該 lnk 更新 access time,但 NTFS 對 access time 預設 disabled,mtime 比較可靠的代理)。

use std::path::PathBuf;

use chrono::{DateTime, Utc};

use super::InstalledApp;

pub fn scan() -> Vec<InstalledApp> {
    let mut roots: Vec<(PathBuf, &'static str)> = Vec::new();
    if let Some(programdata) = std::env::var_os("ProgramData") {
        roots.push((
            PathBuf::from(programdata).join("Microsoft").join("Windows").join("Start Menu").join("Programs"),
            "start_menu_all_users",
        ));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        roots.push((
            PathBuf::from(appdata).join("Microsoft").join("Windows").join("Start Menu").join("Programs"),
            "start_menu_user",
        ));
    }
    if let Some(public) = std::env::var_os("PUBLIC") {
        roots.push((PathBuf::from(public).join("Desktop"), "desktop_public"));
    }
    if let Some(userprofile) = std::env::var_os("USERPROFILE") {
        roots.push((PathBuf::from(userprofile).join("Desktop"), "desktop_user"));
    }

    let mut apps: Vec<InstalledApp> = Vec::new();
    let mut seen_lower: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (root, source) in roots {
        walk_lnk(&root, source, &mut apps, &mut seen_lower);
    }
    apps
}

/// 遞迴走目錄找 `.lnk`,去重(display_name 大小寫不敏感,先進先得)。
fn walk_lnk(
    dir: &std::path::Path,
    source: &'static str,
    out: &mut Vec<InstalledApp>,
    seen: &mut std::collections::HashSet<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_lnk(&path, source, out, seen);
            continue;
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        // 主要看 .lnk;Win 也有 .url(網站捷徑),這版略過
        if !ext.eq_ignore_ascii_case("lnk") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let key = stem.to_lowercase();
        if !seen.insert(key) {
            continue; // 同名 lnk 多 source 重複,先進先得(start_menu 通常先進)
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
