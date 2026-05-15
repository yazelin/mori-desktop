//! brand-3: Theme system。
//!
//! Theme 檔在 `~/.mori/themes/*.json`,VSCode-like 使用者可自訂。
//! 內建 `dark.json` / `light.json` 啟動時 ensure(沒就寫入,有就不動 — 讓 user
//! 編輯保留)。Active theme stem 存在 `~/.mori/active_theme`(plain text 一行)。
//!
//! Theme JSON:
//! ```json
//! { "name": "Mori Dark", "base": "dark", "builtin": true,
//!   "colors": { "page-bg": "#1f3329", "text": "rgba(...)", ... } }
//! ```
//!
//! `colors` map 的 key 對應 CSS variable name(去掉 `--c-` 前綴);value 是
//! 任何合法 CSS color string。frontend 把每 entry 設成 `--c-<key>` style property。

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    /// "dark" | "light" — frontend 用來決定 color-scheme + native widget tint
    pub base: String,
    #[serde(default)]
    pub builtin: bool,
    pub colors: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThemeEntry {
    pub stem: String,
    pub name: String,
    pub base: String,
    pub builtin: bool,
}

const BUILTIN_DARK: &str = include_str!("../themes/dark.json");
const BUILTIN_LIGHT: &str = include_str!("../themes/light.json");

pub fn themes_dir() -> PathBuf {
    crate::mori_dir().join("themes")
}

pub fn active_theme_path() -> PathBuf {
    crate::mori_dir().join("active_theme")
}

/// 啟動時呼叫:確保 themes 目錄存在 + 內建 dark/light 不存在則寫入。
/// 已存在就不動,讓 user 自己編輯保留。
pub fn ensure_builtin() -> Result<()> {
    let dir = themes_dir();
    std::fs::create_dir_all(&dir)?;
    for (stem, body) in [("dark", BUILTIN_DARK), ("light", BUILTIN_LIGHT)] {
        let p = dir.join(format!("{stem}.json"));
        if !p.exists() {
            std::fs::write(&p, body)?;
        }
    }
    Ok(())
}

pub fn list() -> Result<Vec<ThemeEntry>> {
    let dir = themes_dir();
    let mut entries = vec![];
    if !dir.exists() {
        return Ok(entries);
    }
    for ent in std::fs::read_dir(&dir)? {
        let ent = ent?;
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let body = match std::fs::read_to_string(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let theme: Theme = match serde_json::from_str(&body) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(stem = %stem, error = %e, "skip invalid theme json");
                continue;
            }
        };
        entries.push(ThemeEntry {
            stem,
            name: theme.name,
            base: theme.base,
            builtin: theme.builtin,
        });
    }
    // builtin 排前面,同類 name 字典序
    entries.sort_by(|a, b| b.builtin.cmp(&a.builtin).then(a.name.cmp(&b.name)));
    Ok(entries)
}

pub fn read(stem: &str) -> Result<Theme> {
    let path = themes_dir().join(format!("{stem}.json"));
    let body = std::fs::read_to_string(&path)?;
    let theme: Theme = serde_json::from_str(&body)?;
    Ok(theme)
}

pub fn get_active_stem() -> String {
    get_active_stem_with_default(false)
}

/// v0.4.1:首次啟動沒 active_theme 檔時,用 `default_light` 提示 fallback
/// 是該選 light 還是 dark。前端可從 `prefers-color-scheme` 算這個值傳進來
/// → user 在 OS 設了 light theme,Mori 預設也跟 light。
/// 已存在 active_theme 檔時 hint 忽略 — 尊重 user 顯式 set 過的值。
pub fn get_active_stem_with_default(default_light: bool) -> String {
    std::fs::read_to_string(active_theme_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| if default_light { "light".to_string() } else { "dark".to_string() })
}

pub fn set_active_stem(stem: &str) -> Result<()> {
    std::fs::write(active_theme_path(), stem)?;
    Ok(())
}

/// 給 user 一個方便方式 toggle dark <-> light 的對應 stem
/// (從同 base 內建 theme 找一個切過去)
pub fn toggle_base_stem(current_stem: &str) -> Result<String> {
    let current = read(current_stem).ok();
    let want_base = match current.as_ref().map(|t| t.base.as_str()) {
        Some("dark") => "light",
        _ => "dark", // light or unknown → 切到 dark
    };
    // 先找 builtin 對應 base
    let entries = list()?;
    if let Some(e) = entries
        .iter()
        .find(|e| e.base == want_base && e.builtin)
    {
        return Ok(e.stem.clone());
    }
    // 沒 builtin 同 base 的話,找任何 non-builtin 同 base
    if let Some(e) = entries.iter().find(|e| e.base == want_base) {
        return Ok(e.stem.clone());
    }
    // fallback:回 stem 本身(切不過去就不切)
    Ok(current_stem.to_string())
}
