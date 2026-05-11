//! Agent profile system — `~/.mori/agent/AGENT-XX.主題.md`。
//!
//! Agent profile 控制當使用者按 `Ctrl+Alt+N` 進入 Mori 模式時：
//! - **provider**：用哪個 LLM 主腦（claude-bash / groq / gemini-cli ...）
//! - **enabled_skills**：暴露哪些 skill 給 Mori 呼叫（空 = 全開）
//! - **system prompt body**：Mori 的人格 / 該情境的態度
//!
//! 對應 VoiceInput profile (`~/.mori/voice_input/USER-XX.md`)，但任務不同：
//! VoiceInput 處理「字」，Agent 處理「事」。

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;

use serde::Deserialize;

use crate::voice_input_profile::SlotSwitchInfo;

// ─── Frontmatter ──────────────────────────────────────────────────────────

/// 從 AGENT-XX.md 的 YAML frontmatter 解析出來的設定。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AgentFrontmatter {
    /// LLM provider（claude-bash / groq / ollama / claude-cli / ...）
    /// None → 用 ~/.mori/config.json 的 provider
    pub provider: Option<String>,
    /// STT provider override
    pub stt_provider: Option<String>,
    /// 此 profile 暴露給 LLM 的 skill 子集（empty / 缺 → 全 SkillRegistry 都暴露）
    pub enabled_skills: Vec<String>,
    /// 啟用 profile body 的 #file: 預處理
    pub enable_read: bool,
    /// 5H: 使用者自訂的 shell skills（依附此 profile，切走就消失）
    pub shell_skills: Vec<ShellSkillDef>,
}

/// 5H: profile 中自訂的 shell skill 定義。每次 profile 切換時動態建立成
/// `Skill` trait 物件註冊到 SkillRegistry 給 LLM 呼叫。
///
/// **安全**：
/// - `command` 是 array，第一個元素是 binary，其餘是 args。**永遠不走 shell**
///   (沒有 `; && | $()` 解析)，所以 LLM 即使把奇怪內容塞進參數也無法 escape。
/// - 參數值用 `{{name}}` 在 array 元素內替換，仍是字面字串。
/// - profile 由使用者撰寫（信任來源），LLM 沒能力動 `command` 的內容。
#[derive(Debug, Clone, Deserialize)]
pub struct ShellSkillDef {
    /// 唯一 skill 名（給 LLM 用，OpenAI tool_calls 的 function name 規則）
    pub name: String,
    /// LLM 看到的描述：什麼時候該呼叫這個工具
    pub description: String,
    /// 參數定義（給 LLM 看 schema，給 mori 看怎麼驗證）
    #[serde(default)]
    pub parameters: HashMap<String, ParamDef>,
    /// 實際執行的 binary + args；元素可含 `{{param_name}}` 替換
    pub command: Vec<String>,
    /// 工作目錄（可選，含 ~ 展開）
    #[serde(default)]
    pub working_dir: Option<String>,
    /// 執行 timeout（秒），預設 30
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// 執行成功後給 LLM 的訊息（可含 `{{stdout}}`），預設「已執行 <name>」
    #[serde(default)]
    pub success_message: Option<String>,
}

fn default_timeout_secs() -> u64 {
    30
}

/// 單一參數的 schema 定義。
#[derive(Debug, Clone, Deserialize)]
pub struct ParamDef {
    /// JSON Schema type：目前只支援 "string"
    #[serde(rename = "type", default = "default_param_type")]
    pub kind: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default: Option<String>,
}

fn default_param_type() -> String {
    "string".to_string()
}

impl AgentFrontmatter {
    /// 給定一個 skill name，這個 profile 是否允許 LLM 呼叫它。
    /// enabled_skills 為空 → 全開（向後相容、預設行為）
    pub fn is_skill_enabled(&self, skill_name: &str) -> bool {
        if self.enabled_skills.is_empty() {
            return true;
        }
        self.enabled_skills.iter().any(|s| s == skill_name)
    }
}

// ─── Profile ──────────────────────────────────────────────────────────────

/// 載入完成的 AGENT-XX.md profile。
#[derive(Debug, Clone)]
pub struct AgentProfile {
    /// 檔名（不含 .md），例如 "AGENT-01.程式助理"
    pub name: String,
    pub frontmatter: AgentFrontmatter,
    /// frontmatter 之後的 prompt 本文（Mori 的人格指示）
    pub body: String,
}

impl AgentProfile {
    /// 取得 LLM provider 顯示名（給 floating widget 用）
    pub fn provider_display(&self) -> String {
        self.frontmatter
            .provider
            .clone()
            .unwrap_or_else(|| "default".to_string())
    }
}

// ─── Parsing ──────────────────────────────────────────────────────────────

pub fn parse_agent_profile(name: &str, content: &str) -> AgentProfile {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return AgentProfile {
            name: name.to_string(),
            frontmatter: AgentFrontmatter::default(),
            body: content.trim().to_string(),
        };
    }
    let after_open = trimmed[3..].trim_start_matches(['\r', '\n']);
    if let Some(close_pos) = after_open.find("\n---") {
        let fm_str = &after_open[..close_pos];
        let body_raw = &after_open[close_pos + 4..];
        let body = body_raw.trim_start_matches(['\r', '\n']).trim().to_string();
        AgentProfile {
            name: name.to_string(),
            frontmatter: parse_agent_frontmatter(fm_str),
            body,
        }
    } else {
        AgentProfile {
            name: name.to_string(),
            frontmatter: AgentFrontmatter::default(),
            body: content.trim().to_string(),
        }
    }
}

fn parse_agent_frontmatter(s: &str) -> AgentFrontmatter {
    // 5H: 改用 serde_yml 解析完整 YAML，支援 shell_skills 之類的巢狀結構。
    match serde_yml::from_str::<AgentFrontmatter>(s) {
        Ok(fm) => fm,
        Err(e) => {
            tracing::warn!(?e, "agent profile frontmatter YAML parse failed, using defaults");
            AgentFrontmatter::default()
        }
    }
}

// ─── File I/O ─────────────────────────────────────────────────────────────

pub fn agent_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(|h| PathBuf::from(h).join(".mori").join("agent"))
        .unwrap_or_else(|_| PathBuf::from(".mori/agent"))
}

/// 讀當前 active agent profile。
/// `active` 檔案 → AGENT-0N.*.md → 找不到 → AGENT.md → 內建預設。
pub fn load_active_agent_profile() -> AgentProfile {
    let dir = agent_dir();
    let active_name = std::fs::read_to_string(dir.join("active"))
        .unwrap_or_default()
        .trim()
        .to_string();

    if !active_name.is_empty() {
        let path = dir.join(format!("{active_name}.md"));
        if let Ok(content) = std::fs::read_to_string(&path) {
            tracing::debug!(profile = %active_name, "agent profile loaded");
            return parse_agent_profile(&active_name, &content);
        }
        tracing::warn!(profile = %active_name, "active agent profile not found, falling back to AGENT.md");
    }

    let default_path = dir.join("AGENT.md");
    if let Ok(content) = std::fs::read_to_string(&default_path) {
        return parse_agent_profile("AGENT", &content);
    }

    tracing::debug!("no AGENT.md found, using built-in default");
    parse_agent_profile("AGENT", DEFAULT_AGENT_MD)
}

/// 5K-2: 列出 agent 目錄裡的 profile,給 tray submenu / picker UI 用。
/// 回傳 `(stem, display_name)`,排序:AGENT.md 第一、AGENT-NN 依數字、其他依字典序。
/// 預設 AGENT.md 用 stem="AGENT"、display="Mori（自由判斷）"。
pub fn list_agent_profiles() -> Vec<(String, String)> {
    let dir = agent_dir();
    let mut entries: Vec<(String, String)> = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flat_map(|d| d.filter_map(|e| e.ok()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".md"))
        .map(|name| {
            let stem = name.trim_end_matches(".md").to_string();
            let display = if stem == "AGENT" {
                "Mori（自由判斷）".to_string()
            } else {
                stem.splitn(3, '.').nth(1).unwrap_or(&stem).to_string()
            };
            (stem, display)
        })
        .collect();
    entries.sort_by(|(a, _), (b, _)| {
        // AGENT.md 永遠第一
        if a == "AGENT" { return std::cmp::Ordering::Less; }
        if b == "AGENT" { return std::cmp::Ordering::Greater; }
        let slot_a = a.strip_prefix("AGENT-").and_then(|s| s.split('.').next()).and_then(|s| s.parse::<u32>().ok());
        let slot_b = b.strip_prefix("AGENT-").and_then(|s| s.split('.').next()).and_then(|s| s.parse::<u32>().ok());
        match (slot_a, slot_b) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.cmp(b),
        }
    });
    entries
}

/// 5K-2: 依 stem 切到 agent profile,給 tray / picker UI 用。
/// stem="AGENT" → AGENT.md 預設(同 switch_agent_slot(0))。
pub fn switch_to_agent_profile(stem: &str) -> Option<SlotSwitchInfo> {
    let dir = agent_dir();
    if stem == "AGENT" {
        let _ = std::fs::remove_file(dir.join("active"));
        let profile = load_active_agent_profile();
        return Some(SlotSwitchInfo {
            profile_name: "Mori（自由判斷）".to_string(),
            llm_provider: profile.frontmatter.provider.unwrap_or_else(|| "default".into()),
        });
    }
    let filename = format!("{stem}.md");
    let path = dir.join(&filename);
    if !path.exists() {
        tracing::warn!(stem, "switch_to_agent_profile: file not found");
        return None;
    }
    let _ = std::fs::write(dir.join("active"), stem);
    let profile_name = stem.splitn(3, '.').nth(1).unwrap_or(stem).to_string();
    let llm_provider = std::fs::read_to_string(&path)
        .ok()
        .map(|content| {
            let profile = parse_agent_profile(stem, &content);
            profile.frontmatter.provider.unwrap_or_else(|| "default".into())
        })
        .unwrap_or_else(|| "default".to_string());
    tracing::info!(profile = stem, llm = %llm_provider, "agent profile switched (by name)");
    Some(SlotSwitchInfo { profile_name, llm_provider })
}

/// Ctrl+Alt+N 切換時：寫 active 檔 + 回傳顯示資訊。
/// slot=0 → AGENT.md（default Mori）；slot 1~9 → 掃 AGENT-0N.* 的第一個。
pub fn switch_agent_slot(n: u8) -> Option<SlotSwitchInfo> {
    let dir = agent_dir();
    if n == 0 {
        // slot 0 = AGENT.md (default)，active 設成空 / 移除
        let _ = std::fs::remove_file(dir.join("active"));
        let profile = load_active_agent_profile();
        return Some(SlotSwitchInfo {
            profile_name: "Mori（自由判斷）".to_string(),
            llm_provider: profile.frontmatter.provider.unwrap_or_else(|| "default".into()),
        });
    }

    let prefix = format!("AGENT-{n:02}");
    let entry = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(&prefix) && name.ends_with(".md"))
        .min();

    if let Some(filename) = entry {
        let stem = filename.trim_end_matches(".md").to_string();
        let _ = std::fs::write(dir.join("active"), &stem);
        let profile_name = stem.splitn(3, '.').nth(1).unwrap_or(&stem).to_string();
        let llm_provider = std::fs::read_to_string(dir.join(&filename))
            .ok()
            .map(|content| {
                let profile = parse_agent_profile(&stem, &content);
                profile
                    .frontmatter
                    .provider
                    .unwrap_or_else(|| "default".into())
            })
            .unwrap_or_else(|| "default".to_string());

        tracing::info!(slot = n, profile = %stem, llm = %llm_provider, "agent profile switched");
        return Some(SlotSwitchInfo { profile_name, llm_provider });
    }

    tracing::debug!(slot = n, "no AGENT-{:02}.* file found", n);
    None
}

/// 首次啟動時建 `~/.mori/agent/` 並寫預設 AGENT.md。冪等：已存在不覆蓋。
pub fn ensure_agent_dir_initialized() {
    let dir = agent_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(?e, path = %dir.display(), "could not create agent dir");
        return;
    }
    let path = dir.join("AGENT.md");
    if path.exists() {
        return;
    }
    if let Err(e) = std::fs::write(&path, DEFAULT_AGENT_MD) {
        tracing::warn!(?e, path = %path.display(), "could not write default AGENT.md");
    } else {
        tracing::info!(path = %path.display(), "created default AGENT.md");
    }
}

/// 給上層用：取得 enabled_skills 的 HashSet，方便 contains 檢查。
pub fn enabled_skills_set(profile: &AgentProfile) -> Option<HashSet<String>> {
    if profile.frontmatter.enabled_skills.is_empty() {
        None // 全開
    } else {
        Some(profile.frontmatter.enabled_skills.iter().cloned().collect())
    }
}

// ─── #file: 預處理（5G-8）─────────────────────────────────────────────────

/// 單檔大小上限（10KB，~3000 中文字 / ~10000 ASCII 字符）
const FILE_INCLUDE_MAX_BYTES: usize = 10 * 1024;
/// 整個 profile body 加總引入檔案的上限（50KB）
const FILE_INCLUDE_TOTAL_MAX_BYTES: usize = 50 * 1024;

/// 處理 profile body 內的 `#file:path` 引用。
/// 找到 `#file:path/to/file` 字串 → 讀對應檔案 → inline 替換為內容。
///
/// 安全：
/// - 只接受 `$HOME` 子樹下的檔案（path canonicalize 後檢查）
/// - 單檔超過 [`FILE_INCLUDE_MAX_BYTES`] 截斷
/// - 全部加總超過 [`FILE_INCLUDE_TOTAL_MAX_BYTES`] 後續忽略
/// - 失敗替換為 `[#file 讀取失敗: ...]` 標記
///
/// 若 `enable` 為 false → 原文不動回傳。
pub fn preprocess_file_includes(body: &str, enable: bool) -> String {
    if !enable {
        return body.to_string();
    }
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return body.to_string(),
    };
    let home_path = std::path::PathBuf::from(&home);

    let mut total_bytes = 0usize;
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '#' && peek_starts_with(&mut chars, "file:") {
            // consume "file:"
            for _ in 0..5 {
                chars.next();
            }
            // collect path 直到 whitespace
            let mut path_str = String::new();
            while let Some(&pc) = chars.peek() {
                if pc.is_whitespace() {
                    break;
                }
                path_str.push(pc);
                chars.next();
            }

            // 處理 path：~ 展開、相對路徑、canonicalize
            let resolved = if let Some(stripped) = path_str.strip_prefix("~/") {
                home_path.join(stripped)
            } else if path_str.starts_with('/') {
                std::path::PathBuf::from(&path_str)
            } else {
                home_path.join(&path_str)
            };

            match read_inline(&resolved, &home_path, total_bytes) {
                Ok((content, bytes)) => {
                    total_bytes += bytes;
                    out.push_str(&format!(
                        "\n[--- 引用檔案 {} ---]\n{content}\n[--- end of {} ---]\n",
                        path_str, path_str
                    ));
                }
                Err(e) => {
                    out.push_str(&format!("[#file 讀取失敗: {} — {}]", path_str, e));
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn peek_starts_with(chars: &mut std::iter::Peekable<std::str::Chars>, pat: &str) -> bool {
    let snapshot: Vec<char> = chars.clone().take(pat.len()).collect();
    snapshot.iter().collect::<String>() == pat
}

fn read_inline(
    path: &std::path::Path,
    home_root: &std::path::Path,
    already_read: usize,
) -> std::result::Result<(String, usize), String> {
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("檔案不存在或無權讀取: {e}"))?;
    if !canonical.starts_with(home_root) {
        return Err(format!(
            "路徑超出 HOME ({}) 子樹，拒絕讀取",
            home_root.display()
        ));
    }
    if already_read >= FILE_INCLUDE_TOTAL_MAX_BYTES {
        return Err(format!(
            "已引入 {} bytes，超過總上限 {} bytes",
            already_read, FILE_INCLUDE_TOTAL_MAX_BYTES
        ));
    }
    let content = std::fs::read_to_string(&canonical).map_err(|e| format!("讀檔失敗: {e}"))?;
    let bytes = content.len();
    let truncated = if bytes > FILE_INCLUDE_MAX_BYTES {
        let mut s = content
            .char_indices()
            .take_while(|(i, _)| *i < FILE_INCLUDE_MAX_BYTES)
            .map(|(_, c)| c)
            .collect::<String>();
        s.push_str(&format!(
            "\n... [檔案截斷於 {} bytes，原檔 {} bytes]",
            FILE_INCLUDE_MAX_BYTES, bytes
        ));
        s
    } else {
        content
    };
    Ok((truncated, bytes.min(FILE_INCLUDE_MAX_BYTES)))
}

// ─── Default file content ────────────────────────────────────────────────

pub const DEFAULT_AGENT_MD: &str = r#"---
# 預設 Mori — Ctrl+Alt+0 啟動
# 5I 起 claude-bash / gemini-bash / codex-bash 也都看得到 action_skill 和
# shell_skill（skill_server 已動態化）。可改成任何支援工具呼叫的 provider。
provider: claude-bash
enable_read: true   # 啟用 #file: 預處理（讓 body 能引用 ~/.mori/corrections.md）

# enabled_skills 留空 = 全 built-in skill 都可用（包含 open_url / open_app
# / send_keys / google_search / ask_chatgpt / ask_gemini / find_youtube 等）
# enabled_skills: [translate, polish, summarize, remember, recall_memory, open_url]
---
你是 Mori，森林精靈、桌面 AI 同伴。

## 共用 STT 校正 + 用詞偏好

#file:~/.mori/corrections.md

## 行為

判斷使用者意圖：
- 純對話（聊天、提問、想法討論）→ 直接回應，floating widget 會顯示
- 想動作（開網址、開 app、查資料）→ 主動呼叫對應 skill
- 兩者皆有 → 動作 + 簡短說明結果

語氣：自然、簡潔、不客套。繁中為主。
有記憶能力（remember / recall_memory），跨 session 記得使用者的偏好。
"#;

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_no_frontmatter() {
        let p = parse_agent_profile("test", "just body");
        assert_eq!(p.body, "just body");
        assert!(p.frontmatter.provider.is_none());
    }

    #[test]
    fn parse_with_provider_and_skills() {
        let content = "---\nprovider: claude-bash\nenabled_skills: [translate, polish, open_url]\n---\nbody";
        let p = parse_agent_profile("test", content);
        assert_eq!(p.frontmatter.provider.as_deref(), Some("claude-bash"));
        assert_eq!(p.frontmatter.enabled_skills, vec!["translate", "polish", "open_url"]);
        assert_eq!(p.body, "body");
    }

    #[test]
    fn parse_enabled_skills_list_serde_yml() {
        // serde_yml 支援 YAML inline list 跟 block list 兩種寫法
        let inline = parse_agent_profile("t", "---\nenabled_skills: [a, b, c]\n---\nbody");
        assert_eq!(inline.frontmatter.enabled_skills, vec!["a", "b", "c"]);

        let block = parse_agent_profile(
            "t",
            "---\nenabled_skills:\n  - a\n  - b\n  - c\n---\nbody",
        );
        assert_eq!(block.frontmatter.enabled_skills, vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_shell_skills_full() {
        let yaml = "---\nshell_skills:\n  - name: gh_pr_list\n    description: List PRs\n    command: [gh, pr, list]\n  - name: ssh_to\n    description: SSH somewhere\n    parameters:\n      host: { type: string, required: true }\n    command: [ssh, \"{{host}}\"]\n    timeout_secs: 60\n---\nbody";
        let p = parse_agent_profile("t", yaml);
        assert_eq!(p.frontmatter.shell_skills.len(), 2);
        assert_eq!(p.frontmatter.shell_skills[0].name, "gh_pr_list");
        assert_eq!(p.frontmatter.shell_skills[0].command, vec!["gh", "pr", "list"]);
        assert_eq!(p.frontmatter.shell_skills[1].name, "ssh_to");
        assert_eq!(p.frontmatter.shell_skills[1].command, vec!["ssh", "{{host}}"]);
        assert_eq!(p.frontmatter.shell_skills[1].timeout_secs, 60);
        let host = p.frontmatter.shell_skills[1].parameters.get("host").unwrap();
        assert!(host.required);
    }

    #[test]
    fn is_skill_enabled_empty_means_all() {
        let fm = AgentFrontmatter::default();
        assert!(fm.is_skill_enabled("anything"));
    }

    #[test]
    fn is_skill_enabled_filters_correctly() {
        let mut fm = AgentFrontmatter::default();
        fm.enabled_skills = vec!["translate".into(), "polish".into()];
        assert!(fm.is_skill_enabled("translate"));
        assert!(fm.is_skill_enabled("polish"));
        assert!(!fm.is_skill_enabled("open_url"));
    }

    #[test]
    fn enable_read_default_false() {
        let fm = AgentFrontmatter::default();
        assert!(!fm.enable_read);
    }

    #[test]
    fn enable_read_parsed() {
        let p = parse_agent_profile(
            "t",
            "---\nenable_read: true\n---\nbody",
        );
        assert!(p.frontmatter.enable_read);
    }

    #[test]
    fn preprocess_file_includes_disabled_returns_original() {
        let body = "hello #file:~/whatever.txt world";
        assert_eq!(preprocess_file_includes(body, false), body);
    }

    #[test]
    fn preprocess_file_includes_handles_missing_file() {
        let body = "ref: #file:~/this-file-definitely-does-not-exist-12345.txt end";
        let out = preprocess_file_includes(body, true);
        assert!(out.contains("[#file 讀取失敗:"));
        assert!(out.contains("end"));
    }

    #[test]
    fn preprocess_file_includes_reads_real_file() {
        // 用 tempfile 在 HOME 子樹下測試成功讀取
        let home = std::env::var("HOME").unwrap_or_default();
        if home.is_empty() {
            return; // skip 沒 HOME 環境（CI 容器等）
        }
        let tmp = std::path::PathBuf::from(&home).join(".mori-test-fileref.txt");
        std::fs::write(&tmp, "FILE_CONTENT_MARKER\n").unwrap();
        let body = format!("see: #file:{} end", tmp.display());
        let out = preprocess_file_includes(&body, true);
        assert!(out.contains("FILE_CONTENT_MARKER"), "got: {out}");
        assert!(out.contains("end"));
        std::fs::remove_file(&tmp).ok();
    }
}
