//! Installed apps catalog — 平台無關抽象 + cfg-gated platform impl。
//!
//! 用途:給 `OpenAppSkill` 的 LLM system prompt 注入「user 實際裝的 app 列表」,
//! 取代「LLM 用猜的 → ShellExecute / xdg-open / gtk-launch」這條失誤率高的路線。
//!
//! ## Scan source(依平台)
//!
//! | OS | 來源 |
//! |---|---|
//! | Windows | `%ProgramData%\Microsoft\Windows\Start Menu\Programs\*.lnk` + `%APPDATA%\Microsoft\Windows\Start Menu\Programs\*.lnk`(遞迴) |
//! | Linux | `$XDG_DATA_HOME` + `$XDG_DATA_DIRS` + 預設 `~/.local/share/applications`、`/usr/share/applications`、`/var/lib/flatpak/exports/share/applications` 的 `*.desktop` |
//! | macOS | `/Applications/*.app`、`~/Applications/*.app` |
//!
//! ## Usage signals
//!
//! v0.5.0 baseline 走「檔案 mtime as `lastUsedAt`」— 跨平台直接拿得到,不需要
//! Win UserAssist registry / Spotlight metadata 那種重型整合。未來版本可加更
//! 多 signal sources(`usageSignals: [{ source: "...", lastUsedAt: ... }]`)。
//!
//! ## Cache
//!
//! 平台掃完的結果寫 `~/.mori/installed-apps.<platform>.json`。`load_cached()`
//! 讀 cache,`refresh()` 重掃 + 覆寫 cache。`is_stale(hours)` 判斷是否該 refresh。

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// 一個已裝的桌面 app entry。跨平台同 schema。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledApp {
    /// User-facing 名字(`Chrome` / `KV STUDIO Ver.12G`)
    pub display_name: String,
    /// 啟動目標 — `.lnk` / `.desktop` / `.app` 路徑,或可執行檔絕對路徑
    pub launch_target: String,
    /// 來源(`start_menu` / `xdg_desktop` / `applications_folder` 等),
    /// debug 用,LLM context 不注入
    pub source: String,
    /// 最後使用時間(目前 = 檔案 mtime)。沒拿到回 None。
    pub last_used_at: Option<DateTime<Utc>>,
}

/// 完整 catalog,寫進 cache + 出去給前端 / LLM 用。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Catalog {
    pub platform: String,
    pub cached_at: DateTime<Utc>,
    pub applications: Vec<InstalledApp>,
}

impl Catalog {
    /// 依 `last_used_at` 降冪排序(最近用過在前)。沒 last_used 的塞最後。
    /// 給 LLM context 注入 top-K 時用。
    pub fn sorted_by_recency(&self) -> Vec<&InstalledApp> {
        let mut apps: Vec<&InstalledApp> = self.applications.iter().collect();
        apps.sort_by(|a, b| match (a.last_used_at, b.last_used_at) {
            (Some(a), Some(b)) => b.cmp(&a),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.display_name.cmp(&b.display_name),
        });
        apps
    }

    /// 是否 stale(超過 `hours` 沒更新)。caller 決定 refresh 政策。
    pub fn is_stale(&self, hours: i64) -> bool {
        let age = Utc::now().signed_duration_since(self.cached_at);
        age.num_hours() >= hours
    }
}

/// 取 cache 檔路徑 `~/.mori/installed-apps.<platform>.json`。
fn cache_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let platform = current_platform();
    Some(
        PathBuf::from(home)
            .join(".mori")
            .join(format!("installed-apps.{platform}.json")),
    )
}

/// 平台 short id,用在 cache 檔名 + Catalog.platform field。
pub fn current_platform() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        "other"
    }
}

/// 從磁碟 cache 讀;cache 不存在 / parse 失敗回 None,caller 該 refresh。
pub fn load_cached() -> Option<Catalog> {
    let path = cache_path()?;
    let body = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&body).ok()
}

/// 把 catalog 寫進 cache,失敗 log warn 不 panic。
pub fn save_to_cache(catalog: &Catalog) {
    let Some(path) = cache_path() else {
        tracing::warn!("installed_apps: no HOME/USERPROFILE, cache skipped");
        return;
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!(?e, "installed_apps: mkdir cache parent failed");
            return;
        }
    }
    match serde_json::to_string_pretty(catalog) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                tracing::warn!(?e, path = %path.display(), "installed_apps: write cache failed");
            }
        }
        Err(e) => tracing::warn!(?e, "installed_apps: serialize cache failed"),
    }
}

/// 平台 native scan — 跨平台 dispatch。回新 Catalog(內含當下 mtime)。
/// 慢:Win 走 fs walk + lnk parse,Linux 走 .desktop parse,macOS 走 .app dir 掃。
/// caller 應 spawn blocking task 跑,別擋 UI thread。
pub fn scan_now() -> Catalog {
    let applications = scan_platform();
    Catalog {
        platform: current_platform().to_string(),
        cached_at: Utc::now(),
        applications,
    }
}

fn scan_platform() -> Vec<InstalledApp> {
    #[cfg(target_os = "windows")]
    {
        windows::scan()
    }
    #[cfg(target_os = "linux")]
    {
        linux::scan()
    }
    #[cfg(target_os = "macos")]
    {
        macos::scan()
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}

/// 格式化成 LLM 可讀的 markdown section,塞 `OpenAppSkill` 的 description 用。
/// top_k 取最近用過前 K 個,避免全 inventory 塞爆 token(可能上千條 app)。
/// 沒 `last_used_at` 的排最後,跟 sorted_by_recency 一致。
pub fn format_for_llm(catalog: &Catalog, top_k: usize) -> String {
    let total = catalog.applications.len();
    let sorted = catalog.sorted_by_recency();
    let mut out = format!(
        "\n\n## 此機器已安裝的 app(共 {total} 個,以下列出最近常用 {} 個)\n\n",
        top_k.min(total)
    );
    for app in sorted.iter().take(top_k) {
        match app.last_used_at {
            Some(ts) => {
                let age = chrono::Utc::now().signed_duration_since(ts);
                let rel = if age.num_days() >= 30 {
                    format!("{}mo", (age.num_days() / 30).max(1))
                } else if age.num_days() > 0 {
                    format!("{}d", age.num_days())
                } else if age.num_hours() > 0 {
                    format!("{}h", age.num_hours())
                } else {
                    format!("{}m", age.num_minutes().max(1))
                };
                out.push_str(&format!("- `{}` (~{} ago)\n", app.display_name, rel));
            }
            None => out.push_str(&format!("- `{}`\n", app.display_name)),
        }
    }
    if total > top_k {
        out.push_str(&format!(
            "\n(還有 {} 個較少用的 app 沒列出。user 講的若不在上面 → 在 chat 跟 user 確認完整名稱,**不要亂猜**。)\n",
            total - top_k
        ));
    }
    out.push_str(
        "\n選 app 時請**直接 match 上面列表的 display name**(允許 fuzzy:user 講 \"SQL\" → match \"SQL Server Management Studio 21\")。\n",
    );
    out
}

/// Refresh = scan + save。回剛掃出來的 catalog。
pub fn refresh() -> Catalog {
    let catalog = scan_now();
    save_to_cache(&catalog);
    catalog
}

/// 拿 catalog:先讀 cache,沒 cache / stale 才 refresh。`stale_hours = None` 表示
/// 不檢查 stale(永遠用 cache 直到 caller 手動 refresh)。
pub fn get_or_refresh(stale_hours: Option<i64>) -> Catalog {
    match load_cached() {
        Some(c) if stale_hours.map(|h| !c.is_stale(h)).unwrap_or(true) => c,
        _ => refresh(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_by_recency_recent_first() {
        let cat = Catalog {
            platform: "test".into(),
            cached_at: Utc::now(),
            applications: vec![
                InstalledApp {
                    display_name: "old_app".into(),
                    launch_target: "/old".into(),
                    source: "test".into(),
                    last_used_at: Some(Utc::now() - chrono::Duration::days(30)),
                },
                InstalledApp {
                    display_name: "no_signal".into(),
                    launch_target: "/x".into(),
                    source: "test".into(),
                    last_used_at: None,
                },
                InstalledApp {
                    display_name: "recent".into(),
                    launch_target: "/recent".into(),
                    source: "test".into(),
                    last_used_at: Some(Utc::now() - chrono::Duration::hours(1)),
                },
            ],
        };
        let sorted = cat.sorted_by_recency();
        assert_eq!(sorted[0].display_name, "recent");
        assert_eq!(sorted[1].display_name, "old_app");
        assert_eq!(sorted[2].display_name, "no_signal");
    }

    #[test]
    fn is_stale_fresh() {
        let cat = Catalog {
            platform: "test".into(),
            cached_at: Utc::now(),
            applications: vec![],
        };
        assert!(!cat.is_stale(1));
    }

    #[test]
    fn is_stale_old() {
        let cat = Catalog {
            platform: "test".into(),
            cached_at: Utc::now() - chrono::Duration::days(2),
            applications: vec![],
        };
        assert!(cat.is_stale(24));
    }
}
