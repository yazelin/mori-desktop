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
//!
//! ## 5N 之後（key 大小寫整合）
//! - Frontmatter key 全部 case-insensitive：`enable_read` 跟 `ENABLE_READ` 視為等價，
//!   `provider` 跟 `Provider` 也行。canonical 寫法是 lowercase snake_case（跟
//!   Rust field 名一致）。
//! - 過去 voice profile 內 `enable_read: true` 因 parser 只認 SCREAMING_SNAKE 被
//!   silently 忽略 — 5N 修好了，會真的觸發 `#file:` 預處理。
//!
//! ## 5N+ 之後（過渡碼移除）
//! - **ZEROTYPE_AIPROMPT_\* / `ResolvedProvider::OpenAiCompat` 整套移除**:自訂
//!   OpenAI-compat 端點請寫 `provider: <custom-name>` + `~/.mori/config.json`
//!   `providers.<custom-name>`(`api_base` + `api_key_env` + `model`)。詳見
//!   `docs/providers.html`「進階:自訂 OpenAI-compat 端點」段。

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
/// 未知鍵靜默忽略,不熟悉的鍵丟進來不會炸。
#[derive(Debug, Clone)]
pub struct VoiceInputFrontmatter {
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
    /// mori 具名 provider 快捷（groq / ollama / claude-bash / ... / 自訂 OpenAI-compat）。
    /// 自訂端點要在 `~/.mori/config.json` `providers.<name>` 設 api_base / api_key_env / model。
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
    /// 5E-3:cleanup LLM call 之前注入哪些 memory type 進 system prompt(read-only)。
    /// 用來給 VoiceInput 提供「校正詞庫 / 專有名詞 / 個人慣用語」之類 context,
    /// 例 `[voice_dict]`。Memory 寫入仍只走 Agent 模式。
    ///
    /// 語意:
    /// - `None` → fallback 到 config.json `voice_input.inject_memory_types`
    /// - `Some(vec![])` → 強制不 inject(即使 config 全域有設)
    /// - `Some([...])` → 用 profile 自己的清單
    pub inject_memory_types: Option<Vec<String>>,
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
            inject_memory_types: None,
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

    /// 決定實際使用的 provider 名稱。`provider:` 有設用具名;沒設交給 routing。
    pub fn resolved_provider(&self) -> ResolvedProvider {
        if let Some(name) = &self.provider {
            return ResolvedProvider::Named(name.clone());
        }
        ResolvedProvider::Default
    }
}

/// profile 解析出來的 provider 決策。
#[derive(Debug, Clone)]
pub enum ResolvedProvider {
    /// 使用 mori 的具名 provider(groq / ollama / claude-bash / ... / 自訂 OpenAI-compat)
    Named(String),
    /// 沒有設定,交給呼叫端的 routing 決定
    Default,
}

impl ResolvedProvider {
    /// 給 UI 顯示用的簡短名稱(groq / gemini / ollama / claude-bash / 自訂 name)
    pub fn display_name(&self) -> String {
        match self {
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
    // 5N: 偵測 SCREAMING_SNAKE 舊寫法用，end-of-loop 統一 warn 一次。
    let mut deprecated_uppercase: Vec<String> = Vec::new();
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(colon) = line.find(':') else { continue };
        let raw_key = line[..colon].trim();
        let value = line[colon + 1..].trim();
        // 5N: key 全 case-insensitive — `enable_read` / `ENABLE_READ` 視為等價。
        // canonical 是 lowercase snake_case;SCREAMING_SNAKE 為過渡期 deprecated alias。
        let key = raw_key.to_ascii_lowercase();
        if raw_key.chars().any(|c| c.is_ascii_uppercase()) {
            deprecated_uppercase.push(raw_key.to_string());
        }

        match key.as_str() {
            // ── enable flags(canonical: lowercase enable_X)─────────────────
            "enable_smart_paste" => fm.enable_smart_paste = parse_bool(value),
            "enable_auto_enter" => fm.enable_auto_enter = parse_bool(value),
            "enable_send_keys" => fm.enable_send_keys = parse_bool(value),
            "enable_open_url" => fm.enable_open_url = parse_bool(value),
            "enable_open_app" => fm.enable_open_app = parse_bool(value),
            "enable_google_search" => fm.enable_google_search = parse_bool(value),
            "enable_ask_chatgpt" => fm.enable_ask_chatgpt = parse_bool(value),
            "enable_ask_gemini" => fm.enable_ask_gemini = parse_bool(value),
            "enable_find_youtube" => fm.enable_find_youtube = parse_bool(value),
            "enable_read" => fm.enable_read = parse_bool(value),
            "enable_run_shell" => fm.enable_run_shell = parse_bool(value),
            // ── mori 原生鍵 ────────────────────────────────────────────
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
            // 5E-3: inline array 寫法 `[voice_dict, glossary]`(hand-rolled
            // parser 不支援 YAML block list,只接 inline)。空 `[]` 表「強制不
            // inject」,跟「沒寫這行」(走 config fallback)語意不同。
            "inject_memory_types" => {
                fm.inject_memory_types = Some(parse_inline_string_array(value));
            }
            _ => {} // 未知鍵靜默忽略
        }
    }
    if !deprecated_uppercase.is_empty() {
        tracing::warn!(
            keys = ?deprecated_uppercase,
            "voice profile frontmatter 用了 SCREAMING_SNAKE 寫法,canonical 是 \
             lowercase snake_case — 例 ENABLE_READ → enable_read。下版可能移除大寫接受。",
        );
    }
    fm
}

fn parse_bool(s: &str) -> bool {
    matches!(s.to_lowercase().as_str(), "true" | "yes" | "1")
}

fn non_empty(s: &str) -> Option<String> {
    if s.is_empty() { None } else { Some(s.to_string()) }
}

/// 5E-3:解析 inline YAML array 字串 `[a, b, "c"]` → `Vec<String>`。
/// 支援 quote(`"x"` / `'x'`)+ 空白容忍。不支援 block list(`- a` 多行)— 5N
/// hand-rolled line-parser 跨多行成本太高,沒這需求。
fn parse_inline_string_array(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    inner
        .split(',')
        .map(|tok| tok.trim().trim_matches(|c| c == '"' || c == '\''))
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}


// ─── 5E-3: VoiceInput memory inject 設定 ─────────────────────────────────

/// 從 `~/.mori/config.json` 讀 `voice_input.inject_memory_types`。沒設或失敗 → 空 vec。
/// 預期 JSON array of string,例 `["voice_dict"]`。
pub fn read_config_inject_memory_types() -> Vec<String> {
    let path = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".mori").join("config.json"));
    let Some(path) = path else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    json.pointer("/voice_input/inject_memory_types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// 5E-3:解析 profile → 一個 `Vec<MemoryType>`(經過 fallback chain)。
/// - profile.frontmatter.inject_memory_types = `Some(v)` → 用 v(空 = 強制不 inject)
/// - profile = `None` → 讀 config.json `voice_input.inject_memory_types`
/// - 都沒 → 空 vec
pub fn resolve_inject_memory_types(profile: &VoiceInputProfile) -> Vec<crate::memory::MemoryType> {
    let names = match &profile.frontmatter.inject_memory_types {
        Some(v) => v.clone(),
        None => read_config_inject_memory_types(),
    };
    names
        .iter()
        .map(|s| crate::memory::MemoryType::parse(s))
        .collect()
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

/// 5K-2: 列出 voice_input 目錄裡所有 USER-*.md profile,給 tray submenu / picker UI 用。
/// 回傳 `(filename_stem, display_name)`,display_name 取檔名第二段(去 USER-XX. 前綴 + .md 後綴)。
/// 排序:有 slot prefix(USER-00 ~ USER-99)的依 slot 數字升序,其餘依檔名字典序。
pub fn list_voice_profiles() -> Vec<(String, String)> {
    let dir = voice_input_dir();
    let mut entries: Vec<(String, String)> = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flat_map(|d| d.filter_map(|e| e.ok()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("USER-") && name.ends_with(".md"))
        .map(|name| {
            let stem = name.trim_end_matches(".md").to_string();
            let display = stem.splitn(3, '.').nth(1).unwrap_or(&stem).to_string();
            (stem, display)
        })
        .collect();
    entries.sort_by(|(a, _), (b, _)| {
        // USER-NN.* → slot 數字優先;沒數字的擺後面
        let slot_a = a.strip_prefix("USER-").and_then(|s| s.split('.').next()).and_then(|s| s.parse::<u32>().ok());
        let slot_b = b.strip_prefix("USER-").and_then(|s| s.split('.').next()).and_then(|s| s.parse::<u32>().ok());
        match (slot_a, slot_b) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.cmp(b),
        }
    });
    entries
}

/// 5K-2: 依檔名 stem(不含 .md)切到特定 profile,給 tray / picker UI 用。
/// 比 `switch_to_slot` 通用 — slot 編號之外的命名 profile 也能切。
pub fn switch_to_profile(stem: &str) -> Option<SlotSwitchInfo> {
    let dir = voice_input_dir();
    let filename = format!("{stem}.md");
    let path = dir.join(&filename);
    if !path.exists() {
        tracing::warn!(stem, "switch_to_profile: file not found");
        return None;
    }
    let _ = std::fs::write(dir.join("active"), stem);
    let profile_name = stem.splitn(3, '.').nth(1).unwrap_or(stem).to_string();
    let llm_provider = std::fs::read_to_string(&path)
        .ok()
        .map(|content| {
            let profile = parse_profile(stem, &content);
            profile.frontmatter.resolved_provider().display_name()
        })
        .unwrap_or_else(|| "default".to_string());
    tracing::info!(profile = stem, llm = %llm_provider, "voice input profile switched (by name)");
    Some(SlotSwitchInfo { profile_name, llm_provider })
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
    fn parse_profile_lowercase_enable_keys() {
        // 5N: lowercase canonical 寫法應該正常被認識(過去因 parser 只認 SCREAMING
        // 被默默忽略,所有 USER-XX profile 的 `enable_read: true` 都沒效)。
        let content = "---\nenable_read: true\nenable_auto_enter: true\nenable_smart_paste: false\n---\nbody";
        let p = parse_profile("test", content);
        assert!(p.frontmatter.enable_read);
        assert!(p.frontmatter.enable_auto_enter);
        assert!(!p.frontmatter.enable_smart_paste);
    }

    #[test]
    fn parse_profile_mixed_case_keys_treated_equal() {
        // 5N: case-insensitive — Enable_Read / ENABLE_read 等 weird 寫法也吃
        let content = "---\nEnable_Read: true\nENABLE_run_shell: yes\n---\nbody";
        let p = parse_profile("test", content);
        assert!(p.frontmatter.enable_read);
        assert!(p.frontmatter.enable_run_shell);
    }

    #[test]
    fn parse_profile_mori_provider() {
        let content = "---\nprovider: ollama\ncleanup_level: minimal\n---\nbody";
        let p = parse_profile("test", content);
        assert_eq!(p.frontmatter.provider.as_deref(), Some("ollama"));
        assert_eq!(p.frontmatter.cleanup_level, Some(CleanupLevel::Minimal));
    }

    #[test]
    fn resolved_provider_named_from_provider_field() {
        let content = "---\nprovider: groq\n---\nbody";
        let p = parse_profile("test", content);
        assert!(matches!(p.frontmatter.resolved_provider(), ResolvedProvider::Named(name) if name == "groq"));
    }

    #[test]
    fn resolved_provider_default_when_no_provider() {
        let content = "---\nenable_read: true\n---\nbody";
        let p = parse_profile("test", content);
        assert!(matches!(p.frontmatter.resolved_provider(), ResolvedProvider::Default));
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
    fn parse_inline_string_array_basics() {
        assert_eq!(parse_inline_string_array("[a, b, c]"), vec!["a", "b", "c"]);
        assert_eq!(
            parse_inline_string_array(r#"["voice_dict", "glossary"]"#),
            vec!["voice_dict", "glossary"]
        );
        assert_eq!(parse_inline_string_array("[]"), Vec::<String>::new());
        assert_eq!(parse_inline_string_array("[ a ]"), vec!["a"]);
        // 沒 bracket 包裝也容忍(寬鬆 parse)
        assert_eq!(parse_inline_string_array("a, b"), vec!["a", "b"]);
    }

    #[test]
    fn parse_profile_inject_memory_types_inline() {
        let content =
            "---\ninject_memory_types: [voice_dict, glossary]\n---\nbody";
        let p = parse_profile("test", content);
        assert_eq!(
            p.frontmatter.inject_memory_types.as_deref(),
            Some(["voice_dict".to_string(), "glossary".to_string()].as_slice())
        );
    }

    #[test]
    fn parse_profile_inject_memory_types_empty_means_force_off() {
        let content = "---\ninject_memory_types: []\n---\nbody";
        let p = parse_profile("test", content);
        // 「明確空 list」跟「沒寫」語意不同 — Some(empty) 表示 user 想強制關掉
        assert_eq!(p.frontmatter.inject_memory_types, Some(vec![]));
    }

    #[test]
    fn parse_profile_inject_memory_types_missing_means_fallback() {
        let content = "---\nprovider: groq\n---\nbody";
        let p = parse_profile("test", content);
        // 沒這行 → None → resolve 時走 config fallback
        assert!(p.frontmatter.inject_memory_types.is_none());
    }

    #[test]
    fn resolve_inject_types_profile_some_takes_precedence() {
        // profile Some(["voice_dict"]) → 即使 config 有東西也用 profile
        let content = "---\ninject_memory_types: [voice_dict]\n---\nbody";
        let p = parse_profile("test", content);
        let resolved = resolve_inject_memory_types(&p);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], crate::memory::MemoryType::VoiceDict);
    }

    #[test]
    fn resolve_inject_types_profile_empty_forces_off() {
        let content = "---\ninject_memory_types: []\n---\nbody";
        let p = parse_profile("test", content);
        // 即使 config.json 有設,profile Some(empty) 強制空 — 不走 fallback
        let resolved = resolve_inject_memory_types(&p);
        assert!(resolved.is_empty());
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
