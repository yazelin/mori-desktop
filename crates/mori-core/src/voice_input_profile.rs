//! Voice input profile system (5J 單層 profile + Rust context 注入).
//!
//! ## 檔案結構
//! ```text
//! ~/.mori/voice_input/
//!   USER-01.*.md   ← Alt+1 對應
//!   ...
//!   active         ← 一行，存當前選中的 profile 名稱（不含 .md）
//! ```
//!
//! ## 5J 之後（簡化）
//! - 不再有 SYSTEM.md 模板與 `{{CONTEXT.*}}` 佔位符
//! - profile body = 純人格 / cleanup 指示（使用者寫的）
//! - 時間 / 視窗 / 剪貼簿 / OS 等動態 context 由 mori-tauri 的 `build_context_section()`
//!   在 LLM call 之前拼到 system prompt 後面
//! - ZeroType 相容鍵（ZEROTYPE_AIPROMPT_* / ENABLE_*）仍可解析，但建議改用 mori-native
//!   的 `provider:` / `stt_provider:` 寫法

use std::path::PathBuf;

use crate::voice_cleanup::CleanupLevel;

// ─── Context ──────────────────────────────────────────────────────────────

/// 熱鍵觸發瞬間 + STT 完成後收集的系統資訊，用來填入 SYSTEM.md 模板。
#[derive(Debug, Clone, Default)]
pub struct VoiceInputContext {
    pub current_time: String,
    pub today_date: String,
    pub os: String,
    /// 活躍視窗的 process 名稱（xdotool → /proc/pid/comm），失敗時為空字串
    pub process_name: String,
    /// process_name 的人類可讀版（目前相同，未來可做 display name 映射）
    pub active_app: String,
    /// 活躍視窗標題（xdotool getwindowname），失敗時為空字串
    pub window_title: String,
    /// 剪貼簿內容（1KB cap），無內容時為空字串
    pub clipboard: String,
    /// X11 PRIMARY selection（滑鼠反白文字，1.5KB cap），無時為空字串
    pub selected_text: String,
}

impl VoiceInputContext {
    pub fn new_now() -> Self {
        let now = chrono::Local::now();
        Self {
            current_time: now.format("%Y-%m-%d %H:%M:%S").to_string(),
            today_date: now.format("%Y-%m-%d").to_string(),
            os: std::env::consts::OS.to_string(),
            ..Default::default()
        }
    }
}

// ─── Frontmatter ──────────────────────────────────────────────────────────

/// 從 USER-*.md 的 YAML frontmatter 解析出來的設定。
/// 未知鍵靜默忽略，ZeroType profile 丟進來不會炸。
#[derive(Debug, Clone)]
pub struct VoiceInputFrontmatter {
    // ── ZeroType API 相容鍵 ─────────────────────────────────────────
    pub zerotype_api_base: Option<String>,
    pub zerotype_api_key_env: Option<String>,
    pub zerotype_model: Option<String>,
    pub zerotype_model_effort: Option<String>,

    // ── Type A flags（不需要 agent loop）──────────────────────────
    /// 處理完貼回（預設 true）
    pub enable_smart_paste: bool,
    /// 貼回後模擬 Enter（預設 false）
    pub enable_auto_enter: bool,

    // ── Type B flags（有任何一個為 true → 走 agent loop）──────────
    pub enable_send_keys: bool,
    pub enable_open_url: bool,
    pub enable_open_app: bool,
    pub enable_google_search: bool,
    pub enable_ask_chatgpt: bool,
    pub enable_ask_gemini: bool,
    pub enable_find_youtube: bool,
    pub enable_read: bool,
    pub enable_run_shell: bool,

    // ── mori 專屬鍵 ───────────────────────────────────────────────
    /// mori 具名 provider 快捷（groq / ollama / claude-bash / ...）
    /// 若與 zerotype_api_base 並存，以 zerotype_api_base 為準（保留原始意圖）
    pub provider: Option<String>,
    /// 覆蓋此 profile 的 STT provider（groq / whisper-local）
    pub stt_provider: Option<String>,
    /// 覆蓋此 profile 的 paste 快捷鍵：
    /// - `ctrl_v`（一般 app：VS Code / 瀏覽器 / 文字編輯器）
    /// - `ctrl_shift_v`（terminal：gnome-terminal / kitty / Claude Code 等 CLI 工具）
    /// 沒設時自動偵測 process name；偵測失敗（Wayland 原生視窗）退到 ctrl_v。
    pub paste_shortcut: Option<PasteShortcut>,
    /// 覆蓋全域 cleanup_level
    pub cleanup_level: Option<CleanupLevel>,
}

/// 貼回游標時用哪組 ydotool 按鍵。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteShortcut {
    /// `Ctrl+V` — 大部分 GUI app（VS Code、瀏覽器、文字編輯器、聊天 app）
    CtrlV,
    /// `Ctrl+Shift+V` — terminal app（在 terminal 裡 Ctrl+V 是 literal ^V）
    CtrlShiftV,
}

impl Default for VoiceInputFrontmatter {
    fn default() -> Self {
        Self {
            zerotype_api_base: None,
            zerotype_api_key_env: None,
            zerotype_model: None,
            zerotype_model_effort: None,
            enable_smart_paste: true,
            enable_auto_enter: false,
            enable_send_keys: false,
            enable_open_url: false,
            enable_open_app: false,
            enable_google_search: false,
            enable_ask_chatgpt: false,
            enable_ask_gemini: false,
            enable_find_youtube: false,
            enable_read: false,
            enable_run_shell: false,
            provider: None,
            stt_provider: None,
            paste_shortcut: None,
            cleanup_level: None,
        }
    }
}

impl VoiceInputFrontmatter {
    /// 是否有任何 Type B flag 開啟（需要 agent loop）
    pub fn has_type_b_flags(&self) -> bool {
        self.enable_send_keys
            || self.enable_open_url
            || self.enable_open_app
            || self.enable_google_search
            || self.enable_ask_chatgpt
            || self.enable_ask_gemini
            || self.enable_find_youtube
            || self.enable_read
            || self.enable_run_shell
    }

    /// 決定實際使用的 provider 名稱或 ZeroType API 設定。
    /// ZeroType API 設定優先（保留原始意圖），mori provider: 作為 fallback。
    pub fn resolved_provider(&self) -> ResolvedProvider {
        if let (Some(base), Some(key_env)) =
            (&self.zerotype_api_base, &self.zerotype_api_key_env)
        {
            let api_key = resolve_api_key(key_env);
            return ResolvedProvider::OpenAiCompat {
                api_base: base.clone(),
                api_key,
                model: self.zerotype_model.clone().unwrap_or_default(),
            };
        }
        if let Some(name) = &self.provider {
            return ResolvedProvider::Named(name.clone());
        }
        ResolvedProvider::Default
    }
}

/// profile 解析出來的 provider 決策。
#[derive(Debug, Clone)]
pub enum ResolvedProvider {
    /// 使用 ZeroType 的 openai-compatible 設定（ZEROTYPE_AIPROMPT_* 鍵）
    OpenAiCompat {
        api_base: String,
        api_key: String,
        model: String,
    },
    /// 使用 mori 的具名 provider（groq / ollama / claude-bash / ...）
    Named(String),
    /// 沒有設定，交給呼叫端的 routing 決定
    Default,
}

impl ResolvedProvider {
    /// 給 UI 顯示用的簡短名稱（groq / gemini / ollama / claude-bash / ...）
    pub fn display_name(&self) -> String {
        match self {
            ResolvedProvider::OpenAiCompat { api_base, model, .. } => {
                if api_base.contains("googleapis") {
                    // Gemini — 從 model 取最後一段
                    model.split('-').next().unwrap_or("gemini").to_string()
                } else if api_base.contains("openai.com") {
                    "openai".to_string()
                } else if api_base.contains("azure") {
                    "azure".to_string()
                } else if api_base.contains("groq") {
                    "groq".to_string()
                } else {
                    "api".to_string()
                }
            }
            ResolvedProvider::Named(name) => name.clone(),
            ResolvedProvider::Default => "default".to_string(),
        }
    }
}

// ─── Profile ──────────────────────────────────────────────────────────────

/// 載入完成的 USER-*.md profile。
#[derive(Debug, Clone)]
pub struct VoiceInputProfile {
    /// 檔名（不含 .md），例如 "USER-01.朋友閒聊"
    pub name: String,
    pub frontmatter: VoiceInputFrontmatter,
    /// frontmatter 之後的 prompt 本文（注入至 {{CONTEXT.USER_PROMPT}}）
    pub body: String,
}

impl VoiceInputProfile {
    pub fn cleanup_level_effective(&self) -> CleanupLevel {
        self.frontmatter
            .cleanup_level
            .unwrap_or_else(crate::voice_cleanup::read_cleanup_level)
    }
}

// ─── Parsing ──────────────────────────────────────────────────────────────

/// 從檔名 + 檔案內容解析出 VoiceInputProfile。
pub fn parse_profile(name: &str, content: &str) -> VoiceInputProfile {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return VoiceInputProfile {
            name: name.to_string(),
            frontmatter: VoiceInputFrontmatter::default(),
            body: content.trim().to_string(),
        };
    }

    // 跳過開頭的 ---（含換行）
    let after_open = trimmed[3..].trim_start_matches(['\r', '\n']);

    // 找結尾的 ---
    if let Some(close_pos) = after_open.find("\n---") {
        let fm_str = &after_open[..close_pos];
        let body_raw = &after_open[close_pos + 4..];
        let body = body_raw.trim_start_matches(['\r', '\n']).trim().to_string();
        VoiceInputProfile {
            name: name.to_string(),
            frontmatter: parse_frontmatter(fm_str),
            body,
        }
    } else {
        VoiceInputProfile {
            name: name.to_string(),
            frontmatter: VoiceInputFrontmatter::default(),
            body: content.trim().to_string(),
        }
    }
}

fn parse_frontmatter(s: &str) -> VoiceInputFrontmatter {
    let mut fm = VoiceInputFrontmatter::default();
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(colon) = line.find(':') else { continue };
        let key = line[..colon].trim();
        let value = line[colon + 1..].trim();

        match key {
            "ZEROTYPE_AIPROMPT_API_BASE" => fm.zerotype_api_base = non_empty(value),
            "ZEROTYPE_AIPROMPT_API_KEY_ENV" => fm.zerotype_api_key_env = non_empty(value),
            "ZEROTYPE_AIPROMPT_MODEL" => fm.zerotype_model = non_empty(value),
            "ZEROTYPE_AIPROMPT_MODEL_EFFORT" => fm.zerotype_model_effort = non_empty(value),
            "ENABLE_SMART_PASTE" => fm.enable_smart_paste = parse_bool(value),
            "ENABLE_AUTO_ENTER" => fm.enable_auto_enter = parse_bool(value),
            "ENABLE_SEND_KEYS" => fm.enable_send_keys = parse_bool(value),
            "ENABLE_OPEN_URL" => fm.enable_open_url = parse_bool(value),
            "ENABLE_OPEN_APP" => fm.enable_open_app = parse_bool(value),
            "ENABLE_GOOGLE_SEARCH" => fm.enable_google_search = parse_bool(value),
            "ENABLE_ASK_CHATGPT" => fm.enable_ask_chatgpt = parse_bool(value),
            "ENABLE_ASK_GEMINI" => fm.enable_ask_gemini = parse_bool(value),
            "ENABLE_FIND_YOUTUBE" => fm.enable_find_youtube = parse_bool(value),
            "ENABLE_READ" => fm.enable_read = parse_bool(value),
            "ENABLE_RUN_SHELL" => fm.enable_run_shell = parse_bool(value),
            "provider" => fm.provider = non_empty(value),
            "stt_provider" => fm.stt_provider = non_empty(value),
            "paste_shortcut" => {
                fm.paste_shortcut = match value.to_lowercase().replace([' ', '-'], "_").as_str() {
                    "ctrl_v" | "ctrl+v" => Some(PasteShortcut::CtrlV),
                    "ctrl_shift_v" | "ctrl+shift+v" => Some(PasteShortcut::CtrlShiftV),
                    _ => None,
                };
            }
            "cleanup_level" => {
                fm.cleanup_level = match value {
                    "smart" => Some(CleanupLevel::Smart),
                    "minimal" => Some(CleanupLevel::Minimal),
                    "none" => Some(CleanupLevel::None),
                    _ => None,
                }
            }
            _ => {} // 未知鍵靜默忽略，ZeroType profile 相容
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

/// API key 解析順序：
/// 1. OS 環境變數（`std::env::var(key_env_name)`）
/// 2. `~/.mori/config.json` 的 `api_keys.<key_env_name>`
///
/// 這讓使用者不需要設 Linux GUI 環境變數，直接在 config.json 裡管理 key：
/// ```json
/// { "api_keys": { "GEMINI_API_KEY": "AIza..." } }
/// ```
fn resolve_api_key(key_env_name: &str) -> String {
    // 1. 先試 OS env var
    if let Ok(val) = std::env::var(key_env_name) {
        if !val.is_empty() {
            return val;
        }
    }
    // 2. fallback 到 ~/.mori/config.json api_keys.<name>
    let key = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .and_then(|h| {
            let path = std::path::PathBuf::from(h).join(".mori").join("config.json");
            let text = std::fs::read_to_string(path).ok()?;
            let json: serde_json::Value = serde_json::from_str(&text).ok()?;
            json.pointer(&format!("/api_keys/{key_env_name}"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default();

    if key.is_empty() {
        tracing::warn!(
            key_env = key_env_name,
            "API key not found in env or config.json api_keys — \
             add to ~/.mori/config.json: {{\"api_keys\": {{\"{key_env_name}\": \"your_key\"}}}}"
        );
    }
    key
}

// ─── File I/O ─────────────────────────────────────────────────────────────

pub fn voice_input_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(|h| PathBuf::from(h).join(".mori").join("voice_input"))
        .unwrap_or_else(|_| PathBuf::from(".mori/voice_input"))
}

/// 讀當前 active profile。
/// active 檔案 → USER-0N.*.md → 找不到 → fallback 用內建最小 profile。
pub fn load_active_profile() -> VoiceInputProfile {
    let dir = voice_input_dir();
    let active_name = std::fs::read_to_string(dir.join("active"))
        .unwrap_or_default()
        .trim()
        .to_string();

    if !active_name.is_empty() {
        let path = dir.join(format!("{active_name}.md"));
        if let Ok(content) = std::fs::read_to_string(&path) {
            tracing::debug!(profile = %active_name, "voice input profile loaded");
            return parse_profile(&active_name, &content);
        }
        tracing::warn!(profile = %active_name, "active profile file not found, using built-in fallback");
    }

    tracing::debug!("no active voice_input profile, using built-in fallback");
    parse_profile("FALLBACK", FALLBACK_PROFILE_MD)
}

/// Alt+N 切換時回傳的資訊（供 floating widget 顯示）。
#[derive(Debug, Clone)]
pub struct SlotSwitchInfo {
    /// 顯示名稱，例如 "朋友閒聊"
    pub profile_name: String,
    /// LLM provider 顯示名稱，例如 "groq" / "gemini"
    pub llm_provider: String,
}

impl SlotSwitchInfo {
    /// 組合成 floating label 文字，例如 "朋友閒聊 · groq"
    pub fn label(&self) -> String {
        format!("{} · {}", self.profile_name, self.llm_provider)
    }
}

/// Alt+N 切換時：寫 active 檔案 + 回傳顯示資訊。
pub fn switch_to_slot(n: u8) -> Option<SlotSwitchInfo> {
    let dir = voice_input_dir();
    let prefix = format!("USER-{n:02}");
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

        // 讀取 profile 以取得 LLM provider 顯示名稱
        let llm_provider = std::fs::read_to_string(dir.join(&filename))
            .ok()
            .map(|content| {
                let profile = parse_profile(&stem, &content);
                profile.frontmatter.resolved_provider().display_name()
            })
            .unwrap_or_else(|| "default".to_string());

        tracing::info!(slot = n, profile = %stem, llm = %llm_provider, "voice input profile switched");
        return Some(SlotSwitchInfo { profile_name, llm_provider });
    }

    tracing::debug!(slot = n, "no profile file found for slot");
    None
}

/// 首次啟動時建 `~/.mori/voice_input/` 目錄。
/// 5J 起不再生成預設 SYSTEM.md / USER.md（context 注入改由 Rust 統一處理，
/// fallback 用內建 FALLBACK_PROFILE_MD 常數，使用者不會看到該 fallback）。
pub fn ensure_voice_input_dir_initialized() {
    let dir = voice_input_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(?e, path = %dir.display(), "could not create voice_input dir");
    }
}

// ─── Built-in fallback ────────────────────────────────────────────────────

/// 使用者未設任何 voice_input profile 時的內部 fallback。
/// 不寫到磁碟，只在 `load_active_profile()` 找不到檔案時臨時 parse。
const FALLBACK_PROFILE_MD: &str = r#"你是 mori 語音輸入助理。把 STT 輸出的純文字做最小幅度的繁中（台灣用語）校正：
修錯字、補標點、保留原意。只輸出處理後的純文字，不要解釋。"#;

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_profile_no_frontmatter() {
        let p = parse_profile("test", "Hello world");
        assert_eq!(p.name, "test");
        assert_eq!(p.body, "Hello world");
        assert!(p.frontmatter.enable_smart_paste); // default true
    }

    #[test]
    fn parse_profile_with_frontmatter() {
        let content = "---\nENABLE_AUTO_ENTER: true\nENABLE_SMART_PASTE: false\n---\nHello";
        let p = parse_profile("test", content);
        assert!(p.frontmatter.enable_auto_enter);
        assert!(!p.frontmatter.enable_smart_paste);
        assert_eq!(p.body, "Hello");
    }

    #[test]
    fn parse_profile_zerotype_api_keys() {
        let content = "---\nZEROTYPE_AIPROMPT_API_BASE: https://example.com/v1\nZEROTYPE_AIPROMPT_API_KEY_ENV: MY_KEY\nZEROTYPE_AIPROMPT_MODEL: gpt-4\n---\nPrompt body";
        let p = parse_profile("test", content);
        assert_eq!(p.frontmatter.zerotype_api_base.as_deref(), Some("https://example.com/v1"));
        assert_eq!(p.frontmatter.zerotype_api_key_env.as_deref(), Some("MY_KEY"));
        assert_eq!(p.frontmatter.zerotype_model.as_deref(), Some("gpt-4"));
        assert_eq!(p.body, "Prompt body");
    }

    #[test]
    fn parse_profile_mori_provider() {
        let content = "---\nprovider: ollama\ncleanup_level: minimal\n---\nbody";
        let p = parse_profile("test", content);
        assert_eq!(p.frontmatter.provider.as_deref(), Some("ollama"));
        assert_eq!(p.frontmatter.cleanup_level, Some(CleanupLevel::Minimal));
    }

    #[test]
    fn resolved_provider_zerotype_wins_over_mori() {
        let content = "---\nZEROTYPE_AIPROMPT_API_BASE: https://api.example.com\nZEROTYPE_AIPROMPT_API_KEY_ENV: MY_KEY\nprovider: groq\n---\nbody";
        let p = parse_profile("test", content);
        // ZEROTYPE_AIPROMPT_* 優先於 provider:
        assert!(matches!(p.frontmatter.resolved_provider(), ResolvedProvider::OpenAiCompat { .. }));
    }

    #[test]
    fn resolved_provider_mori_named_when_no_zerotype() {
        let content = "---\nprovider: groq\n---\nbody";
        let p = parse_profile("test", content);
        assert!(matches!(p.frontmatter.resolved_provider(), ResolvedProvider::Named(name) if name == "groq"));
    }

    #[test]
    fn has_type_b_flags_false_by_default() {
        let fm = VoiceInputFrontmatter::default();
        assert!(!fm.has_type_b_flags());
    }

    #[test]
    fn has_type_b_flags_true_when_send_keys() {
        let content = "---\nENABLE_SEND_KEYS: true\n---\nbody";
        let p = parse_profile("test", content);
        assert!(p.frontmatter.has_type_b_flags());
    }

    #[test]
    fn switch_to_slot_returns_display_name() {
        // 這個測試需要真實 fs，只做 unit 解析邏輯
        // 驗證 stem 拆法
        let stem = "USER-01.朋友閒聊";
        let display = stem.splitn(3, '.').nth(1).unwrap_or(stem).to_string();
        assert_eq!(display, "朋友閒聊");
    }
}
