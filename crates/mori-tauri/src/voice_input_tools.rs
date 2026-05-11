//! Phase 5F-4: Voice input agent loop 的工具實作。
//!
//! 當 profile frontmatter 有任何 type-B `ENABLE_*` flag 時，voice input
//! pipeline 從「純文字轉換」切到「agent loop」模式：LLM 可以呼叫工具
//! 來執行動作（開網址、開 app、送鍵盤、跑 shell 等），不只能輸出文字。
//!
//! ## 安全
//! - 所有工具都明確由 profile frontmatter 「opt-in」才會暴露給 LLM
//! - `run_shell` 需要 `~/.mori/config.json` 的 `run_shell_whitelist` 才能執行
//! - `open_url` 只接受 `http(s)://` 開頭的 URL，不允許 `file://`、`javascript:` 等

use anyhow::{anyhow, bail, Context as _, Result};
use mori_core::llm::ToolDefinition;
use mori_core::voice_input_profile::VoiceInputFrontmatter;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;

// ─── Tool list 組裝 ──────────────────────────────────────────────────

/// 根據 profile frontmatter 的 ENABLE_* flag 決定要暴露哪些工具給 LLM。
/// 沒任何 type-B flag 時回空 vec，呼叫端可以判斷是否走 agent loop。
pub fn build_tool_list(fm: &VoiceInputFrontmatter) -> Vec<ToolDefinition> {
    let mut tools = Vec::new();

    if fm.enable_open_url {
        tools.push(ToolDefinition {
            name: "open_url".into(),
            description:
                "Open an absolute URL (http:// or https://) in the system default browser. \
                 Use this when the user wants to navigate to a specific webpage or service."
                    .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Absolute URL including scheme (http:// or https://)."
                    }
                },
                "required": ["url"]
            }),
        });
    }

    if fm.enable_open_app {
        tools.push(ToolDefinition {
            name: "open_app".into(),
            description:
                "Launch a locally installed desktop application by name (e.g. \"Firefox\", \
                 \"Code\", \"Slack\"). Searches .desktop entries and launches via gtk-launch."
                    .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "app": {
                        "type": "string",
                        "description": "Application display name or executable name."
                    }
                },
                "required": ["app"]
            }),
        });
    }

    if fm.enable_google_search {
        tools.push(ToolDefinition {
            name: "google_search".into(),
            description: "Open a Google search for the given query in the system browser.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query string." }
                },
                "required": ["query"]
            }),
        });
    }

    if fm.enable_ask_chatgpt {
        tools.push(ToolDefinition {
            name: "ask_chatgpt".into(),
            description: "Open ChatGPT in the browser with a pre-filled prompt.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Prompt to send to ChatGPT." }
                },
                "required": ["prompt"]
            }),
        });
    }

    if fm.enable_ask_gemini {
        tools.push(ToolDefinition {
            name: "ask_gemini".into(),
            description: "Open Google Gemini in the browser with a pre-filled prompt that auto-submits.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Prompt to send to Gemini." }
                },
                "required": ["prompt"]
            }),
        });
    }

    if fm.enable_find_youtube {
        tools.push(ToolDefinition {
            name: "find_youtube".into(),
            description: "Open YouTube search results for the given query.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query string." }
                },
                "required": ["query"]
            }),
        });
    }

    if fm.enable_send_keys {
        tools.push(ToolDefinition {
            name: "send_keys".into(),
            description:
                "Send a keyboard shortcut to the currently focused window. Use for control \
                 shortcuts like Ctrl+S, Ctrl+Shift+P, Alt+Tab. NOT for typing text — use \
                 smart_paste for that. Format: \"Ctrl+S\", \"Ctrl+Shift+P\", \"Alt+F4\"."
                    .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "keys": {
                        "type": "string",
                        "description": "Key combination like \"Ctrl+S\" or \"Ctrl+Shift+V\"."
                    }
                },
                "required": ["keys"]
            }),
        });
    }

    if fm.enable_run_shell {
        tools.push(ToolDefinition {
            name: "run_shell".into(),
            description:
                "Run a whitelisted shell command. Only commands listed in mori config's \
                 `run_shell_whitelist` array can execute. Use for diagnostics, simple status \
                 checks. Do NOT use for destructive operations."
                    .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command to run." }
                },
                "required": ["command"]
            }),
        });
    }

    tools
}

// ─── Tool 執行 ───────────────────────────────────────────────────────

/// 執行 LLM 要求的工具呼叫，回傳給 LLM 看的結果（JSON string）。
/// 失敗也回 JSON（不 bail），讓 LLM 知道工具錯誤可以調整。
pub async fn execute_tool(name: &str, args: Value) -> Result<Value> {
    match name {
        "open_url" => execute_open_url(args),
        "open_app" => execute_open_app(args),
        "google_search" => execute_open_url_with_template(
            &args,
            "query",
            "https://www.google.com/search?q={}",
        ),
        "ask_chatgpt" => execute_open_url_with_template(
            &args,
            "prompt",
            "https://chatgpt.com/?prompt={}",
        ),
        "ask_gemini" => execute_open_url_with_template(
            &args,
            "prompt",
            "https://gemini.google.com/app#autoSubmit=true&prompt={}",
        ),
        "find_youtube" => execute_open_url_with_template(
            &args,
            "query",
            "https://www.youtube.com/results?search_query={}",
        ),
        "send_keys" => execute_send_keys(args),
        "run_shell" => execute_run_shell(args).await,
        other => Ok(json!({ "error": format!("unknown tool '{other}'") })),
    }
}

// ─── 個別工具 ────────────────────────────────────────────────────────

fn execute_open_url(args: Value) -> Result<Value> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("open_url: missing 'url' argument"))?
        .trim();

    // 安全：只接受 http(s)，拒絕 file://、javascript:、data: 等。
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Ok(json!({
            "error": "open_url only accepts http:// or https:// URLs",
            "url": url,
        }));
    }

    tracing::info!(url, "voice-input tool: open_url");
    let status = Command::new("xdg-open")
        .arg(url)
        .status()
        .context("spawn xdg-open")?;

    if status.success() {
        Ok(json!({ "ok": true, "opened": url }))
    } else {
        Ok(json!({ "error": format!("xdg-open exited {status}") }))
    }
}

fn execute_open_url_with_template(args: &Value, key: &str, template: &str) -> Result<Value> {
    let q = args
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("missing '{key}' argument"))?;
    let encoded = urlencode(q);
    let url = template.replace("{}", &encoded);
    execute_open_url(json!({ "url": url }))
}

/// 最小版 URL encoding（RFC 3986 unreserved），不引入額外 crate。
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

fn execute_open_app(args: Value) -> Result<Value> {
    let name = args
        .get("app")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("open_app: missing 'app' argument"))?
        .trim();

    tracing::info!(app = name, "voice-input tool: open_app");

    // 找 .desktop 檔（user-level 和 system-level 都掃）
    let desktop_path = find_desktop_file(name);

    let result = match desktop_path {
        Some(path) => {
            let desktop_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| anyhow!("invalid .desktop path: {}", path.display()))?;
            tracing::info!(desktop_id, "launching via gtk-launch");
            Command::new("gtk-launch")
                .arg(desktop_id)
                .spawn()
                .context("spawn gtk-launch")?;
            Ok(json!({ "ok": true, "launched": desktop_id }))
        }
        None => {
            // Fallback：直接當作 binary 名稱跑（會以 mori 為 parent，少數 GUI 沒問題）
            tracing::warn!(name, "no .desktop file matched, trying as binary name");
            match Command::new(name).spawn() {
                Ok(_) => Ok(json!({ "ok": true, "spawned": name })),
                Err(e) => Ok(json!({
                    "error": format!("no .desktop file and binary spawn failed: {e}"),
                    "searched": name,
                })),
            }
        }
    };
    result
}

/// 搜 `~/.local/share/applications/` 和 `/usr/share/applications/` 找 .desktop 檔，
/// 用 case-insensitive substring 比對 filename / Name= 欄位。回傳第一個符合的路徑。
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
        if let Ok(entries) = std::fs::read_dir(dir) {
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

                // 先 match filename stem（最常見的 case：使用者說「firefox」對應 firefox.desktop）
                if stem.contains(&q) {
                    return Some(path);
                }

                // 退一步比對 Name= 欄位（user 可能說「文字編輯器」對應 org.gnome.TextEditor）
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
    }

    None
}

fn execute_send_keys(args: Value) -> Result<Value> {
    let keys = args
        .get("keys")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("send_keys: missing 'keys' argument"))?
        .trim();

    tracing::info!(keys, "voice-input tool: send_keys");

    let codes = parse_key_combo(keys)?;
    let mut cmd = Command::new("ydotool");
    cmd.arg("key");
    for c in &codes {
        cmd.arg(c);
    }
    let status = cmd.status().context("spawn ydotool")?;

    if status.success() {
        Ok(json!({ "ok": true, "sent": keys }))
    } else {
        Ok(json!({ "error": format!("ydotool exited {status}"), "keys": keys }))
    }
}

/// "Ctrl+Shift+V" → ydotool 序列 ["29:1", "42:1", "47:1", "47:0", "42:0", "29:0"]
fn parse_key_combo(combo: &str) -> Result<Vec<String>> {
    let parts: Vec<&str> = combo.split('+').map(|s| s.trim()).collect();
    if parts.is_empty() {
        bail!("empty key combo");
    }
    let mut codes_in_order: Vec<u16> = Vec::new();
    for p in &parts {
        let code = key_name_to_code(p)
            .ok_or_else(|| anyhow!("unknown key '{}' in combo '{}'", p, combo))?;
        codes_in_order.push(code);
    }
    // 按下順序: M1, M2, ..., Key
    // 釋放順序: Key, ..., M2, M1（反向）
    let mut out: Vec<String> = codes_in_order.iter().map(|c| format!("{c}:1")).collect();
    for c in codes_in_order.iter().rev() {
        out.push(format!("{c}:0"));
    }
    Ok(out)
}

fn key_name_to_code(name: &str) -> Option<u16> {
    // Linux input event codes (subset)
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
        "up" => Some(103),
        "down" => Some(108),
        "left" => Some(105),
        "right" => Some(106),
        "home" => Some(102),
        "end" => Some(107),
        "pageup" => Some(104),
        "pagedown" => Some(109),
        "a" => Some(30), "b" => Some(48), "c" => Some(46), "d" => Some(32),
        "e" => Some(18), "f" => Some(33), "g" => Some(34), "h" => Some(35),
        "i" => Some(23), "j" => Some(36), "k" => Some(37), "l" => Some(38),
        "m" => Some(50), "n" => Some(49), "o" => Some(24), "p" => Some(25),
        "q" => Some(16), "r" => Some(19), "s" => Some(31), "t" => Some(20),
        "u" => Some(22), "v" => Some(47), "w" => Some(17), "x" => Some(45),
        "y" => Some(21), "z" => Some(44),
        "0" => Some(11), "1" => Some(2),  "2" => Some(3),  "3" => Some(4),
        "4" => Some(5),  "5" => Some(6),  "6" => Some(7),  "7" => Some(8),
        "8" => Some(9),  "9" => Some(10),
        "f1" => Some(59),  "f2" => Some(60),  "f3" => Some(61),  "f4" => Some(62),
        "f5" => Some(63),  "f6" => Some(64),  "f7" => Some(65),  "f8" => Some(66),
        "f9" => Some(67),  "f10" => Some(68), "f11" => Some(87), "f12" => Some(88),
        _ => None,
    }
}

async fn execute_run_shell(args: Value) -> Result<Value> {
    let cmd_str = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("run_shell: missing 'command' argument"))?
        .trim()
        .to_string();

    let whitelist = read_shell_whitelist();
    if !whitelist.contains(&cmd_str) {
        tracing::warn!(
            command = %cmd_str,
            whitelist_size = whitelist.len(),
            "run_shell rejected: command not in whitelist",
        );
        return Ok(json!({
            "error": "command not in run_shell_whitelist",
            "command": cmd_str,
            "hint": "add the exact command string to ~/.mori/config.json `run_shell_whitelist`",
        }));
    }

    tracing::info!(command = %cmd_str, "voice-input tool: run_shell (whitelisted)");
    let output = Command::new("sh")
        .arg("-c")
        .arg(&cmd_str)
        .output()
        .context("spawn sh -c")?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok(json!({
        "ok": output.status.success(),
        "exit_code": output.status.code(),
        "stdout": stdout.chars().take(2000).collect::<String>(),
        "stderr": stderr.chars().take(500).collect::<String>(),
    }))
}

fn read_shell_whitelist() -> Vec<String> {
    let path = std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".mori").join("config.json"));
    let Some(path) = path else { return vec![] };
    let Ok(text) = std::fs::read_to_string(&path) else { return vec![] };
    let Ok(json) = serde_json::from_str::<Value>(&text) else { return vec![] };
    json.pointer("/run_shell_whitelist")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencode_basic() {
        assert_eq!(urlencode("hello world"), "hello%20world");
        assert_eq!(urlencode("中文"), "%E4%B8%AD%E6%96%87");
        assert_eq!(urlencode("a+b/c"), "a%2Bb%2Fc");
        assert_eq!(urlencode("foo_bar.baz~qux"), "foo_bar.baz~qux"); // unreserved
    }

    #[test]
    fn parse_key_combo_ctrl_shift_v() {
        let codes = parse_key_combo("Ctrl+Shift+V").unwrap();
        // Press: 29(Ctrl), 42(Shift), 47(V), Release reverse: 47, 42, 29
        assert_eq!(codes, vec!["29:1", "42:1", "47:1", "47:0", "42:0", "29:0"]);
    }

    #[test]
    fn parse_key_combo_single_enter() {
        let codes = parse_key_combo("Enter").unwrap();
        assert_eq!(codes, vec!["28:1", "28:0"]);
    }

    #[test]
    fn parse_key_combo_unknown_fails() {
        let r = parse_key_combo("Ctrl+ZZZ");
        assert!(r.is_err());
    }

    #[test]
    fn build_tool_list_empty_when_no_flags() {
        let fm = VoiceInputFrontmatter::default();
        // 預設只開 enable_smart_paste，沒 type-B → 空 tool list
        let tools = build_tool_list(&fm);
        assert!(tools.is_empty(), "got: {:?}", tools.iter().map(|t| &t.name).collect::<Vec<_>>());
    }

    #[test]
    fn build_tool_list_includes_open_url() {
        let mut fm = VoiceInputFrontmatter::default();
        fm.enable_open_url = true;
        let tools = build_tool_list(&fm);
        assert!(tools.iter().any(|t| t.name == "open_url"));
    }

    #[test]
    fn open_url_rejects_file_scheme() {
        let r = execute_open_url(json!({ "url": "file:///etc/passwd" })).unwrap();
        assert!(r.get("error").is_some(), "should reject file:// — got {r}");
    }

    #[test]
    fn open_url_rejects_javascript_scheme() {
        let r = execute_open_url(json!({ "url": "javascript:alert(1)" })).unwrap();
        assert!(r.get("error").is_some());
    }
}
