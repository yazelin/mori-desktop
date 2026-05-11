//! Agent profile system — `~/.mori/agent/AGENT-XX.主題.md`。
//!
//! Agent profile 控制當使用者按 `Ctrl+Alt+N` 進入 Mori 模式時：
//! - **provider**：用哪個 LLM 主腦（claude-bash / groq / gemini-cli ...）
//! - **enabled_skills**：暴露哪些 skill 給 Mori 呼叫（空 = 全開）
//! - **system prompt body**：Mori 的人格 / 該情境的態度
//!
//! 對應 VoiceInput profile (`~/.mori/voice_input/USER-XX.md`)，但任務不同：
//! VoiceInput 處理「字」，Agent 處理「事」。

use std::collections::HashSet;
use std::path::PathBuf;

use crate::voice_input_profile::SlotSwitchInfo;

// ─── Frontmatter ──────────────────────────────────────────────────────────

/// 從 AGENT-XX.md 的 YAML frontmatter 解析出來的設定。
#[derive(Debug, Clone, Default)]
pub struct AgentFrontmatter {
    /// LLM provider（claude-bash / groq / ollama / claude-cli / ...）
    /// None → 用 ~/.mori/config.json 的 default_provider
    pub provider: Option<String>,
    /// STT provider override
    pub stt_provider: Option<String>,
    /// 此 profile 暴露給 LLM 的 skill 子集（empty / 缺 → 全 SkillRegistry 都暴露）
    pub enabled_skills: Vec<String>,
    /// 啟用 profile body 的 #file: 預處理
    pub enable_read: bool,
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
    let mut fm = AgentFrontmatter::default();
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(colon) = line.find(':') else { continue };
        let key = line[..colon].trim().to_lowercase();
        let value = line[colon + 1..].trim();

        match key.as_str() {
            "provider" => fm.provider = non_empty(value),
            "stt_provider" => fm.stt_provider = non_empty(value),
            "enable_read" => fm.enable_read = parse_bool(value),
            "enabled_skills" => {
                fm.enabled_skills = parse_list(value);
            }
            _ => {} // 未知鍵靜默忽略
        }
    }
    fm
}

fn parse_bool(s: &str) -> bool {
    matches!(s.to_lowercase().as_str(), "true" | "yes" | "1")
}

fn non_empty(s: &str) -> Option<String> {
    if s.is_empty() { None } else { Some(s.to_string()) }
}

/// 解析 YAML 風格的 inline list：`[a, b, c]` 或 `a, b, c`。
fn parse_list(s: &str) -> Vec<String> {
    let trimmed = s.trim().trim_start_matches('[').trim_end_matches(']');
    if trimmed.is_empty() {
        return vec![];
    }
    trimmed
        .split(',')
        .map(|item| item.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect()
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

// ─── Default file content ────────────────────────────────────────────────

pub const DEFAULT_AGENT_MD: &str = r#"---
# 預設 Mori — Ctrl+Alt+0 啟動
# provider 留空 = 用 ~/.mori/config.json 的 default_provider
# enabled_skills 留空 = 全 skill 可用

# provider: claude-bash
# enabled_skills: [translate, polish, summarize, remember, recall_memory, open_url]
---
你是 Mori，森林精靈、桌面 AI 同伴。

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
    fn parse_list_inline_and_quoted() {
        assert_eq!(parse_list("[a, b, c]"), vec!["a", "b", "c"]);
        assert_eq!(parse_list("a, b, c"), vec!["a", "b", "c"]);
        assert_eq!(parse_list("[\"open_url\", \"send_keys\"]"), vec!["open_url", "send_keys"]);
        assert!(parse_list("[]").is_empty());
        assert!(parse_list("").is_empty());
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
}
