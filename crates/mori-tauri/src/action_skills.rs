//! Phase 5G-6: Action skills — Agent 模式呼叫的「動作」工具，封裝成 Skill。
//!
//! 之前在 5F-4 的 voice_input_tools.rs 把 open_url / open_app / send_keys 等
//! 直接接到 voice input 的 agent loop。5G 把它們改成 mori-core Skill trait
//! 實作，可以在 Agent 模式（chat pipeline）由 SkillRegistry 統一管理。
//!
//! 平台特定的 shell-out（xdg-open、ydotool、gtk-launch）留在 mori-tauri，
//! 因為 mori-core 不該有平台 API 依賴。

use anyhow::{anyhow, bail, Context as _, Result};
use async_trait::async_trait;
use mori_core::context::Context;
use mori_core::skill::{Skill, SkillOutput};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;

// ─── open_url ─────────────────────────────────────────────────────────────

pub struct OpenUrlSkill;

#[async_trait]
impl Skill for OpenUrlSkill {
    fn name(&self) -> &'static str {
        "open_url"
    }
    fn description(&self) -> &'static str {
        "Open an absolute URL (http:// or https://) in the system default browser \
         via xdg-open. Use when the user wants to navigate to a specific webpage."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Absolute URL with scheme."}
            },
            "required": ["url"]
        })
    }
    async fn execute(&self, args: Value, _ctx: &Context) -> Result<SkillOutput> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing 'url'"))?
            .trim();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            bail!("open_url only accepts http:// or https:// URLs");
        }
        tracing::info!(url, "skill open_url");
        let status = Command::new("xdg-open")
            .arg(url)
            .status()
            .context("spawn xdg-open")?;
        if status.success() {
            Ok(SkillOutput {
                user_message: format!("已開啟 {url}"),
                data: Some(json!({ "opened": url })),
            })
        } else {
            bail!("xdg-open exited {status}");
        }
    }
}

// ─── open_app ─────────────────────────────────────────────────────────────

pub struct OpenAppSkill;

#[async_trait]
impl Skill for OpenAppSkill {
    fn name(&self) -> &'static str {
        "open_app"
    }
    fn description(&self) -> &'static str {
        "Launch a locally installed desktop application by name (e.g. Firefox, Code, Slack). \
         Searches ~/.local/share/applications and /usr/share/applications .desktop entries."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "app": {"type": "string", "description": "Application name."}
            },
            "required": ["app"]
        })
    }
    async fn execute(&self, args: Value, _ctx: &Context) -> Result<SkillOutput> {
        let name = args
            .get("app")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing 'app'"))?
            .trim();
        tracing::info!(app = name, "skill open_app");

        if let Some(path) = find_desktop_file(name) {
            let desktop_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| anyhow!("invalid .desktop path"))?;
            Command::new("gtk-launch")
                .arg(desktop_id)
                .spawn()
                .context("spawn gtk-launch")?;
            Ok(SkillOutput {
                user_message: format!("已開啟 {name}"),
                data: Some(json!({ "launched": desktop_id })),
            })
        } else {
            match Command::new(name).spawn() {
                Ok(_) => Ok(SkillOutput {
                    user_message: format!("已開啟 {name}（無 .desktop，直接 spawn）"),
                    data: Some(json!({ "spawned": name })),
                }),
                Err(e) => bail!("no .desktop file and binary spawn failed: {e}"),
            }
        }
    }
}

fn find_desktop_file(query: &str) -> Option<PathBuf> {
    let q = query.to_lowercase();
    let dirs = [
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".local/share/applications")),
        Some(PathBuf::from("/usr/share/applications")),
        Some(PathBuf::from("/var/lib/flatpak/exports/share/applications")),
    ];
    for dir in dirs.iter().flatten() {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_lowercase();
            if stem.contains(&q) {
                return Some(path);
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                for line in content.lines() {
                    if let Some(name_value) = line.strip_prefix("Name=") {
                        if name_value.to_lowercase().contains(&q) {
                            return Some(path);
                        }
                        break;
                    }
                }
            }
        }
    }
    None
}

// ─── send_keys ────────────────────────────────────────────────────────────

pub struct SendKeysSkill;

#[async_trait]
impl Skill for SendKeysSkill {
    fn name(&self) -> &'static str {
        "send_keys"
    }
    fn description(&self) -> &'static str {
        "Send a keyboard shortcut to the currently focused window (Ctrl+S, Alt+Tab, \
         Ctrl+Shift+Period etc.). NOT for typing text — use other text-insertion skills."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "keys": {"type": "string", "description": "Key combo like \"Ctrl+S\"."}
            },
            "required": ["keys"]
        })
    }
    async fn execute(&self, args: Value, _ctx: &Context) -> Result<SkillOutput> {
        let keys = args
            .get("keys")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing 'keys'"))?
            .trim();
        tracing::info!(keys, "skill send_keys");
        let codes = parse_key_combo(keys)?;
        let mut cmd = Command::new("ydotool");
        cmd.arg("key");
        for c in &codes {
            cmd.arg(c);
        }
        let status = cmd.status().context("spawn ydotool")?;
        if status.success() {
            Ok(SkillOutput {
                user_message: format!("已送出 {keys}"),
                data: Some(json!({ "sent": keys })),
            })
        } else {
            bail!("ydotool exited {status}");
        }
    }
}

fn parse_key_combo(combo: &str) -> Result<Vec<String>> {
    let parts: Vec<&str> = combo.split('+').map(|s| s.trim()).collect();
    if parts.is_empty() {
        bail!("empty key combo");
    }
    let mut codes: Vec<u16> = Vec::new();
    for p in &parts {
        codes.push(key_name_to_code(p).ok_or_else(|| anyhow!("unknown key '{}'", p))?);
    }
    let mut out: Vec<String> = codes.iter().map(|c| format!("{c}:1")).collect();
    for c in codes.iter().rev() {
        out.push(format!("{c}:0"));
    }
    Ok(out)
}

#[rustfmt::skip]
fn key_name_to_code(name: &str) -> Option<u16> {
    match name.to_lowercase().as_str() {
        "ctrl" | "control" => Some(29),
        "shift" => Some(42),
        "alt" => Some(56),
        "super" | "win" | "meta" => Some(125),
        "enter" | "return" => Some(28),
        "escape" | "esc" => Some(1),
        "tab" => Some(15),
        "space" => Some(57),
        "backspace" => Some(14),
        "delete" => Some(111),
        "up" => Some(103),"down" => Some(108),"left" => Some(105),"right" => Some(106),
        "home" => Some(102),"end" => Some(107),"pageup" => Some(104),"pagedown" => Some(109),
        "a"=>Some(30),"b"=>Some(48),"c"=>Some(46),"d"=>Some(32),"e"=>Some(18),"f"=>Some(33),
        "g"=>Some(34),"h"=>Some(35),"i"=>Some(23),"j"=>Some(36),"k"=>Some(37),"l"=>Some(38),
        "m"=>Some(50),"n"=>Some(49),"o"=>Some(24),"p"=>Some(25),"q"=>Some(16),"r"=>Some(19),
        "s"=>Some(31),"t"=>Some(20),"u"=>Some(22),"v"=>Some(47),"w"=>Some(17),"x"=>Some(45),
        "y"=>Some(21),"z"=>Some(44),
        "0"=>Some(11),"1"=>Some(2),"2"=>Some(3),"3"=>Some(4),"4"=>Some(5),
        "5"=>Some(6),"6"=>Some(7),"7"=>Some(8),"8"=>Some(9),"9"=>Some(10),
        "f1"=>Some(59),"f2"=>Some(60),"f3"=>Some(61),"f4"=>Some(62),"f5"=>Some(63),
        "f6"=>Some(64),"f7"=>Some(65),"f8"=>Some(66),"f9"=>Some(67),"f10"=>Some(68),
        "f11"=>Some(87),"f12"=>Some(88),
        "period" | "." => Some(52),"comma" | "," => Some(51),"semicolon" | ";" => Some(39),
        "minus" | "-" => Some(12),"equal" | "=" => Some(13),"slash" | "/" => Some(53),
        "backslash" | "\\" => Some(43),"leftbracket" | "[" => Some(26),
        "rightbracket" | "]" => Some(27),"apostrophe" | "'" => Some(40),
        "grave" | "`" => Some(41),
        _ => None,
    }
}

// ─── URL 模板系列：google_search / ask_chatgpt / ask_gemini / find_youtube ─

fn url_open_skill_exec(
    args: &Value,
    arg_key: &str,
    template: &str,
    label: &str,
) -> Result<SkillOutput> {
    let q = args
        .get(arg_key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("missing '{}'", arg_key))?;
    let url = template.replace("{}", &urlencode(q));
    let status = Command::new("xdg-open")
        .arg(&url)
        .status()
        .context("spawn xdg-open")?;
    if !status.success() {
        bail!("xdg-open exited {status}");
    }
    Ok(SkillOutput {
        user_message: format!("已{label}「{q}」"),
        data: Some(json!({ "opened": url })),
    })
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

macro_rules! url_skill {
    ($struct_name:ident, $skill_name:literal, $desc:literal, $arg_key:literal, $template:literal, $label:literal) => {
        pub struct $struct_name;
        #[async_trait]
        impl Skill for $struct_name {
            fn name(&self) -> &'static str { $skill_name }
            fn description(&self) -> &'static str { $desc }
            fn parameters_schema(&self) -> Value {
                json!({
                    "type": "object",
                    "properties": { $arg_key: {"type": "string"} },
                    "required": [$arg_key]
                })
            }
            async fn execute(&self, args: Value, _ctx: &Context) -> Result<SkillOutput> {
                tracing::info!(skill = $skill_name, "skill {}", $skill_name);
                url_open_skill_exec(&args, $arg_key, $template, $label)
            }
        }
    };
}

url_skill!(
    GoogleSearchSkill,
    "google_search",
    "Open a Google search for the query in the system browser.",
    "query",
    "https://www.google.com/search?q={}",
    "搜尋"
);

url_skill!(
    AskChatGptSkill,
    "ask_chatgpt",
    "Open ChatGPT in the browser with a pre-filled prompt.",
    "prompt",
    "https://chatgpt.com/?prompt={}",
    "送 ChatGPT"
);

url_skill!(
    AskGeminiSkill,
    "ask_gemini",
    "Open Google Gemini in the browser with a pre-filled prompt that auto-submits.",
    "prompt",
    "https://gemini.google.com/app#autoSubmit=true&prompt={}",
    "送 Gemini"
);

url_skill!(
    FindYoutubeSkill,
    "find_youtube",
    "Open YouTube search results for the query.",
    "query",
    "https://www.youtube.com/results?search_query={}",
    "搜 YouTube"
);

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencode_basic() {
        assert_eq!(urlencode("hello world"), "hello%20world");
        assert_eq!(urlencode("中文"), "%E4%B8%AD%E6%96%87");
    }

    #[test]
    fn parse_key_ctrl_shift_period() {
        let codes = parse_key_combo("Ctrl+Shift+Period").unwrap();
        assert_eq!(codes, vec!["29:1", "42:1", "52:1", "52:0", "42:0", "29:0"]);
    }

    #[test]
    fn parse_key_unknown_fails() {
        assert!(parse_key_combo("Ctrl+ZZZ").is_err());
    }
}
