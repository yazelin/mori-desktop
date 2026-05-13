//! 全域熱鍵設定 — `~/.mori/config.json` 的 `hotkeys` 子樹。
//!
//! 兩條 path 共用同一份設定:
//! - X11 session 走 `tauri-plugin-global-shortcut`(XGrabKey)
//! - Wayland session 走 `org.freedesktop.portal.GlobalShortcuts`,
//!   `key` 字串只當 portal 註冊時的 preferred_trigger 建議值,使用者改 GNOME
//!   Settings 後以系統設定為準。
//!
//! 設計:hybrid defaults + overrides。沒寫的欄位 fallback 到內建預設,
//! 所以最小 config 是 `{ "hotkeys": {} }` 或乾脆不寫 `hotkeys` key。

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use tauri_plugin_global_shortcut::Shortcut;

const DEFAULT_TOGGLE: &str = "Ctrl+Alt+Space";
const DEFAULT_CANCEL: &str = "Ctrl+Alt+Escape";
const DEFAULT_PICKER: &str = "Ctrl+Alt+P";
const DEFAULT_VOICE_SLOT_MODIFIER: &str = "Alt";
const DEFAULT_AGENT_SLOT_MODIFIER: &str = "Ctrl+Alt";

/// 反序列化 `~/.mori/config.json` 的 `hotkeys` 子樹。所有欄位皆可省略,缺的走預設。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HotkeyConfig {
    pub toggle: String,
    pub cancel: String,
    pub picker: String,
    /// 套用到 voice slot 0~9 的 modifier(預設 `Alt`)。
    pub voice_slot_modifier: String,
    /// 套用到 agent slot 0~9 的 modifier(預設 `Ctrl+Alt`)。
    pub agent_slot_modifier: String,
    /// 個別 voice slot override(key 是 slot 編號 0~9 字串,value 是完整 hotkey 字串如 `"F1"`)。
    pub voice_slot_overrides: HashMap<String, String>,
    /// 個別 agent slot override。
    pub agent_slot_overrides: HashMap<String, String>,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            toggle: DEFAULT_TOGGLE.to_string(),
            cancel: DEFAULT_CANCEL.to_string(),
            picker: DEFAULT_PICKER.to_string(),
            voice_slot_modifier: DEFAULT_VOICE_SLOT_MODIFIER.to_string(),
            agent_slot_modifier: DEFAULT_AGENT_SLOT_MODIFIER.to_string(),
            voice_slot_overrides: HashMap::new(),
            agent_slot_overrides: HashMap::new(),
        }
    }
}

/// 一個 action 對應一個 hotkey,resolve 後的中間表達。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HotkeyAction {
    Toggle,
    Cancel,
    Picker,
    VoiceSlot(u8),
    AgentSlot(u8),
}

impl HotkeyAction {
    /// Portal session 註冊用的穩定 ID。對應 `portal_hotkey.rs` 已 ship 的常數,
    /// 不能改(改了使用者 portal 權限會 reset)。
    pub fn portal_id(&self) -> String {
        match self {
            HotkeyAction::Toggle => "toggle".to_string(),
            HotkeyAction::Cancel => "cancel".to_string(),
            HotkeyAction::Picker => "picker".to_string(),
            HotkeyAction::VoiceSlot(n) => format!("slot-{n}"),
            HotkeyAction::AgentSlot(n) => format!("agent-slot-{n}"),
        }
    }

    /// portal 顯示給使用者看的描述(GNOME Settings UI 用)。
    pub fn description(&self) -> String {
        match self {
            HotkeyAction::Toggle => "Mori — 開始 / 停止錄音".to_string(),
            HotkeyAction::Cancel => "Mori — 錄音中按下取消(丟棄音檔,不送出)".to_string(),
            HotkeyAction::Picker => "Mori — 開 Profile picker 視窗(方向鍵選)".to_string(),
            HotkeyAction::VoiceSlot(0) => {
                "Mori — VoiceInput 純語音輸入(USER-00 極簡聽寫)".to_string()
            }
            HotkeyAction::VoiceSlot(n) => format!("Mori — 切換 VoiceInput Profile {n}"),
            HotkeyAction::AgentSlot(0) => "Mori — Agent 自由判斷模式(default Mori)".to_string(),
            HotkeyAction::AgentSlot(n) => format!("Mori — 切換 Agent Profile {n}"),
        }
    }
}

/// resolve 後的單筆 binding。
#[derive(Debug, Clone)]
pub struct HotkeyBinding {
    pub action: HotkeyAction,
    /// 使用者格式,例如 `"Ctrl+Alt+Space"`。tauri-plugin-global-shortcut 直接吃。
    pub key: String,
}

impl HotkeyBinding {
    /// Parse 成 plugin 用的 `Shortcut`。
    pub fn to_shortcut(&self) -> Result<Shortcut> {
        use std::str::FromStr;
        Shortcut::from_str(&self.key)
            .with_context(|| format!("invalid hotkey for {:?}: {:?}", self.action, self.key))
    }

    /// 轉成 portal 的 `preferred_trigger` 字串(`CTRL+ALT+space` 格式)。
    pub fn to_portal_trigger(&self) -> String {
        to_portal_trigger(&self.key)
    }
}

impl HotkeyConfig {
    /// 從 `~/.mori/config.json` 讀 `hotkeys` 子樹;檔案或欄位不存在就用預設。
    pub fn load(config_path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(config_path) else {
            return Self::default();
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            tracing::warn!(
                path = %config_path.display(),
                "config.json malformed, hotkeys fall back to defaults",
            );
            return Self::default();
        };
        let Some(hotkeys) = json.get("hotkeys") else {
            return Self::default();
        };
        match serde_json::from_value::<HotkeyConfig>(hotkeys.clone()) {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::warn!(
                    ?e,
                    "hotkeys section invalid, falling back to defaults",
                );
                Self::default()
            }
        }
    }

    /// 展開成 22 筆 binding,跑衝突檢查 + 語法驗證。
    /// 衝突回 `Err`;單筆語法錯也回 `Err`,因為任一鍵打錯整段熱鍵都該擋下來給使用者修。
    pub fn resolve(&self) -> Result<Vec<HotkeyBinding>> {
        let mut out: Vec<HotkeyBinding> = Vec::with_capacity(22);

        out.push(HotkeyBinding {
            action: HotkeyAction::Toggle,
            key: self.toggle.clone(),
        });
        out.push(HotkeyBinding {
            action: HotkeyAction::Cancel,
            key: self.cancel.clone(),
        });
        out.push(HotkeyBinding {
            action: HotkeyAction::Picker,
            key: self.picker.clone(),
        });

        for n in 0u8..=9 {
            let voice_key = self
                .voice_slot_overrides
                .get(&n.to_string())
                .cloned()
                .unwrap_or_else(|| format!("{}+{n}", self.voice_slot_modifier));
            out.push(HotkeyBinding {
                action: HotkeyAction::VoiceSlot(n),
                key: voice_key,
            });

            let agent_key = self
                .agent_slot_overrides
                .get(&n.to_string())
                .cloned()
                .unwrap_or_else(|| format!("{}+{n}", self.agent_slot_modifier));
            out.push(HotkeyBinding {
                action: HotkeyAction::AgentSlot(n),
                key: agent_key,
            });
        }

        // 語法驗證(plugin 解析器是 super-set,能 parse 的 portal 也能正規化)
        for b in &out {
            b.to_shortcut()?;
        }

        // 衝突偵測 — 用 normalized form 比對(避免 "Ctrl+Alt+P" vs "Alt+Ctrl+P" 漏掉)
        let mut seen: HashMap<String, HotkeyAction> = HashMap::new();
        for b in &out {
            let normalized = normalize_for_compare(&b.key);
            if let Some(prev) = seen.insert(normalized.clone(), b.action.clone()) {
                anyhow::bail!(
                    "hotkey conflict: {:?} and {:?} both map to {}",
                    prev,
                    b.action,
                    b.key,
                );
            }
        }

        Ok(out)
    }
}

/// `Ctrl+Alt+P` → `CTRL+ALT+p`(portal trigger 格式,XKB keysym 名稱)。
fn to_portal_trigger(key: &str) -> String {
    let tokens: Vec<&str> = key.split('+').map(|t| t.trim()).collect();
    let mut parts: Vec<String> = Vec::with_capacity(tokens.len());
    for tok in tokens {
        let upper = tok.to_uppercase();
        match upper.as_str() {
            "CTRL" | "CONTROL" => parts.push("CTRL".to_string()),
            "ALT" | "OPTION" => parts.push("ALT".to_string()),
            "SHIFT" => parts.push("SHIFT".to_string()),
            "SUPER" | "META" | "CMD" | "COMMAND" => parts.push("SUPER".to_string()),
            _ => parts.push(key_to_keysym(tok)),
        }
    }
    parts.join("+")
}

/// 單獨的 key token → X11 keysym name(空白 → `space`、Escape → `Escape`、p → `p` 等)。
/// 對齊 keysymdef.h 慣例;不在 lookup table 的走 best-effort(原樣保留)。
fn key_to_keysym(key: &str) -> String {
    let upper = key.to_uppercase();
    // 單字母 A-Z → 小寫
    if upper.len() == 1 {
        let c = upper.chars().next().unwrap();
        if c.is_ascii_alphabetic() {
            return upper.to_lowercase();
        }
        if c.is_ascii_digit() {
            return upper;
        }
    }
    // Tauri 的 Code 形式 — KeyA / Digit0
    if let Some(rest) = upper.strip_prefix("KEY") {
        if rest.len() == 1 {
            return rest.to_lowercase();
        }
    }
    if let Some(rest) = upper.strip_prefix("DIGIT") {
        if rest.len() == 1 {
            return rest.to_string();
        }
    }
    match upper.as_str() {
        "SPACE" => "space".to_string(),
        "ESCAPE" | "ESC" => "Escape".to_string(),
        "ENTER" | "RETURN" => "Return".to_string(),
        "TAB" => "Tab".to_string(),
        "BACKSPACE" => "BackSpace".to_string(),
        "DELETE" | "DEL" => "Delete".to_string(),
        "INSERT" | "INS" => "Insert".to_string(),
        "HOME" => "Home".to_string(),
        "END" => "End".to_string(),
        "PAGEUP" => "Page_Up".to_string(),
        "PAGEDOWN" => "Page_Down".to_string(),
        "LEFT" | "ARROWLEFT" => "Left".to_string(),
        "RIGHT" | "ARROWRIGHT" => "Right".to_string(),
        "UP" | "ARROWUP" => "Up".to_string(),
        "DOWN" | "ARROWDOWN" => "Down".to_string(),
        "MINUS" | "-" => "minus".to_string(),
        "EQUAL" | "=" => "equal".to_string(),
        "COMMA" | "," => "comma".to_string(),
        "PERIOD" | "." => "period".to_string(),
        "SLASH" | "/" => "slash".to_string(),
        "BACKSLASH" | "\\" => "backslash".to_string(),
        "BRACKETLEFT" | "[" => "bracketleft".to_string(),
        "BRACKETRIGHT" | "]" => "bracketright".to_string(),
        "SEMICOLON" | ";" => "semicolon".to_string(),
        "QUOTE" | "APOSTROPHE" | "'" => "apostrophe".to_string(),
        "BACKQUOTE" | "GRAVE" | "`" => "grave".to_string(),
        s if s.starts_with('F') && s[1..].parse::<u32>().is_ok() => s.to_string(),
        _ => key.to_string(),
    }
}

/// 衝突檢查用 — 把 hotkey 字串歸一成「sorted modifier set + key」格式。
fn normalize_for_compare(key: &str) -> String {
    let tokens: Vec<&str> = key.split('+').map(|t| t.trim()).collect();
    let mut mods: Vec<String> = Vec::new();
    let mut k: Option<String> = None;
    for tok in tokens {
        let upper = tok.to_uppercase();
        match upper.as_str() {
            "CTRL" | "CONTROL" => mods.push("CTRL".to_string()),
            "ALT" | "OPTION" => mods.push("ALT".to_string()),
            "SHIFT" => mods.push("SHIFT".to_string()),
            "SUPER" | "META" | "CMD" | "COMMAND" => mods.push("SUPER".to_string()),
            _ => k = Some(key_to_keysym(tok).to_uppercase()),
        }
    }
    mods.sort();
    mods.dedup();
    let key_part = k.unwrap_or_default();
    if mods.is_empty() {
        key_part
    } else {
        format!("{}+{}", mods.join("+"), key_part)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_resolve_without_conflict() {
        let cfg = HotkeyConfig::default();
        let bindings = cfg.resolve().unwrap();
        assert_eq!(bindings.len(), 23); // toggle + cancel + picker + 10 voice + 10 agent
    }

    #[test]
    fn portal_trigger_format_matches_existing() {
        assert_eq!(to_portal_trigger("Ctrl+Alt+Space"), "CTRL+ALT+space");
        assert_eq!(to_portal_trigger("Ctrl+Alt+Escape"), "CTRL+ALT+Escape");
        assert_eq!(to_portal_trigger("Ctrl+Alt+P"), "CTRL+ALT+p");
        assert_eq!(to_portal_trigger("Alt+0"), "ALT+0");
        assert_eq!(to_portal_trigger("F1"), "F1");
    }

    #[test]
    fn conflict_detection_catches_duplicates() {
        let mut cfg = HotkeyConfig::default();
        // 把 picker 改成跟 toggle 同鍵
        cfg.picker = "Ctrl+Alt+Space".to_string();
        let err = cfg.resolve().unwrap_err();
        assert!(err.to_string().contains("conflict"), "got: {err}");
    }

    #[test]
    fn slot_overrides_apply() {
        let mut cfg = HotkeyConfig::default();
        cfg.voice_slot_overrides
            .insert("0".to_string(), "F1".to_string());
        let bindings = cfg.resolve().unwrap();
        let slot0 = bindings
            .iter()
            .find(|b| b.action == HotkeyAction::VoiceSlot(0))
            .unwrap();
        assert_eq!(slot0.key, "F1");
    }

    #[test]
    fn modifier_swap_caught_in_normalize() {
        assert_eq!(
            normalize_for_compare("Ctrl+Alt+P"),
            normalize_for_compare("Alt+Ctrl+P"),
        );
    }

    #[test]
    fn invalid_key_rejected() {
        let mut cfg = HotkeyConfig::default();
        cfg.toggle = "Ctrl+Alt+NotARealKey".to_string();
        assert!(cfg.resolve().is_err());
    }
}
