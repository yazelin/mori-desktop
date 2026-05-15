//! Linux 平台 installed apps scan — XDG `.desktop` 規格。
//!
//! 來源(去重以 desktop file 的 stem 為 key):
//! - `$XDG_DATA_HOME` / 預設 `~/.local/share/applications`(per-user)
//! - 每個 `$XDG_DATA_DIRS` / 預設 `/usr/share/applications`(system)
//! - `/var/lib/flatpak/exports/share/applications`(flatpak)
//! - `/var/lib/snapd/desktop/applications`(snap)
//!
//! Parse `.desktop` 行為:抓 `Name=` 為 display_name,`Exec=` 為 launch_target(去
//! freedesktop `%U / %F / %u / %f` 等 placeholder)。`NoDisplay=true` 跳過(不在
//! menu 顯示,通常是 helper / mime handler)。

use std::path::PathBuf;

use chrono::{DateTime, Utc};

use super::InstalledApp;

pub fn scan() -> Vec<InstalledApp> {
    let mut roots: Vec<(PathBuf, &'static str)> = Vec::new();
    // user
    let user_dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .map(|d| d.join("applications"));
    if let Some(d) = user_dir {
        roots.push((d, "xdg_desktop_user"));
    }
    // system
    if let Some(dirs) = std::env::var_os("XDG_DATA_DIRS") {
        for d in std::env::split_paths(&dirs) {
            roots.push((d.join("applications"), "xdg_desktop_system"));
        }
    } else {
        roots.push((PathBuf::from("/usr/share/applications"), "xdg_desktop_system"));
        roots.push((PathBuf::from("/usr/local/share/applications"), "xdg_desktop_system"));
    }
    // sandboxed package managers
    roots.push((PathBuf::from("/var/lib/flatpak/exports/share/applications"), "flatpak"));
    roots.push((PathBuf::from("/var/lib/snapd/desktop/applications"), "snap"));

    let mut apps: Vec<InstalledApp> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (root, source) in roots {
        scan_dir(&root, source, &mut apps, &mut seen);
    }
    apps
}

fn scan_dir(
    dir: &std::path::Path,
    source: &'static str,
    out: &mut Vec<InstalledApp>,
    seen: &mut std::collections::HashSet<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, source, out, seen);
            continue;
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("desktop") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if !seen.insert(stem.to_lowercase()) {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(&path) else { continue };
        let Some(parsed) = parse_desktop(&body) else { continue };
        let last_used_at = entry.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| {
            let d = t.duration_since(std::time::UNIX_EPOCH).ok()?;
            DateTime::<Utc>::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
        });
        out.push(InstalledApp {
            display_name: parsed.name,
            launch_target: parsed.exec.unwrap_or_else(|| path.to_string_lossy().into_owned()),
            source: source.to_string(),
            last_used_at,
        });
    }
}

struct DesktopEntry {
    name: String,
    exec: Option<String>,
}

fn parse_desktop(body: &str) -> Option<DesktopEntry> {
    // 走「進入 [Desktop Entry] section 後取 Name / Exec / NoDisplay」極簡 parser。
    // 不處理 localized Name[zh_TW]= 等多語版本(留 v0.5.1)。
    let mut in_main = false;
    let mut name: Option<String> = None;
    let mut exec: Option<String> = None;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_main = trimmed.eq_ignore_ascii_case("[Desktop Entry]");
            continue;
        }
        if !in_main {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "Name" => {
                if name.is_none() {
                    name = Some(value.to_string());
                }
            }
            "Exec" => exec = Some(clean_exec(value)),
            "NoDisplay" if value.eq_ignore_ascii_case("true") => return None,
            "Hidden" if value.eq_ignore_ascii_case("true") => return None,
            "Type" if !value.eq_ignore_ascii_case("Application") => return None,
            _ => {}
        }
    }
    Some(DesktopEntry { name: name?, exec })
}

/// 去掉 freedesktop spec 的 `%U / %F / %u / %f / %c / %i / %k` placeholder。
fn clean_exec(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            if chars.peek().is_some() {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_desktop() {
        let body = "[Desktop Entry]\nType=Application\nName=GIMP\nExec=gimp %F\n";
        let e = parse_desktop(body).unwrap();
        assert_eq!(e.name, "GIMP");
        assert_eq!(e.exec.as_deref(), Some("gimp"));
    }

    #[test]
    fn skip_nodisplay() {
        let body = "[Desktop Entry]\nType=Application\nName=Hidden\nNoDisplay=true\n";
        assert!(parse_desktop(body).is_none());
    }

    #[test]
    fn skip_non_application() {
        let body = "[Desktop Entry]\nType=Link\nName=Link\n";
        assert!(parse_desktop(body).is_none());
    }
}
