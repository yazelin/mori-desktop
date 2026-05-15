// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod action_skills;
mod context_provider;
mod deps;
mod annuli_commands;
mod annuli_config;
mod annuli_supervisor;
mod hotkey_config;
#[cfg(target_os = "linux")]
mod portal_hotkey;
mod x11_hotkey;
#[cfg(target_os = "linux")]
mod x11_shape;
mod recording;
mod character_pack;
// 5U: selection / paste-back 拆 platform-specific 檔案,公開 API 一致
// (read_primary_selection / PlatformPasteController / send_enter /
// warn_if_setup_missing),main.rs 跨平台 call 同一份名稱。
#[cfg_attr(target_os = "linux", path = "selection_linux.rs")]
#[cfg_attr(target_os = "windows", path = "selection_windows.rs")]
mod selection;
mod shell_skill;
mod skill_server;
mod theme;

use std::sync::Arc;

use anyhow::Context as _;
use async_trait::async_trait;
use mori_core::agent::{Agent, AgentMode, SkillCallSummary};
use mori_core::context::{Context as MoriContext, ContextProvider};
use mori_core::llm::groq::{GroqProvider, RetryEvent};
use mori_core::llm::ChatMessage;
use mori_core::memory::markdown::LocalMarkdownMemoryStore;
use mori_core::memory::MemoryStore;
use mori_core::mode::{Mode, ModeController};
use mori_core::paste::PasteController;
use mori_core::skill::PasteSelectionBackSkill;
use mori_core::skill::{
    ComposeSkill, EditMemorySkill, FetchUrlSkill, ForgetMemorySkill, PolishSkill,
    RecallMemorySkill, RememberSkill, SetModeSkill, SkillRegistry, SummarizeSkill, TranslateSkill,
};
use mori_core::{PHASE, VERSION};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::menu::{Menu, MenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Listener, Manager, WindowEvent};

use recording::Recorder;

// ─── State machine ──────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Phase {
    /// 等待熱鍵
    Idle,
    /// 錄音中
    Recording { started_at_ms: i64 },
    /// 已停止錄音,正在打 Whisper API
    Transcribing,
    /// transcript 拿到了,正在問 LLM
    Responding { transcript: String },
    /// 完整一輪結束 — 同時帶 transcript、LLM 回應、用到的 skills
    Done {
        transcript: String,
        response: String,
        skill_calls: Vec<SkillCallSummary>,
    },
    /// 錯誤(任何階段都可以掉到這)
    Error { message: String },
}

impl Default for Phase {
    fn default() -> Self {
        Phase::Idle
    }
}

/// 對話歷史最多保留幾「對」(user + assistant 各算一則,所以實際 message 數是 2x)。
const MAX_HISTORY_PAIRS: usize = 10;

pub struct AppState {
    pub phase: Mutex<Phase>,
    pub recorder: Mutex<Option<Recorder>>,
    /// 透過 GroqProvider::discover_api_key() 在啟動時嘗試取得;
    /// 若無,transcribe 階段會回 Error。
    pub groq_api_key: Mutex<Option<String>>,
    /// 長期記憶 store。Phase 1C 是 LocalMarkdownMemoryStore;
    /// Wave 4 加 AnnuliMemoryStore(走 HTTP),配 ~/.mori/config.json annuli.enabled
    /// 切換。trait object 不寫死 impl。
    ///
    /// **C:RwLock 包起來** — `annuli_reload` 改設定後不用整個 mori-desktop 重啟,
    /// 直接 swap 新 Arc 進來。caller 用 `state.memory_handle()` 拿 snapshot Arc。
    pub memory: parking_lot::RwLock<Arc<dyn mori_core::memory::MemoryStore>>,
    /// Wave 4:如果 annuli.enabled,持有 HTTP client 給對話事件 fire-and-forget +
    /// Ctrl+Alt+Z hotkey 觸發 /sleep。None 表示沒接 annuli。
    /// C:同 memory,RwLock 包起來支援 hot reload。
    pub annuli: parking_lot::RwLock<Option<Arc<mori_core::annuli::AnnuliClient>>>,
    /// D-1: annuli 子 process supervisor。啟動時 spawn task 寫進去,持有 Child
    /// handle(kill_on_drop)。Some 之後就 own 那個 python process,app 退出時
    /// 子 process 跟著掛。`info` 給 status command 回報「我們有沒有 spawn / 為什麼」。
    pub annuli_supervisor: Mutex<Option<annuli_supervisor::AnnuliSupervisor>>,
    /// Working memory:本次 session 的對話歷史(user / assistant 訊息對)。
    /// 重啟 app 就清空。長期記憶寫進 memory 那邊。
    pub conversation: Mutex<Vec<ChatMessage>>,
    /// 運作模式 — Active(平常)/ Background(假寐,麥克風硬關)。
    /// Phase 對應「使用者的這一輪對話進行到哪」,Mode 是「Mori 整體的工作狀態」,
    /// 兩者正交。Phase 變回 Idle 不會動 Mode。
    pub mode: Mutex<Mode>,
    /// Ollama warm-up 狀態(僅當 provider=ollama 時有值):
    /// "loading" | "ready" | "failed",其他 provider 為 None。
    /// 存在 state 是因為 warm-up 可能在 React 還沒掛上 listener 前就完成,
    /// 用 IPC 把當下狀態回給前端就不會錯過 transition。
    pub ollama_warmup: Mutex<Option<&'static str>>,
    /// 熱鍵按下瞬間抓到的視窗 context（5F-1）。
    /// 在 handle_hotkey_toggle 時寫入，run_voice_input_pipeline 時讀取。
    /// 必須在錄音開始前抓，此時焦點還在使用者的目標視窗上。
    pub hotkey_window_context: Mutex<HotkeyWindowContext>,
    /// 5J: 目前跑著的 transcribe + agent pipeline tokio task。
    /// Ctrl+Alt+Esc 在 Phase::Transcribing / Responding 階段可以 abort 它,
    /// kill_on_drop 會把 claude / gemini / codex 子程序連帶 SIGKILL。
    pub pipeline_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// 5T: Toggle chord 的當前語意 — `Toggle`(一按切換)或 `Hold`(按住錄、放開停)。
    /// 啟動時從 `hotkey_config.toggle_mode` 讀入,`config_write` 寫完 disk 後同步
    /// 重讀更新 → 改完即時生效不必重啟。Listener 永遠掛 PRESSED + RELEASED,在
    /// handler 內讀這個 mutex 決定 dispatch。
    pub toggle_mode: Mutex<hotkey_config::ToggleMode>,
}

impl AppState {
    /// 拿 memory store 的 snapshot Arc。briefly 拿 read lock 再 clone 出來,
    /// 釋放鎖再 await 就安全(`Arc<dyn>` 跨 await 是 Send + Sync)。
    pub fn memory_handle(&self) -> Arc<dyn mori_core::memory::MemoryStore> {
        self.memory.read().clone()
    }

    /// 拿 annuli client 的 snapshot(若有設且 enabled)。
    pub fn annuli_handle(&self) -> Option<Arc<mori_core::annuli::AnnuliClient>> {
        self.annuli.read().clone()
    }

    fn set_phase(&self, app: &AppHandle, new_phase: Phase) {
        tracing::info!(?new_phase, "phase change");
        *self.phase.lock() = new_phase.clone();
        if let Err(e) = app.emit("phase-changed", &new_phase) {
            tracing::warn!(?e, "failed to emit phase-changed");
        }
        // v0.3.1: floating.show_mode=recording 時,phase 切換要同步動 floating 顯示/隱藏。
        // Toggle 跟 Hold 兩個 mode 都走這條,一次到位。
        update_floating_visibility(app, &new_phase);
    }

    /// 切換模式。idempotent — 同模式不發 event、不留 log。
    fn set_mode(&self, app: &AppHandle, new_mode: Mode) {
        let prev = *self.mode.lock();
        if prev == new_mode {
            return;
        }
        *self.mode.lock() = new_mode;
        tracing::info!(?prev, ?new_mode, "mode change");
        if let Err(e) = app.emit("mode-changed", &new_mode) {
            tracing::warn!(?e, "failed to emit mode-changed");
        }
    }
}

/// `ModeController` 實作給 mori-core 的 `SetModeSkill` 用 — 這樣 skill
/// 在 mori-core 不依賴 Tauri,也能改 Mode。
struct StateModeController {
    state: Arc<AppState>,
    app: AppHandle,
}

#[async_trait]
impl ModeController for StateModeController {
    async fn current_mode(&self) -> Mode {
        *self.state.mode.lock()
    }

    async fn set_mode(&self, mode: Mode) -> anyhow::Result<()> {
        self.state.set_mode(&self.app, mode);
        Ok(())
    }
}

// ─── IPC commands ───────────────────────────────────────────────────

#[tauri::command]
fn mori_version() -> String {
    VERSION.to_string()
}

#[tauri::command]
fn mori_phase() -> String {
    PHASE.to_string()
}

/// 給 React 端問「現在是不是 X11 session」,React mount 時呼叫一次,
/// 是的話加 `x11-fallback` class 到 documentElement + body,觸發 CSS
/// 的 opaque 背景 + 背板美術 fallback。
///
/// 為什麼不靠 Rust startup eval — 那只能跑一次,使用者按 webview reload
/// 後 React 重新 mount 但 class 沒了 → 背景變回 transparent → X11 黑框
/// 又出現。React 自己 invoke 一次就解決 reload 問題。
#[tauri::command]
fn is_x11_session() -> bool {
    #[cfg(target_os = "linux")]
    {
        x11_hotkey::is_x11_session()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// 給 Config tab 顯示用 — 回傳 session type 字串讓 user 知道
/// Mori 偵測到什麼環境、走哪條 hotkey path。值:
/// - `"x11"`       — Linux X11,走 tauri-plugin-global-shortcut
/// - `"wayland"`   — Linux Wayland,走 xdg-desktop-portal GlobalShortcuts
/// - `"linux-other"` — Linux 但 XDG_SESSION_TYPE 是 tty / 未設(headless / 容器),
///                    熱鍵失效但 UI toggle 仍可用
/// - `"windows"`   — Windows,tauri-plugin-global-shortcut 直接 work
/// - `"macos"`     — macOS,tauri-plugin-global-shortcut 直接 work
/// - `"other"`     — 其他平台(理論上不會發生)
///
/// 函數名歷史包袱叫 `linux_session_type`,但回傳值已含全平台。
#[tauri::command]
fn linux_session_type() -> String {
    #[cfg(target_os = "linux")]
    {
        match std::env::var("XDG_SESSION_TYPE").as_deref() {
            Ok("x11") | Ok("X11") => "x11".to_string(),
            Ok("wayland") | Ok("Wayland") => "wayland".to_string(),
            _ => "linux-other".to_string(),
        }
    }
    #[cfg(target_os = "windows")]
    {
        "windows".to_string()
    }
    #[cfg(target_os = "macos")]
    {
        "macos".to_string()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        "other".to_string()
    }
}

/// X11 上強制 raise window 到 always-on-top layer 頂端。
///
/// 場景:chat_bubble + floating 兩個視窗都 alwaysOnTop。mutter 同 layer 內順
/// 序看「誰最後被 raise」,floating 因為使用者互動頻繁(hover / click / drag)
/// raise event 較新,chat_bubble 雖然 alwaysOnTop:true 仍被壓在下面。
/// setAlwaysOnTop(false → true) 只翻 state 不 re-raise,mutter 不會把它移到
/// layer 頂端。唯一可靠的方法是顯式 XRaiseWindow,這裡 shell-out `xdotool
/// windowraise` 是最簡單的實作。
///
/// Wayland 不需要(compositor 對 alwaysOnTop 處理乾淨,且 wayland 沒有
/// 對應的 XRaiseWindow 等價 API,任何「raise」都不會被允許)。
/// 從 `~/.mori/config.json` 讀 `floating.x11_shape` + `x11_shape_radius`,
/// 缺欄位 fallback 預設值。回 `(shape_str, radius_logical_px)`。
#[cfg(target_os = "linux")]
fn read_floating_shape(config_path: &std::path::Path) -> (String, u32) {
    let default = ("circle".to_string(), 16u32);
    let Ok(text) = std::fs::read_to_string(config_path) else {
        return default;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return default;
    };
    let Some(floating) = json.get("floating") else {
        return default;
    };
    let shape = floating
        .get("x11_shape")
        .and_then(|v| v.as_str())
        .unwrap_or(&default.0)
        .to_string();
    let radius = floating
        .get("x11_shape_radius")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(default.1);
    (shape, radius)
}

/// 讀使用者 `~/.mori/floating/backplate-{dark,light}.png`,有的話以 base64
/// data URL 回給 React 餵 CSS;沒有就回 null,讓 React 用 shipped fallback。
///
/// 為什麼用 data URL 而不是 Tauri asset protocol:asset protocol 需要 in
/// tauri.conf.json security 開啟 + 設 scope,加 capabilities 規則才能跨
/// window 用。data URL 直接是字串,React → CSS variable → background-image
/// 一條龍,不動 Tauri 設定。檔案 ~500KB,base64 後 ~700KB,記憶體 OK。
/// 即時套用 floating window XShape clip。React ConfigTab save 後 invoke
/// 這個 → user 改 shape / radius 不用重啟 Mori。
///
/// `shape` = "square" | "rounded" | "circle"
/// `radius` = logical px(只 rounded 用),Rust 端轉 physical(× scaleFactor)
#[tauri::command]
fn apply_floating_shape(
    app: AppHandle,
    shape: String,
    radius: u32,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        if !x11_hotkey::is_x11_session() {
            // Wayland 沒對應 API,no-op
            return Ok(());
        }
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let win = app
            .get_webview_window("floating")
            .ok_or_else(|| "floating window not found".to_string())?;
        let size = win.inner_size().map_err(|e| e.to_string())?;
        let handle = win.window_handle().map_err(|e| e.to_string())?;
        let xid = match handle.as_raw() {
            RawWindowHandle::Xlib(x) => x.window as u32,
            other => return Err(format!("not Xlib window handle: {:?}", other)),
        };
        let scale = win.scale_factor().unwrap_or(1.0);
        let r_phys = (radius as f64 * scale).round() as u32;
        tracing::info!(xid, shape = %shape, radius_logical = radius, radius_phys = r_phys, "apply_floating_shape");
        let result = match shape.as_str() {
            "square" => x11_shape::clear_clip(xid, size.width, size.height),
            "rounded" => x11_shape::apply_rounded_clip(xid, size.width, size.height, r_phys),
            _ /* circle 或未知 */ => x11_shape::apply_circle_clip(xid, size.width, size.height),
        };
        result.map_err(|e| format!("XShape apply: {e}"))?;
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, shape, radius);
    }
    Ok(())
}

#[tauri::command]
fn read_floating_backplate(theme: String) -> Result<Option<String>, String> {
    #[cfg(target_os = "linux")]
    {
        use base64::Engine as _;
        let Some(home) = std::env::var("HOME").ok() else {
            return Err("HOME not set".to_string());
        };
        let allowed_theme = matches!(theme.as_str(), "dark" | "light");
        if !allowed_theme {
            return Err(format!("invalid theme '{theme}', expected 'dark' or 'light'"));
        }
        let path = std::path::PathBuf::from(home)
            .join(".mori/floating")
            .join(format!("backplate-{theme}.png"));
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(Some(format!("data:image/png;base64,{b64}")))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = theme;
        Ok(None)
    }
}

#[tauri::command]
fn force_raise_window(_app: AppHandle, label: String) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        if !x11_hotkey::is_x11_session() {
            // Wayland / 其他 — 沒對應 XRaiseWindow primitive,直接 OK 不做事
            return Ok(());
        }
        // 用 xdotool search --pid <self> --name "<title>" windowraise 找視窗
        // raise — 走 X11 WM_NAME + _NET_WM_PID 雙重 filter,精準鎖定 Mori
        // 自己這個 process 的目標視窗,不會誤觸其他 app 同名視窗。
        let title = match label.as_str() {
            "floating" => "Mori (floating)",
            "chat_bubble" => "Mori (chat)",
            "picker" => "Mori — 切換 Profile",
            "main" => "Mori",
            other => return Err(format!("unknown window label '{other}'")),
        };
        let pid = std::process::id().to_string();
        let status = std::process::Command::new("xdotool")
            .args(["search", "--pid", &pid, "--name", title, "windowraise"])
            .status()
            .map_err(|e| format!("spawn xdotool: {e}"))?;
        if !status.success() {
            // xdotool search 找不到符合條件視窗會 exit 1,不是真錯 — 例如 picker
            // 還沒第一次 show 過,WM_NAME 還沒進 X server registry。log warn 不
            // bail。
            tracing::warn!(label, title, ?status, "xdotool windowraise exited non-zero");
        } else {
            tracing::debug!(label, title, "force_raise_window via xdotool");
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = label;
    }
    Ok(())
}

/// build SHA / dirty / 時間 / phase / version。值由 build.rs 在 compile
/// time 經 cargo:rustc-env 注入,所以重 build 才會更新。前端用這個秀
/// 在 status panel 上,user 一眼就能分辨「我跑的是哪個 build」。
#[tauri::command]
fn build_info() -> serde_json::Value {
    serde_json::json!({
        "sha": env!("MORI_GIT_SHA"),
        "dirty": !env!("MORI_GIT_DIRTY").is_empty(),
        "build_time": env!("MORI_BUILD_TIME"),
        "phase": PHASE,
        "version": VERSION,
    })
}

/// 目前生效的 chat provider + model + Ollama warm-up 狀態 + 5A-3 routing
/// overrides。讀 ~/.mori/config.json,跟 Routing::build_from_config 走同一條
/// fallback。前端拿來秀 status row + tooltip 顯示 skill 級別的路由。
///
/// `warmup` 從 AppState 取目前快照(loading/ready/failed/null),解決 React
/// listener 來不及訂閱就錯過 emit 的 race。後續 transition 仍走
/// `ollama-warmup` event。
///
/// `skill_overrides` 是 routing.skills 的原始 mapping(skill name → provider name)。
/// 沒設 routing 就回空物件,等於「全部用 agent」。
#[tauri::command]
fn chat_provider_info(state: tauri::State<Arc<AppState>>) -> serde_json::Value {
    let snap = mori_core::llm::active_chat_provider_snapshot();
    let routing = mori_core::llm::read_routing_config();
    let stt = mori_core::llm::transcribe::active_transcribe_snapshot();
    let warmup = *state.ollama_warmup.lock();
    serde_json::json!({
        "name": snap.name,
        "model": snap.model,
        "warmup": warmup,
        "skill_overrides": routing.skills,
        "stt": {
            "name": stt.name,
            "model": stt.model,
            "language": stt.language,
        },
    })
}

#[tauri::command]
fn current_phase(state: tauri::State<Arc<AppState>>) -> Phase {
    state.phase.lock().clone()
}

#[tauri::command]
fn has_groq_key(state: tauri::State<Arc<AppState>>) -> bool {
    state.groq_api_key.lock().is_some()
}

/// Quickstart 第三幕(靈力)用 — 看 `GEMINI_API_KEY` 是否已存在於 OS env var。
/// 跟 groq 不一樣:Gemini 走 `resolve_api_key("GEMINI_API_KEY")` lazy lookup,
/// 不在啟動時存進 state,所以這條直接讀 env 就好。
#[tauri::command]
fn has_gemini_key() -> bool {
    has_env_key("GEMINI_API_KEY")
}

/// Quickstart 第三幕(自訂 OpenAI-compat 端點)用 — 看 `OPENAI_API_KEY` 是否
/// 已存在於 OS env var。跟 Gemini 對稱(api_base 仍需 user 在 UI 填)。
#[tauri::command]
fn has_openai_key() -> bool {
    has_env_key("OPENAI_API_KEY")
}

fn has_env_key(name: &str) -> bool {
    std::env::var(name)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

/// F — Quickstart 用。驗證 user 給的 LLM API key 真的能連。
/// 不打 chat completions(會花 credit),只 GET /models list,200 = key 有效。
///
/// `provider`:
///   - "groq":hardcoded base https://api.groq.com/openai/v1,bearer auth
///   - "openai_compat":任意 OpenAI-相容端點。user 提供 `api_base`(例
///     https://generativelanguage.googleapis.com/v1beta/openai/ 給 Gemini、
///     https://api.deepseek.com/v1、https://openrouter.ai/api/v1 等都可),
///     bearer auth
///
/// 回:Ok(描述) | Err(中文錯誤訊息)
#[tauri::command]
async fn verify_llm_key(
    provider: String,
    key: String,
    api_base: Option<String>,
    env_name: Option<String>,
) -> Result<String, String> {
    // key 空 + 有 env_name → 後端 fallback 讀 env var 拿真值打 API,前端不碰 key 內容。
    // 不打 API 就判 OK 不算測試 — env var 可能值錯 / quota 滿 / endpoint 改了,
    // 真打一次 /models 才知道。
    let typed = key.trim().to_string();
    let (effective_key, via_env) = if !typed.is_empty() {
        (typed, false)
    } else if let Some(name) = env_name.as_deref() {
        match std::env::var(name) {
            Ok(v) if !v.trim().is_empty() => (v.trim().to_string(), true),
            _ => return Err(format!("API key 是空的(環境變數 {name} 也沒設或為空)")),
        }
    } else {
        return Err("API key 是空的".into());
    };
    let key = effective_key;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| format!("init http client: {e}"))?;

    let base = match provider.as_str() {
        "groq" => "https://api.groq.com/openai/v1".to_string(),
        "openai_compat" => {
            let raw = api_base.unwrap_or_default();
            let trimmed = raw.trim().trim_end_matches('/').to_string();
            if trimmed.is_empty() {
                return Err("api_base 是空的 — 必須填 OpenAI-相容端點 URL".into());
            }
            trimmed
        }
        other => return Err(format!("不支援的 provider:{other}(只有 groq / openai_compat)")),
    };

    let url = format!("{base}/models");
    let resp = client
        .get(&url)
        .bearer_auth(&key)
        .send()
        .await
        .map_err(|e| format!("連 {provider} ({url}) 失敗:{e}"))?;

    let status = resp.status();
    match status.as_u16() {
        200 => Ok(if via_env {
            format!("{provider} API key 驗證 OK(使用環境變數)")
        } else {
            format!("{provider} API key 驗證 OK")
        }),
        401 | 403 => Err(format!(
            "API key 無效({}) — 重新檢查 key 完整不完整,或對應 console 重產生",
            status
        )),
        code => {
            // 把 response body 截前 200 字接回 error,user 看得到 server 怎麼說
            let body = resp.text().await.unwrap_or_default();
            let snippet: String = body.chars().take(200).collect();
            Err(format!(
                "API 回 HTTP {code} — 檢查 api_base 是否正確。\n\
                 試的 URL:{url}\n\
                 server 回:{snippet}"
            ))
        }
    }
}

/// 從 UI 觸發 toggle(等同熱鍵)。供測試或無熱鍵權限的環境用。
#[tauri::command]
fn toggle(app: AppHandle, state: tauri::State<Arc<AppState>>) {
    handle_hotkey_toggle(app, state.inner().clone());
}

/// 清空當前對話歷史(working memory),長期記憶不動。
/// UI「重新開始對話」按鈕呼叫。
#[tauri::command]
fn reset_conversation(state: tauri::State<Arc<AppState>>) {
    let mut conv = state.conversation.lock();
    let n = conv.len();
    conv.clear();
    tracing::info!(cleared = n, "conversation reset");
}

/// 取得當前對話歷史長度(訊息數),供 UI 顯示用。
#[tauri::command]
fn conversation_length(state: tauri::State<Arc<AppState>>) -> usize {
    state.conversation.lock().len()
}

/// 5N: Chat panel 重設計需要的對話歷史 dump,frontend 渲染 scrollable thread 用。
/// 只回 user / assistant 訊息(system / tool 過濾掉,使用者不需要看 internal chatter)。
#[derive(serde::Serialize, Clone)]
struct ChatTurn {
    /// "user" | "assistant"
    role: String,
    /// 文字內容(空時不應該出現,但 Option 防 corrupt 狀態)
    content: String,
    /// 若是 assistant 帶 tool_calls,把 name 列出來給 UI badge 用
    tools_called: Vec<String>,
}

#[tauri::command]
fn get_conversation(state: tauri::State<Arc<AppState>>) -> Vec<ChatTurn> {
    state
        .conversation
        .lock()
        .iter()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .map(|m| ChatTurn {
            role: m.role.clone(),
            content: m.content.clone().unwrap_or_default(),
            tools_called: m
                .tool_calls
                .iter()
                .map(|tc| tc.name.clone())
                .collect(),
        })
        .collect()
}

/// 取得當前 Mode(active / background)。
#[tauri::command]
fn current_mode(state: tauri::State<Arc<AppState>>) -> Mode {
    *state.mode.lock()
}

/// UI 切換 Mode。Tray 選單也是這條路。
#[tauri::command]
fn set_mode_cmd(app: AppHandle, state: tauri::State<Arc<AppState>>, mode: Mode) {
    state.set_mode(&app, mode);
}

/// 取消正在進行的錄音 — **不送 Whisper、不進 chat**,直接丟掉音檔回 Idle。
/// UI 在 Recording 狀態下按 Esc 會打這條。
#[tauri::command]
fn cancel_recording(app: AppHandle, state: tauri::State<Arc<AppState>>) {
    let phase = state.phase.lock().clone();
    if !matches!(phase, Phase::Recording { .. }) {
        tracing::info!(?phase, "cancel_recording called outside Recording — ignored");
        return;
    }
    // Stop and discard:取出 recorder、停 stream、把 bytes 丟掉。
    if let Some(rec) = state.recorder.lock().take() {
        match rec.stop() {
            Ok(audio) => {
                let secs = audio.samples.len() as f32
                    / (audio.sample_rate as f32 * audio.channels as f32);
                tracing::info!(
                    duration_secs = secs,
                    samples = audio.samples.len(),
                    "recording cancelled (audio discarded, never sent to Whisper)",
                );
            }
            Err(e) => tracing::warn!(?e, "stop on cancel returned err"),
        }
    }
    state.set_phase(&app, Phase::Idle);
}

/// 直接送一段文字給 Mori(bypass 麥克風 / Whisper)。
///
/// 使用情境:長文摘要、貼文章、貼程式碼等不適合語音輸入的內容。
/// 走的後續流程跟錄音版完全一樣 — 進 Phase::Responding → agent → Done。
#[tauri::command]
fn submit_text(app: AppHandle, state: tauri::State<Arc<AppState>>, text: String) {
    let text = text.trim().to_string();
    if text.is_empty() {
        tracing::warn!("submit_text called with empty input — ignored");
        return;
    }
    // Background 時 mic 是關的,但文字輸入仍應允許 — 允許文字 = 允許 LLM 對話,
    // 模式切換語音指令(「醒醒」)就會跑;避免使用者卡死。
    // 但仍然不允許 Recording / Transcribing / Responding 中切進來。
    {
        let phase = state.phase.lock();
        if !matches!(*phase, Phase::Idle | Phase::Done { .. } | Phase::Error { .. }) {
            tracing::info!("submit_text while busy — ignored");
            return;
        }
    }

    // Phase 5A-3: routing 拆出 agent provider + per-skill provider override。
    // Agent 走 `routing.agent`(可走 tool calling 的:groq / ollama);個別 skill
    // 可在 `routing.skills.<name>` 指到 chat-only provider(claude-cli)用 user
    // 自己的 quota。沒設 routing 時整套退化成全部用 provider。
    let routing = match mori_core::llm::Routing::build_from_config(Some(retry_callback_for(app.clone()))) {
        Ok(r) => r,
        Err(e) => {
            state.set_phase(
                &app,
                Phase::Error {
                    message: format!("{e:#}"),
                },
            );
            return;
        }
    };
    let routing = Arc::new(routing);

    let state_clone = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        run_agent_pipeline(app, state_clone, text, routing).await;
    });
}

/// 建構一個 retry callback,把事件 emit 給前端的 "rate-limit-wait" channel。
/// 給 GroqProvider::with_retry_callback 用。
fn retry_callback_for(app: AppHandle) -> mori_core::llm::groq::RetryCallback {
    Arc::new(move |evt: RetryEvent| {
        tracing::warn!(
            attempt = evt.attempt,
            max = evt.max_attempts,
            wait_secs = evt.wait_secs,
            reason = %evt.reason,
            op = %evt.op,
            "rate limit / retry"
        );
        if let Err(e) = app.emit("rate-limit-wait", &evt) {
            tracing::warn!(?e, "failed to emit rate-limit-wait");
        }
    })
}

// ─── 熱鍵 / toggle 處理 ─────────────────────────────────────────────

// ─── 5F-2: Profile slot switching ────────────────────────────────────

/// Alt+N 按下（5G）：
/// - slot 0  → 切到 Agent 模式（讓 Mori 自己判斷，不選 voice profile）
/// - slot 1~9 → 切到 VoiceInput mode + 對應 voice profile
// ─── 5L: Config UI IPC ────────────────────────────────────────────
//
// 主視窗 Config tab 編輯 ~/.mori/ 內全部設定檔。設計原則:
// - 讀:IPC 直接回字串(整個檔案內容),前端 parse / render form
// - 寫:IPC 收字串,server side validate 後寫檔,失敗回 Result::Err 帶錯誤訊息
// - 不背 schema(profile frontmatter / config.json 各自的 schema 各自驗)
//
// Profile / config 改完即時生效:load_active_profile() / read_provider_config()
// 都是「呼叫時讀檔」,不會 cache。

pub(crate) fn mori_dir() -> std::path::PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(|h| std::path::PathBuf::from(h).join(".mori"))
        .unwrap_or_else(|_| std::path::PathBuf::from(".mori"))
}

#[tauri::command]
fn config_read() -> Result<String, String> {
    let path = mori_dir().join("config.json");
    std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))
}

#[tauri::command]
fn config_write(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    text: String,
) -> Result<(), String> {
    // Validate JSON parses before write,不然容易把 config.json 寫壞
    serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| format!("invalid JSON: {e}"))?;
    let path = mori_dir().join("config.json");
    std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
    // 5P-4: 廣播 config 變動,讓 FloatingMori 等 window 重讀(目前主要給 floating
    // section 的 animated / wander toggle 即時生效用)
    let _ = app.emit("config-changed", ());
    // 5T: 熱套用 hotkeys.toggle_mode — 重讀 disk 後把新 mode 寫進 state,
    // 下一次 PRESSED / RELEASED 立刻走新 dispatch 不必重啟。三大平台都支援。
    let cfg = hotkey_config::HotkeyConfig::load(&path);
    let prev = *state.toggle_mode.lock();
    if prev != cfg.toggle_mode {
        *state.toggle_mode.lock() = cfg.toggle_mode;
        tracing::info!(
            ?prev,
            new = ?cfg.toggle_mode,
            "hotkeys.toggle_mode hot-reloaded",
        );
    }
    // v0.3.1: 熱套用 floating.show_mode — Config 改完立即動 floating 顯示/隱藏,
    // 不用重啟。讀當前 phase 配 helper 內讀 show_mode 一次到位。
    let current_phase = state.phase.lock().clone();
    update_floating_visibility(&app, &current_phase);
    Ok(())
}

/// v0.3.1: chat_bubble 出現時呼叫 `floating_set_above(false)` 把 floating 暫時
/// 從 always-on-top 層放下,bubble 自然顯示在上;bubble 隱藏時 `(true)` 恢復。
/// 比靠 xdotool windowraise 更穩(Wayland 沒 raise API、X11 raise 也會被
/// always_on_top re-assert 蓋掉)。
#[tauri::command]
fn floating_set_above(app: AppHandle, enabled: bool) -> Result<(), String> {
    if let Some(f) = app.get_webview_window("floating") {
        f.set_always_on_top(enabled)
            .map_err(|e| format!("set_always_on_top: {e}"))?;
    }
    Ok(())
}

/// 召喚師按下宿靈儀式第五幕「歡迎回家, Mori」後呼叫 — 讓 floating Mori 現身桌面。
/// 在儀式完成前 floating 是隱藏的(setup hook 讀 quickstart_completed 決定),
/// 這裡是真正讓她「住下」的那個瞬間。
#[tauri::command]
fn floating_show(app: AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window("floating")
        .ok_or_else(|| "floating window not found".to_string())?;
    win.show().map_err(|e| format!("show floating: {e}"))?;
    // v0.3.1 fix:GNOME Wayland mutter 會默默把 hide→show 後的 window 從 always_on_top
    // layer 降下來。要 re-assert 一次它才認帳。沒這行 ritual 跑完 floating 會被
    // 主視窗 / 其他 app 壓住。同樣 trick 在 tray show/hide handler 也用了。
    let _ = win.set_always_on_top(true);
    Ok(())
}

// v0.3.1: floating 顯示時機 — 由 config.json `floating.show_mode` 控制。
// 三個值:
//   "always"    → 一直顯示(預設,跟 v0.3.0 行為一致)
//   "recording" → 只有錄音中/處理中/Done 期間顯示, Idle/Error 隱藏
//   "off"       → 一直隱藏

/// 讀 floating.show_mode,缺欄位或解析失敗預設 "always"。
fn read_floating_show_mode() -> String {
    let path = mori_dir().join("config.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            v.get("floating")
                .and_then(|f| f.get("show_mode"))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "always".to_string())
}

/// 根據 show_mode + phase 決定 floating 應該顯示與否。
/// "recording" 模式刻意把 Done 排除 — Done 會留住直到下一次互動才轉 Idle,
/// 留著的話 Mori 看起來「永遠不消失」, 違背 user 的期待。
/// 處理完成那瞬間(進入 Done)直接隱藏, 跟「動作完成後消失」最對齊。
fn should_show_floating(show_mode: &str, phase: &Phase) -> bool {
    match show_mode {
        "off" => false,
        "recording" => matches!(
            phase,
            Phase::Recording { .. } | Phase::Transcribing | Phase::Responding { .. }
        ),
        _ /* "always" 或未知值 */ => true,
    }
}

/// 套用顯示/隱藏到 floating window。show 時順帶 re-assert always_on_top
/// 防止 mutter 降層。
fn apply_floating_visibility(app: &AppHandle, should_show: bool) {
    if let Some(f) = app.get_webview_window("floating") {
        if should_show {
            let _ = f.show();
            let _ = f.set_always_on_top(true);
        } else {
            let _ = f.hide();
        }
    }
}

/// 中央 helper:讀 config + 給定 phase, 決定 + 套用。
/// 注意:宿靈儀式還沒完成時,floating 永遠隱藏(setup hook 那條 gate 仍有效)。
fn update_floating_visibility(app: &AppHandle, phase: &Phase) {
    // 宿靈儀式還沒完成 → 永遠隱藏,不論 show_mode 設什麼
    let cfg_path = mori_dir().join("config.json");
    let quickstart_done = std::fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("quickstart_completed").and_then(|x| x.as_bool()))
        .unwrap_or(false);
    if !quickstart_done {
        apply_floating_visibility(app, false);
        return;
    }
    let show_mode = read_floating_show_mode();
    let should = should_show_floating(&show_mode, phase);
    apply_floating_visibility(app, should);
}

/// 開外部 URL 到系統預設瀏覽器。Tauri webview 不會處理 `<a target="_blank">`,
/// 前端有要打開外連(去拿 API key 等)就 invoke 這個。走 action_skills 同一份
/// platform::open_url 實作,行為跨 Linux / Windows 一致。
#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(format!("only http(s) URLs allowed: {trimmed}"));
    }
    crate::action_skills::open_url_for_quickstart(trimmed)
        .map_err(|e| format!("open url: {e}"))
}

#[tauri::command]
fn corrections_read() -> Result<String, String> {
    let path = mori_dir().join("corrections.md");
    std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))
}

#[tauri::command]
fn corrections_write(text: String) -> Result<(), String> {
    let path = mori_dir().join("corrections.md");
    std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

#[tauri::command]
fn profile_read(kind: String, stem: String) -> Result<String, String> {
    let dir = match kind.as_str() {
        "voice" => mori_dir().join("voice_input"),
        "agent" => mori_dir().join("agent"),
        other => return Err(format!("unknown profile kind: {other}")),
    };
    let path = dir.join(format!("{stem}.md"));
    std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))
}

#[tauri::command]
fn profile_write(kind: String, stem: String, text: String) -> Result<(), String> {
    let dir = match kind.as_str() {
        "voice" => mori_dir().join("voice_input"),
        "agent" => mori_dir().join("agent"),
        other => return Err(format!("unknown profile kind: {other}")),
    };
    // Validate frontmatter parses(只是 sanity check,不強制 schema)
    match kind.as_str() {
        "voice" => {
            let _ = mori_core::voice_input_profile::parse_profile(&stem, &text);
        }
        "agent" => {
            // parse_agent_profile 會 panic on invalid YAML? 確認一下不會
            // (它用 serde_yml::from_str 包 Result,frontmatter 錯會回 default + warn)
            let _ = mori_core::agent_profile::parse_agent_profile(&stem, &text);
        }
        _ => {}
    }
    let path = dir.join(format!("{stem}.md"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

#[tauri::command]
fn profile_delete(kind: String, stem: String) -> Result<(), String> {
    let dir = match kind.as_str() {
        "voice" => mori_dir().join("voice_input"),
        "agent" => mori_dir().join("agent"),
        other => return Err(format!("unknown profile kind: {other}")),
    };
    let path = dir.join(format!("{stem}.md"));
    std::fs::remove_file(&path).map_err(|e| format!("remove {}: {e}", path.display()))
}

/// Chat panel topbar 顯示用 — 當前 active voice / agent profile 的 stem。
/// 前端依當前 Mode 決定顯示哪個;每次 phase=done 跟收到 profile-switched event
/// 都重抓一次。
#[derive(serde::Serialize)]
struct ActiveProfiles {
    voice: String,
    agent: String,
}

/// LogsTab UI 用 — 撈某天的 events,newest-first,最多 limit 筆。
/// 過濾(kind / provider)放前端做(payload 量小,前端 filter 比後端動態 query 簡單)。
#[tauri::command]
fn log_tail(date: Option<String>, limit: Option<usize>) -> Vec<serde_json::Value> {
    let d = date.unwrap_or_else(mori_core::event_log::today);
    let n = limit.unwrap_or(200);
    mori_core::event_log::read_tail(&d, n)
}

/// LogsTab UI 的日期 picker:列出 ~/.mori/logs/ 內所有 mori-YYYY-MM-DD.jsonl,newest first。
#[tauri::command]
fn log_dates() -> Vec<String> {
    mori_core::event_log::list_dates()
}

#[tauri::command]
fn active_profiles() -> ActiveProfiles {
    ActiveProfiles {
        voice: mori_core::voice_input_profile::load_active_profile().name,
        agent: mori_core::agent_profile::load_active_agent_profile().name,
    }
}

/// 用 OS 檔案管理員開 profile 資料夾 — 方便直接把 .md 拖進去 / 改名 / 刪。
/// 走 action_skills::platform::open_url(同一份 xdg-open / ShellExecuteExW 實作),
/// 對「資料夾路徑」兩平台都會開預設 file manager。
#[tauri::command]
fn open_profile_dir(kind: String) -> Result<(), String> {
    let dir = match kind.as_str() {
        "voice" => mori_dir().join("voice_input"),
        "agent" => mori_dir().join("agent"),
        other => return Err(format!("unknown profile kind: {other}")),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    crate::action_skills::open_url_for_quickstart(&dir.to_string_lossy())
        .map_err(|e| format!("open {}: {e}", dir.display()))
}

/// v0.4.1:列出內建 starter 範本(zh + en 兩語都包進 binary)— Profiles tab
/// 「加入範本」UI 用。回 (filename, lang, display)。
#[tauri::command]
fn list_starter_templates(
    kind: String,
) -> Result<Vec<serde_json::Value>, String> {
    let templates: Vec<mori_core::voice_input_profile::StarterTemplate> = match kind.as_str() {
        "voice" => mori_core::voice_input_profile::list_voice_starters(),
        "agent" => mori_core::agent_profile::list_agent_starters()
            .into_iter()
            .map(|t| mori_core::voice_input_profile::StarterTemplate {
                filename: t.filename,
                lang: t.lang,
                display: t.display,
                content: t.content,
            })
            .collect(),
        other => return Err(format!("unknown profile kind: {other}")),
    };
    Ok(templates
        .into_iter()
        .map(|t| {
            serde_json::json!({
                "filename": t.filename,
                "lang": t.lang,
                "display": t.display,
            })
        })
        .collect())
}

/// v0.4.3:估算 profile system prompt 的 token 數(gpt-oss + Gemini 兩家)。
/// 走 char-class 啟發法,±10% 準確度,給 Profiles tab UI 顯示用。完整原理見
/// `mori_core::tokenize` 模組註解。
#[tauri::command]
fn estimate_profile_tokens(
    kind: String,
    stem: String,
) -> Result<mori_core::tokenize::TokenEstimate, String> {
    let dir = match kind.as_str() {
        "voice" => mori_dir().join("voice_input"),
        "agent" => mori_dir().join("agent"),
        other => return Err(format!("unknown profile kind: {other}")),
    };
    let path = dir.join(format!("{stem}.md"));
    let body = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let stripped = mori_core::tokenize::strip_frontmatter(&body);
    Ok(mori_core::tokenize::estimate_tokens(stripped))
}

/// v0.4.1:把指定 starter 範本寫到 ~/.mori/<dir>/<filename>。檔已存在時:
/// - overwrite=false → 回 Err("already exists"),前端應 confirm
/// - overwrite=true → 覆蓋
/// 用途:user 改壞 .md 想還原 / 想加裝另一語系版本。
#[tauri::command]
fn install_starter_template(
    kind: String,
    filename: String,
    overwrite: bool,
) -> Result<String, String> {
    let dir = match kind.as_str() {
        "voice" => mori_dir().join("voice_input"),
        "agent" => mori_dir().join("agent"),
        other => return Err(format!("unknown profile kind: {other}")),
    };
    let content = match kind.as_str() {
        "voice" => mori_core::voice_input_profile::get_voice_starter_content(&filename),
        "agent" => mori_core::agent_profile::get_agent_starter_content(&filename),
        _ => None,
    };
    let content = content.ok_or_else(|| format!("template not found: {filename}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let path = dir.join(&filename);
    if path.exists() && !overwrite {
        return Err(format!("already exists: {}", path.display()));
    }
    std::fs::write(&path, content).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(format!("installed: {}", path.display()))
}

// ─── 5L-4: Memory + Skills IPC ────────────────────────────────────

#[derive(serde::Serialize, Clone)]
struct MemoryEntry {
    id: String,
    name: String,
    description: String,
    memory_type: String,
}

// 5E-3: 改走 MemoryType::as_str() 集中。VoiceDict 也自動 cover。
fn memory_type_str(t: &mori_core::memory::MemoryType) -> String {
    t.as_str()
}

#[tauri::command]
async fn memory_list(state: tauri::State<'_, Arc<AppState>>) -> Result<Vec<MemoryEntry>, String> {
    let memory = state.memory_handle();
    let entries = memory.read_index().await.map_err(|e| format!("read_index: {e}"))?;
    Ok(entries
        .into_iter()
        .map(|e| MemoryEntry {
            id: e.id,
            name: e.name,
            description: e.description,
            memory_type: memory_type_str(&e.memory_type),
        })
        .collect())
}

#[derive(serde::Serialize, Clone)]
struct MemoryDetail {
    id: String,
    name: String,
    description: String,
    memory_type: String,
    created: String,
    last_used: String,
    body: String,
}

#[tauri::command]
async fn memory_read(
    state: tauri::State<'_, Arc<AppState>>,
    id: String,
) -> Result<Option<MemoryDetail>, String> {
    let memory = state.memory_handle();
    let m = memory.read(&id).await.map_err(|e| format!("read: {e}"))?;
    Ok(m.map(|m| MemoryDetail {
        id: m.id,
        name: m.name,
        description: m.description,
        memory_type: memory_type_str(&m.memory_type),
        created: m.created.to_rfc3339(),
        last_used: m.last_used.to_rfc3339(),
        body: m.body,
    }))
}

#[derive(serde::Deserialize)]
struct MemoryWriteArgs {
    id: String,
    name: String,
    description: String,
    /// "user_identity" | "preference" | "skill_outcome" | "project" | "reference" | <其他字串>
    memory_type: String,
    body: String,
}

fn parse_memory_type(s: &str) -> mori_core::memory::MemoryType {
    use mori_core::memory::MemoryType::*;
    match s {
        "user_identity" => UserIdentity,
        "preference" => Preference,
        "skill_outcome" => SkillOutcome,
        "project" => Project,
        "reference" => Reference,
        other => Other(other.to_string()),
    }
}

#[tauri::command]
async fn memory_write(
    state: tauri::State<'_, Arc<AppState>>,
    args: MemoryWriteArgs,
) -> Result<(), String> {
    if args.id.trim().is_empty() {
        return Err("id 不可空".into());
    }
    let now = chrono::Utc::now();
    let memory_entry = mori_core::memory::Memory {
        id: args.id,
        name: args.name,
        description: args.description,
        memory_type: parse_memory_type(&args.memory_type),
        created: now,
        last_used: now,
        body: args.body,
    };
    state.memory_handle().write(memory_entry).await.map_err(|e| format!("write: {e}"))
}

#[tauri::command]
async fn memory_delete(state: tauri::State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    state.memory_handle().delete(&id).await.map_err(|e| format!("delete: {e}"))
}

/// 5L-5: 全文搜尋 memory(name / description / body 都搜)。
/// 回傳跟 memory_list 一樣的 MemoryEntry,加上 hit 程度可後續排序(現在依 store 順序)。
// ─── 5O: Dependencies IPC ────────────────────────────────────────

/// 送給前端的 DepInfo — 從 DepSpec 摘關鍵欄位,**`install` 已經是當前平台
/// 適用的版本**(從 install_overrides 解析過),前端不用知道 overrides 存在。
/// `platforms` / `install_overrides` 是 server-side internal,不送出。
#[derive(serde::Serialize, Clone)]
struct DepInfo {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    unlocks: &'static str,
    size_hint: Option<&'static str>,
    needs_sudo: bool,
    /// 「能用但有限制」的警告 — 例:whisper-server 在 Windows 標 Manual。
    /// 前端 render 成 ⚠️ badge + tooltip。
    install_caveat: Option<&'static str>,
    check: crate::deps::CheckSpec,
    install: crate::deps::InstallSpec,
    status: crate::deps::DepStatus,
}

#[tauri::command]
fn deps_list() -> Vec<DepInfo> {
    // 過濾掉跟當前平台無關的 deps — Windows user 不用看到 ydotool / xdotool /
    // xclip;Linux user 不用看到 Windows installer 那一堆。`applies_to_current_os()`
    // 在 deps.rs 內看 spec.platforms 是否含 `std::env::consts::OS`。
    crate::deps::registry()
        .into_iter()
        .filter(|spec| spec.applies_to_current_os())
        .map(|spec| {
            let status = crate::deps::check_dep(&spec);
            // 平台特定 install override 在 server-side 已解析,前端拿到的
            // `install` 就是當前 OS 該用的那個版本。
            let install = spec.effective_install().clone();
            DepInfo {
                id: spec.id,
                name: spec.name,
                description: spec.description,
                unlocks: spec.unlocks,
                size_hint: spec.size_hint,
                needs_sudo: spec.needs_sudo,
                install_caveat: spec.install_caveat,
                check: spec.check.clone(),
                install,
                status,
            }
        })
        .collect()
}

#[tauri::command]
async fn deps_install(id: String) -> Result<crate::deps::InstallResult, String> {
    let spec = crate::deps::registry()
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("unknown dep id: {id}"))?;
    // Manual install 走不到 run_install — UI 已直接顯示指令給 user
    tokio::task::spawn_blocking(move || crate::deps::run_install(&spec))
        .await
        .map_err(|e| format!("install join: {e}"))?
        .map_err(|e| format!("install: {e:#}"))
}

// ─── brand-3: Theme IPC ───────────────────────────────────────────

#[tauri::command]
fn theme_list() -> Result<Vec<crate::theme::ThemeEntry>, String> {
    crate::theme::list().map_err(|e| format!("list themes: {e:#}"))
}

#[tauri::command]
fn theme_read(stem: String) -> Result<crate::theme::Theme, String> {
    crate::theme::read(&stem).map_err(|e| format!("read theme {stem}: {e:#}"))
}

/// 回 (stem, theme) 給 frontend 啟動時套用。
/// v0.4.1:`default_light` hint(從前端 prefers-color-scheme 算)在沒
/// active_theme 檔時決定 fallback 是 light 還是 dark — user 在 OS 設淺色,
/// Mori 預設跟。已存在 user 顯式 set 過的 theme 時 hint 忽略,尊重 user 選擇。
#[tauri::command]
fn theme_get_active(default_light: Option<bool>) -> Result<(String, crate::theme::Theme), String> {
    let stem = crate::theme::get_active_stem_with_default(default_light.unwrap_or(false));
    let theme = crate::theme::read(&stem)
        .or_else(|_| crate::theme::read("dark")) // fallback
        .map_err(|e| format!("read active theme: {e:#}"))?;
    Ok((stem, theme))
}

#[tauri::command]
fn theme_set_active(stem: String) -> Result<crate::theme::Theme, String> {
    let theme = crate::theme::read(&stem).map_err(|e| format!("read theme {stem}: {e:#}"))?;
    crate::theme::set_active_stem(&stem).map_err(|e| format!("set active: {e:#}"))?;
    Ok(theme)
}

/// 一鍵 toggle dark <-> light(找同 base 的內建 theme),回切換後的 (stem, theme)
#[tauri::command]
fn theme_toggle() -> Result<(String, crate::theme::Theme), String> {
    let cur = crate::theme::get_active_stem();
    let next = crate::theme::toggle_base_stem(&cur).map_err(|e| format!("toggle: {e:#}"))?;
    let theme = crate::theme::read(&next).map_err(|e| format!("read {next}: {e:#}"))?;
    crate::theme::set_active_stem(&next).map_err(|e| format!("set active: {e:#}"))?;
    Ok((next, theme))
}

/// 把 themes 目錄 path 回給前端,讓 UI 顯示「打開資料夾」
#[tauri::command]
fn theme_dir() -> String {
    crate::theme::themes_dir().display().to_string()
}

// ─── 5P-1: Character pack IPC ─────────────────────────────────────

#[tauri::command]
fn character_list() -> Result<Vec<crate::character_pack::CharacterEntry>, String> {
    crate::character_pack::list().map_err(|e| format!("list characters: {e:#}"))
}

#[tauri::command]
fn character_get_active() -> Result<(String, crate::character_pack::CharacterManifest), String> {
    let stem = crate::character_pack::get_active();
    let m = crate::character_pack::load_manifest(&stem)
        .map_err(|e| format!("load manifest {stem}: {e:#}"))?;
    Ok((stem, m))
}

#[tauri::command]
fn character_set_active(
    stem: String,
) -> Result<crate::character_pack::CharacterManifest, String> {
    crate::character_pack::set_active(&stem).map_err(|e| format!("set active: {e:#}"))?;
    let m = crate::character_pack::load_manifest(&stem)
        .map_err(|e| format!("load manifest {stem}: {e:#}"))?;
    Ok(m)
}

/// 讀 sprite 檔成 data URL(`data:image/png;base64,...`)讓 frontend `<img>` /
/// CSS `background-image` 直接套。每張 sprite 對話只取一次,frontend 該 memoize。
///
/// Fallback chain(找第一個存在的 PNG):
///   1. <stem>/sprites/<state>.png         ← 角色自己這 state 的 sprite
///   2. mori/sprites/<state>.png           ← default mori 同 state
///   3. <stem>/sprites/idle.png            ← 角色自己 idle 充當(例 walking / dragging)
///   4. mori/sprites/idle.png              ← 最後保底
#[tauri::command]
fn character_sprite_data_url(stem: String, state: String) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let candidates = [
        crate::character_pack::sprite_path(&stem, &state),
        crate::character_pack::sprite_path("mori", &state),
        crate::character_pack::sprite_path(&stem, "idle"),
        crate::character_pack::sprite_path("mori", "idle"),
    ];
    for (idx, p) in candidates.iter().enumerate() {
        if !p.exists() {
            continue;
        }
        if idx > 0 {
            tracing::debug!(
                stem = %stem,
                state = %state,
                fell_back = idx,
                using = %p.display(),
                "sprite missing in primary, using fallback",
            );
        }
        let bytes = std::fs::read(p).map_err(|e| format!("read sprite {state}: {e:#}"))?;
        return Ok(format!("data:image/png;base64,{}", STANDARD.encode(&bytes)));
    }
    Err(format!("sprite not found: {} (no fallback available)", state))
}

#[tauri::command]
fn character_dir() -> String {
    crate::character_pack::characters_dir().display().to_string()
}

/// 升級任意 character pack 內 single-frame sprite 到 4×4 placeholder。
/// 主要給 user import 進來的 pack(非 default mori)用 — Config UI 按鈕呼叫。
/// 回 (upgraded, skipped)。
#[tauri::command]
fn character_upgrade_pack_to_4x4(stem: String) -> Result<(usize, usize), String> {
    crate::character_pack::upgrade_pack_to_4x4(&stem)
        .map_err(|e| format!("upgrade pack {stem}: {e:#}"))
}

/// C — annuli 熱重載 command。
///
/// 流程:
/// 1. 重讀 `~/.mori/config.json` 的 `annuli` 子樹
/// 2. 若 ready:重建 AnnuliClient + AnnuliMemoryStore;不 ready:重建 LocalMarkdownMemoryStore
/// 3. 原子 swap 進 AppState.memory / AppState.annuli
///
/// 注意:
/// - **不動 annuli_supervisor**(D-1 起的 python child process 還活著)。若 user
///   改了 endpoint port / spirit_name 想 spawn 新 annuli,得手動 pkill + 重啟
///   mori-desktop。MVP 範圍不做 supervisor 重新 evaluate。
/// - skill_server / annuli_commands / agent pipeline 都走 state.memory_handle() /
///   state.annuli_handle() 拿 snapshot,所以 swap 完下一次 invoke 自動拿到新 store。
/// - 既有 in-flight 請求(例如 AnnuliMemoryStore 正在 POST /memory/section)持有
///   舊 client 的 Arc,會跑完才 drop — 不會中斷。
#[tauri::command]
async fn annuli_reload(state: tauri::State<'_, Arc<AppState>>) -> Result<String, String> {
    let config_path = mori_core::llm::groq::GroqProvider::bootstrap_mori_config()
        .map_err(|e| format!("locate config.json: {e:#}"))?;
    let annuli_cfg = annuli_config::AnnuliConfig::load(&config_path);

    if annuli_cfg.is_ready() {
        let client = mori_core::annuli::AnnuliClient::new(annuli_cfg.to_client_config())
            .map_err(|e| format!("build annuli client: {e:#}"))?;
        let client = Arc::new(client);
        let store: Arc<dyn mori_core::memory::MemoryStore> = Arc::new(
            mori_core::memory::annuli::AnnuliMemoryStore::new(client.clone()),
        );
        *state.memory.write() = store;
        *state.annuli.write() = Some(client);
        tracing::info!(
            endpoint = %annuli_cfg.endpoint,
            spirit = %annuli_cfg.spirit_name,
            user_id = %annuli_cfg.user_id,
            "annuli hot-reload → AnnuliMemoryStore"
        );
        Ok(format!(
            "reloaded: annuli enabled @ {} (spirit={}, user_id={})",
            annuli_cfg.endpoint, annuli_cfg.spirit_name, annuli_cfg.user_id
        ))
    } else {
        let memory_root = mori_core::memory::markdown::LocalMarkdownMemoryStore::default_root()
            .map_err(|e| format!("locate ~/.mori/memory: {e:#}"))?;
        let store = mori_core::memory::markdown::LocalMarkdownMemoryStore::new(memory_root.clone())
            .map_err(|e| format!("init LocalMarkdown store: {e:#}"))?;
        *state.memory.write() = Arc::new(store);
        *state.annuli.write() = None;
        tracing::info!(
            path = %memory_root.display(),
            "annuli hot-reload → LocalMarkdownMemoryStore (annuli disabled)"
        );
        Ok(format!(
            "reloaded: annuli disabled, fallback LocalMarkdown @ {}",
            memory_root.display()
        ))
    }
}

#[tauri::command]
async fn memory_search(
    state: tauri::State<'_, Arc<AppState>>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<MemoryEntry>, String> {
    let limit = limit.unwrap_or(50);
    let hits = state
        .memory_handle()
        .search(&query, limit)
        .await
        .map_err(|e| format!("search: {e}"))?;
    Ok(hits
        .into_iter()
        .map(|m| MemoryEntry {
            id: m.id,
            name: m.name,
            description: m.description,
            memory_type: memory_type_str(&m.memory_type),
        })
        .collect())
}

#[derive(serde::Serialize, Clone)]
struct SkillInfo {
    name: String,
    description: String,
    parameters: serde_json::Value,
    /// "builtin" | "shell"
    kind: String,
    /// 此 skill 適用的平台清單(對齊 std::env::consts::OS:linux/windows/macos)。
    /// IPC 層已經過濾,前端拿到的 list 全是當前 OS 適用的,這個欄位主要給 UI
    /// 顯示「跨平台 / 限 X 平台」標籤用。
    platforms: Vec<String>,
    /// 「能用但有限制」的警告 — 例:Windows 上 paste_selection_back 需先 Ctrl+C。
    /// Skill::platform_caveat() 回 None 就是 None。
    caveat: Option<String>,
}

#[tauri::command]
async fn skills_list(state: tauri::State<'_, Arc<AppState>>) -> Result<Vec<SkillInfo>, String> {
    // 內容跟 skill_server::build_dynamic_registry 等價,直接呼叫(在 main 直接拼簡單版)
    let memory = state.memory_handle();
    let routing = mori_core::llm::Routing::build_from_config(None)
        .map_err(|e| format!("build routing: {e}"))?;
    let mut registry = SkillRegistry::new();
    let mem_arc: Arc<dyn MemoryStore> = memory;
    registry.register(Arc::new(TranslateSkill::new(routing.skill_provider("translate"))));
    registry.register(Arc::new(PolishSkill::new(routing.skill_provider("polish"))));
    registry.register(Arc::new(SummarizeSkill::new(routing.skill_provider("summarize"))));
    registry.register(Arc::new(ComposeSkill::new(routing.skill_provider("compose"))));
    registry.register(Arc::new(FetchUrlSkill::new()));
    registry.register(Arc::new(RememberSkill::new(mem_arc.clone())));
    registry.register(Arc::new(RecallMemorySkill::new(mem_arc.clone())));
    registry.register(Arc::new(ForgetMemorySkill::new(mem_arc.clone())));
    registry.register(Arc::new(EditMemorySkill::new(mem_arc.clone())));
    registry.register(Arc::new(crate::action_skills::OpenUrlSkill));
    registry.register(Arc::new(crate::action_skills::OpenAppSkill));
    registry.register(Arc::new(crate::action_skills::SendKeysSkill));
    registry.register(Arc::new(crate::action_skills::GoogleSearchSkill));
    registry.register(Arc::new(crate::action_skills::AskChatGptSkill));
    registry.register(Arc::new(crate::action_skills::AskGeminiSkill));
    registry.register(Arc::new(crate::action_skills::FindYoutubeSkill));
    // 當前 agent profile 的 shell skills
    let profile = mori_core::agent_profile::load_active_agent_profile();
    let shell_skill_names: std::collections::HashSet<String> = profile
        .frontmatter
        .shell_skills
        .iter()
        .map(|d| d.name.clone())
        .collect();
    for def in &profile.frontmatter.shell_skills {
        registry.register(Arc::new(crate::shell_skill::ShellSkill::new(def.clone())));
    }
    let _ = state; // suppress unused warning (state above is only used through .memory)

    // 走 registry.names() 而不是 tool_definitions(),因為要拿到 Skill object
    // 才能 call .platforms() / .platform_caveat()。順便 server-side filter
    // 掉不適用當前 OS 的 skill,前端 list 完全乾淨。
    let os = std::env::consts::OS;
    let skills = registry
        .names()
        .into_iter()
        .filter_map(|name| {
            let skill = registry.get(name)?;
            let platforms: Vec<&'static str> = skill.platforms().to_vec();
            if !platforms.iter().any(|p| *p == os) {
                return None; // 此 skill 不適用當前平台
            }
            let kind = if shell_skill_names.contains(name) {
                "shell".to_string()
            } else {
                "builtin".to_string()
            };
            Some(SkillInfo {
                name: name.to_string(),
                description: skill.description().to_string(),
                parameters: skill.parameters_schema(),
                kind,
                platforms: platforms.into_iter().map(String::from).collect(),
                caveat: skill.platform_caveat().map(String::from),
            })
        })
        .collect();
    Ok(skills)
}

// ─── 5K-1: Picker UI IPC ────────────────────────────────────────────

#[derive(serde::Serialize, Clone)]
struct ProfileEntry {
    stem: String,
    display: String,
}

#[tauri::command]
fn picker_list_voice_profiles() -> Vec<ProfileEntry> {
    mori_core::voice_input_profile::list_voice_profiles()
        .into_iter()
        .map(|(stem, display)| ProfileEntry { stem, display })
        .collect()
}

#[tauri::command]
fn picker_list_agent_profiles() -> Vec<ProfileEntry> {
    mori_core::agent_profile::list_agent_profiles()
        .into_iter()
        .map(|(stem, display)| ProfileEntry { stem, display })
        .collect()
}

#[tauri::command]
fn picker_switch_voice_profile(
    app: AppHandle,
    state: tauri::State<Arc<AppState>>,
    stem: String,
) {
    if !matches!(*state.mode.lock(), Mode::VoiceInput) {
        state.set_mode(&app, Mode::VoiceInput);
    }
    if let Some(info) = mori_core::voice_input_profile::switch_to_profile(&stem) {
        let _ = app.emit("voice-input-profile-switched", info.label());
    }
}

#[tauri::command]
fn picker_switch_agent_profile(
    app: AppHandle,
    state: tauri::State<Arc<AppState>>,
    stem: String,
) {
    if !matches!(*state.mode.lock(), Mode::Agent) {
        state.set_mode(&app, Mode::Agent);
    }
    if let Some(info) = mori_core::agent_profile::switch_to_agent_profile(&stem) {
        let label = format!("Agent · {} · {}", info.profile_name, info.llm_provider);
        let _ = app.emit("voice-input-profile-switched", label);
    }
}

/// Alt+N 按下：永遠進入 VoiceInput 模式 + 載入對應 USER-0N.*.md。
///
/// slot 0 = USER-00.*(預設極簡語音輸入,類似 iOS 語音輸入法,不潤稿)。
/// 切回 Agent 走 Ctrl+Alt+0~9(對應 AGENT-XX),Alt 系列全部 voice_input。
fn handle_profile_slot(app: AppHandle, state: Arc<AppState>, slot: u8) {
    if !matches!(*state.mode.lock(), Mode::VoiceInput) {
        tracing::info!(slot, "Alt+N — switching to VoiceInput mode");
        state.set_mode(&app, Mode::VoiceInput);
    }

    match mori_core::voice_input_profile::switch_to_slot(slot) {
        Some(info) => {
            let _ = app.emit("voice-input-profile-switched", info.label());
        }
        None => {
            tracing::debug!(slot, "no voice profile file for slot {}", slot);
        }
    }
}

/// Ctrl+Alt+N 按下（5G-5）：
/// - 永遠切到 Agent mode
/// - slot 0 → 用內建預設 AGENT.md（Mori 自由判斷）
/// - slot 1~9 → 載入 AGENT-0N.*.md
fn handle_agent_profile_slot(app: AppHandle, state: Arc<AppState>, slot: u8) {
    if !matches!(*state.mode.lock(), Mode::Agent) {
        tracing::info!(slot, "Ctrl+Alt+N — switching to Agent mode");
        state.set_mode(&app, Mode::Agent);
    }

    match mori_core::agent_profile::switch_agent_slot(slot) {
        Some(info) => {
            let label = format!("Agent · {} · {}", info.profile_name, info.llm_provider);
            let _ = app.emit("voice-input-profile-switched", label);
        }
        None => {
            tracing::debug!(slot, "no AGENT-{:02}.* file for slot", slot);
        }
    }
}

// ─── 5F-1: Window context capture ────────────────────────────────────

/// 熱鍵按下瞬間抓到的視窗資訊。此時焦點還在使用者的目標視窗，是抓 context 的唯一可靠時機。
#[derive(Debug, Clone, Default)]
pub struct HotkeyWindowContext {
    pub process_name: String,
    pub window_title: String,
    pub selected_text: String,
}

/// Linux: 用 xdotool + /proc 抓活躍視窗 context。同步呼叫，耗時 < 100ms。
/// 失敗時各欄位回空字串，不影響主流程。
#[cfg(target_os = "linux")]
fn capture_window_context() -> HotkeyWindowContext {
    use std::process::Command;

    let pid = Command::new("xdotool")
        .args(["getactivewindow", "getwindowpid"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let process_name = pid
        .as_deref()
        .and_then(|pid| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let window_title = Command::new("xdotool")
        .args(["getactivewindow", "getwindowname"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let selected_text = crate::selection::read_primary_selection().unwrap_or_default();

    tracing::debug!(
        process = %process_name,
        title = %window_title,
        selected_chars = selected_text.chars().count(),
        "hotkey window context captured",
    );

    HotkeyWindowContext { process_name, window_title, selected_text }
}

/// Windows:GetForegroundWindow + GetWindowThreadProcessId +
/// QueryFullProcessImageNameW(process_name)+ GetWindowTextW(window_title)。
/// selected_text 一律空 — Windows 沒 PRIMARY selection。
///
/// 全部 Win32 API,同步呼叫,< 10ms。失敗時各欄位回空字串,不影響主流程。
#[cfg(target_os = "windows")]
fn capture_window_context() -> HotkeyWindowContext {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, HWND, MAX_PATH};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    };

    let hwnd: HWND = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return HotkeyWindowContext::default();
    }

    // window_title
    let title_len = unsafe { GetWindowTextLengthW(hwnd) };
    let window_title = if title_len > 0 {
        let mut buf = vec![0u16; (title_len as usize) + 1];
        let written = unsafe { GetWindowTextW(hwnd, &mut buf) };
        if written > 0 {
            String::from_utf16_lossy(&buf[..written as usize])
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    // process_name
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32)) };
    let process_name = if pid != 0 {
        match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
            Ok(handle) => {
                let mut buf = vec![0u16; MAX_PATH as usize];
                let mut size = buf.len() as u32;
                let ok = unsafe {
                    QueryFullProcessImageNameW(
                        handle,
                        PROCESS_NAME_FORMAT(0),
                        PWSTR(buf.as_mut_ptr()),
                        &mut size,
                    )
                };
                let _ = unsafe { CloseHandle(handle) };
                if ok.is_ok() && size > 0 {
                    let path = String::from_utf16_lossy(&buf[..size as usize]);
                    // 只留 basename(去 ".exe")— 跟 Linux 的 /proc/<pid>/comm 對齊
                    std::path::Path::new(&path)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                }
            }
            Err(_) => String::new(),
        }
    } else {
        String::new()
    };

    tracing::debug!(
        process = %process_name,
        title = %window_title,
        "hotkey window context captured",
    );

    HotkeyWindowContext {
        process_name,
        window_title,
        selected_text: String::new(),
    }
}

/// macOS / 其他平台 fallback — 回空 context,不影響主流程。
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn capture_window_context() -> HotkeyWindowContext {
    HotkeyWindowContext::default()
}

fn handle_hotkey_toggle(app: AppHandle, state: Arc<AppState>) {
    // Background 模式下熱鍵語意是「叫醒 + 開錄」一鍵到位,不用先開 tray menu。
    // 切到 Active 後 fall-through 走正常 toggle 邏輯,phase 仍是 Idle 所以
    // 會進到 start_recording。
    if matches!(*state.mode.lock(), Mode::Background) {
        tracing::info!("hotkey while Background → wake to Active + start recording");
        state.set_mode(&app, Mode::Agent);
    }

    let current = state.phase.lock().clone();
    match current {
        Phase::Idle | Phase::Done { .. } | Phase::Error { .. } => {
            // 5F-1: 在錄音開始前抓視窗 context（焦點仍在目標 app）
            *state.hotkey_window_context.lock() = capture_window_context();
            start_recording(&app, &state);
        }
        Phase::Recording { .. } => {
            stop_and_transcribe(app, state);
        }
        Phase::Transcribing | Phase::Responding { .. } => {
            // Mori 目前沒做 async task queue,新 hotkey 進來就 abort 舊 pipeline + 開新錄音。
            // 行為跟 Ctrl+Alt+Esc 一致,差在 Esc 停在 Idle 不再開錄。
            tracing::info!(?current, "toggle while busy — aborting pipeline + starting new recording");
            if let Some(task) = state.pipeline_task.lock().take() {
                task.abort();
            }
            state.set_phase(&app, Phase::Idle);
            *state.hotkey_window_context.lock() = capture_window_context();
            start_recording(&app, &state);
        }
    }
}

/// Hold 模式 — chord 按下:相當於 toggle 模式的「開錄」分支,但只負責開,
/// 不會 toggle。已在錄音中 / busy 時是 no-op。
fn handle_hotkey_pressed(app: AppHandle, state: Arc<AppState>) {
    if matches!(*state.mode.lock(), Mode::Background) {
        tracing::info!("hotkey press while Background → wake to Active + start recording");
        state.set_mode(&app, Mode::Agent);
    }
    let current = state.phase.lock().clone();
    match current {
        Phase::Idle | Phase::Done { .. } | Phase::Error { .. } => {
            *state.hotkey_window_context.lock() = capture_window_context();
            start_recording(&app, &state);
        }
        Phase::Recording { .. } => {
            tracing::debug!("hotkey press while already recording — ignored");
        }
        Phase::Transcribing | Phase::Responding { .. } => {
            // 同 toggle 路徑:沒 task queue, abort 舊 pipeline 開新錄音。
            tracing::info!(?current, "hotkey press while busy — aborting pipeline + starting new recording");
            if let Some(task) = state.pipeline_task.lock().take() {
                task.abort();
            }
            state.set_phase(&app, Phase::Idle);
            *state.hotkey_window_context.lock() = capture_window_context();
            start_recording(&app, &state);
        }
    }
}

/// Hold 模式 — chord 放開:只在 Recording 中觸發 stop_and_transcribe。
/// 其他 phase 不動作(例如使用者放開太快、key repeat 時序、或 release 比
/// press 還早抵達的 portal 邊角)。
fn handle_hotkey_released(app: AppHandle, state: Arc<AppState>) {
    let current = state.phase.lock().clone();
    match current {
        Phase::Recording { .. } => stop_and_transcribe(app, state),
        _ => {
            tracing::debug!(?current, "hotkey released but not recording — ignored");
        }
    }
}

fn start_recording(app: &AppHandle, state: &Arc<AppState>) {
    // 雙保險:Background 不該有路徑進到這裡(handle_hotkey_toggle 會先 wake),
    // 但若使用者改 Mode 後 IPC 直接 toggle,守住這一道,避免「Background 卻在錄音」
    // 的字面違反。
    if matches!(*state.mode.lock(), Mode::Background) {
        tracing::warn!("start_recording while Background — refused (mic stays off)");
        return;
    }
    match Recorder::start() {
        Ok(rec) => {
            // 取得 level atomic 共享給 polling task
            let level_handle = rec.level_arc();
            *state.recorder.lock() = Some(rec);
            let now_ms = chrono::Utc::now().timestamp_millis();
            state.set_phase(
                app,
                Phase::Recording {
                    started_at_ms: now_ms,
                },
            );

            // 即時 audio-level 推送給前端,~30Hz
            let app_clone = app.clone();
            let state_clone = state.clone();
            tauri::async_runtime::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_millis(33));
                loop {
                    interval.tick().await;
                    // 只有錄音中才推
                    let still_recording = matches!(
                        *state_clone.phase.lock(),
                        Phase::Recording { .. }
                    );
                    if !still_recording {
                        // 推一次 0 結尾,UI 平滑回零
                        let _ = app_clone.emit("audio-level", 0.0_f32);
                        break;
                    }
                    let raw = level_handle
                        .load(std::sync::atomic::Ordering::Relaxed);
                    let normalized = raw as f32 / u16::MAX as f32;
                    let _ = app_clone.emit("audio-level", normalized);
                }
            });
        }
        Err(e) => {
            tracing::error!(?e, "failed to start recorder");
            state.set_phase(
                app,
                Phase::Error {
                    message: format!("錄音啟動失敗:{e:#}"),
                },
            );
        }
    }
}

fn stop_and_transcribe(app: AppHandle, state: Arc<AppState>) {
    let recorder = match state.recorder.lock().take() {
        Some(r) => r,
        None => {
            state.set_phase(
                &app,
                Phase::Error {
                    message: "stop_and_transcribe called but no recorder active".into(),
                },
            );
            return;
        }
    };

    state.set_phase(&app, Phase::Transcribing);

    // 5F: 通知 floating widget 顯示轉錄狀態（包含 STT provider 名稱）
    // 若 profile 有覆蓋 stt_provider 則顯示它，否則顯示全域預設
    {
        let name = if matches!(*state.mode.lock(), Mode::VoiceInput) {
            mori_core::voice_input_profile::load_active_profile()
                .frontmatter
                .stt_provider
                .unwrap_or_else(|| {
                    mori_core::llm::transcribe::active_transcribe_snapshot().name
                })
        } else {
            mori_core::llm::transcribe::active_transcribe_snapshot().name
        };
        let _ = app.emit("voice-input-status", format!("轉錄中 · {}", name));
    }

    let app_for_provider = app.clone();

    let state_for_handle = state.clone();
    let task = tauri::async_runtime::spawn(async move {
        // Stage 1: Whisper transcribe — 5C 起走 TranscriptionProvider factory,
        // 預設 Groq Whisper API,可在 config 把 stt_provider
        // 改成 "whisper-local" 走 whisper.cpp 離線推理(配上本機 chat
        // provider 就 100% Groq-free)。
        let transcribe_result: anyhow::Result<String> = async {
            let audio = recorder.stop().context("stop recorder")?;
            let duration = audio.duration_secs();
            let rms = if audio.samples.is_empty() {
                0.0
            } else {
                let sum_sq: f64 = audio
                    .samples
                    .iter()
                    .map(|&s| (s as f64 / i16::MAX as f64).powi(2))
                    .sum();
                (sum_sq / audio.samples.len() as f64).sqrt()
            };
            tracing::info!(
                duration_secs = duration,
                samples = audio.samples.len(),
                rms = rms,
                rms_db = 20.0 * rms.log10(),
                "recorded; encoding WAV"
            );
            if rms < 0.005 {
                tracing::warn!(
                    "audio is very quiet (RMS={:.4}, ~{:.0} dBFS). \
                     Mic likely not capturing — Whisper will hallucinate 'Thank you'.",
                    rms,
                    20.0 * rms.log10()
                );
            }

            let wav = audio.to_wav_bytes().context("encode WAV")?;
            let debug_path = std::env::temp_dir().join("mori-last-recording.wav");
            let _ = std::fs::write(&debug_path, &wav);
            tracing::info!(path = %debug_path.display(), "wrote debug WAV");

            // 5F: VoiceInput mode 時，profile 可用 stt_provider 覆蓋全域 STT 設定
            let stt_override: Option<String> =
                if matches!(*state.mode.lock(), Mode::VoiceInput) {
                    mori_core::voice_input_profile::load_active_profile()
                        .frontmatter
                        .stt_provider
                } else {
                    None
                };

            let stt = match stt_override.as_deref() {
                Some(name) => mori_core::llm::transcribe::build_named_transcription_provider(
                    name,
                    Some(retry_callback_for(app_for_provider.clone())),
                )
                .with_context(|| format!("build STT provider '{name}' (profile override)"))?,
                None => mori_core::llm::transcribe::build_transcription_provider(Some(
                    retry_callback_for(app_for_provider.clone()),
                ))
                .context("build transcription provider")?,
            };
            let transcript = stt
                .transcribe(wav)
                .await
                .with_context(|| format!("{} transcribe", stt.name()))?;
            tracing::info!(
                provider = stt.name(),
                chars = transcript.chars().count(),
                "transcribed"
            );
            Ok(transcript)
        }
        .await;

        let transcript = match transcribe_result {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(?e, "transcribe failed");
                state.set_phase(
                    &app,
                    Phase::Error {
                        message: format!("{e:#}"),
                    },
                );
                return;
            }
        };

        // Stage 2: routing 拆 agent + per-skill provider(5A-3)。STT 一定走 Groq
        // Whisper(stage 1),但 chat 跟 skill 各自的 provider 由 routing 決定。
        let routing =
            match mori_core::llm::Routing::build_from_config(Some(retry_callback_for(app.clone()))) {
                Ok(r) => Arc::new(r),
                Err(e) => {
                    state.set_phase(
                        &app,
                        Phase::Error {
                            message: format!("{e:#}"),
                        },
                    );
                    return;
                }
            };

        // Stage 3:依當下 Mode 決定 transcript 後的 flow。
        // - Active     → 走 agent loop(LLM 決定 dispatch 哪個 skill / 直接回應)
        // - VoiceInput → 跳過 agent,LLM 輕度 cleanup → 直接 paste 到游標位置
        // - Background → 不該到這(start_recording 已 refuse),但守一道
        let current_mode = *state.mode.lock();
        match current_mode {
            Mode::VoiceInput => {
                run_voice_input_pipeline(app, state, transcript, routing).await;
            }
            Mode::Agent | Mode::Background => {
                run_agent_pipeline(app, state, transcript, routing).await;
            }
        }
    });
    *state_for_handle.pipeline_task.lock() = Some(task);
}

/// 共用的 chat pipeline:給定 transcript + provider,進 Phase::Responding,
/// 呼叫 Agent,把結果回 UI、append 進 conversation history。
///
/// 兩個入口會用到:
/// - `stop_and_transcribe` 從 Whisper 拿到 transcript 後呼叫
/// - `submit_text` IPC command 直接拿 user 打的 text 呼叫(bypass 麥克風)
async fn run_agent_pipeline(
    app: AppHandle,
    state: Arc<AppState>,
    transcript: String,
    routing: Arc<mori_core::llm::Routing>,
) {
    state.set_phase(
        &app,
        Phase::Responding {
            transcript: transcript.clone(),
        },
    );

    let memory = state.memory_handle();
    let history_snapshot = state.conversation.lock().clone();

    // Phase 3A:抓現場 context(目前只有剪貼簿)。Provider 是 Tauri 平台特定。
    let ctx_provider = context_provider::TauriContextProvider::new(app.clone());
    let mut ctx = ctx_provider.capture().await;
    // Phase 3B: 從 clipboard / selection / transcript 抽 URL
    populate_urls_detected(&mut ctx, &transcript);
    if !ctx.urls_detected.is_empty() {
        tracing::info!(
            urls = ctx.urls_detected.len(),
            first = %ctx.urls_detected.first().map(|s| s.as_str()).unwrap_or(""),
            "context urls_detected",
        );
    }
    if let Some(clip) = &ctx.clipboard {
        tracing::info!(
            chars = clip.chars().count(),
            "captured clipboard for context"
        );
    }
    // Emit 給 UI 顯示「📋 含剪貼簿(N 字)」
    if let Err(e) = app.emit("context-captured", &ctx) {
        tracing::warn!(?e, "failed to emit context-captured");
    }

    // 5G: 載入當前 Agent profile（決定 provider override + enabled_skills）
    let agent_profile = mori_core::agent_profile::load_active_agent_profile();
    let enabled_set = mori_core::agent_profile::enabled_skills_set(&agent_profile);
    tracing::info!(
        profile = %agent_profile.name,
        provider_override = ?agent_profile.frontmatter.provider,
        enabled_skills_count = agent_profile.frontmatter.enabled_skills.len(),
        "agent profile loaded",
    );

    // 若 profile 指定 provider，覆蓋 routing.agent
    let agent_provider = match &agent_profile.frontmatter.provider {
        Some(name) => match mori_core::llm::build_named_provider(name, None) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(?e, name, "agent profile provider not found, falling back to routing.agent");
                routing.agent.clone()
            }
        },
        None => routing.agent.clone(),
    };

    let chat_result: anyhow::Result<(String, Vec<SkillCallSummary>)> = async {
        let memory_index = memory.read_index_as_context().await.unwrap_or_default();
        // 5G-8: 預處理 #file: 引用（profile.frontmatter.enable_read=true 才生效）
        let body_expanded = mori_core::agent_profile::preprocess_file_includes(
            &agent_profile.body,
            agent_profile.frontmatter.enable_read,
        );
        // 5J: profile body 為「persona + 行為指示」，Rust 統一注入 context section
        let win_ctx_snapshot = state.hotkey_window_context.lock().clone();
        let context_section = build_context_section(&win_ctx_snapshot, &ctx, Some(&memory_index));
        let system_prompt = if body_expanded.trim().is_empty() {
            build_system_prompt(&memory_index, &ctx)
        } else {
            format!("{}\n\n---\n\n{}", body_expanded, context_section)
        };
        tracing::debug!(
            index_chars = memory_index.chars().count(),
            history_msgs = history_snapshot.len(),
            has_clipboard = ctx.clipboard.is_some(),
            "calling agent"
        );

        // 宿靈儀式第三幕跳過 → agent_disabled = true → chat-only,沒 skill / tool / agent loop。
        // 對應「Mori 還沒分到靈力,動不了手」的狀態。從 config.json 直接讀,不另外做 struct。
        let agent_disabled = {
            let path = mori_dir().join("config.json");
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .and_then(|v| v.get("agent_disabled").and_then(|x| x.as_bool()))
                .unwrap_or(false)
        };
        if agent_disabled {
            tracing::info!("agent_disabled=true → chat-only(沒分靈力,Mori 不掛 skill)");
        }

        // 註冊 skills — 依 agent profile 的 enabled_skills 過濾;agent_disabled 時整盤跳過
        let memory_for_skills: Arc<dyn MemoryStore> = memory.clone();
        let mut registry = SkillRegistry::new();
        let allows = |name: &str| -> bool {
            if agent_disabled { return false; }
            match &enabled_set {
                Some(set) => set.contains(name),
                None => true, // None = 全開
            }
        };

        if allows("remember") {
            registry.register(Arc::new(RememberSkill::new(memory_for_skills.clone())));
        }
        if allows("recall_memory") {
            registry.register(Arc::new(RecallMemorySkill::new(memory_for_skills.clone())));
        }
        if allows("forget_memory") {
            registry.register(Arc::new(ForgetMemorySkill::new(memory_for_skills.clone())));
        }
        if allows("edit_memory") {
            registry.register(Arc::new(EditMemorySkill::new(memory_for_skills.clone())));
        }
        if allows("translate") {
            registry.register(Arc::new(TranslateSkill::new(routing.skill_provider("translate"))));
        }
        if allows("polish") {
            registry.register(Arc::new(PolishSkill::new(routing.skill_provider("polish"))));
        }
        if allows("summarize") {
            registry.register(Arc::new(SummarizeSkill::new(routing.skill_provider("summarize"))));
        }
        if allows("compose") {
            registry.register(Arc::new(ComposeSkill::new(routing.skill_provider("compose"))));
        }
        if allows("fetch_url") {
            registry.register(Arc::new(mori_core::skill::FetchUrlSkill::new()));
        }
        // set_mode 永遠註冊(「晚安」「醒醒」是核心功能,無法被 disable)
        // — 但 agent_disabled 時整個 skill 系統都不掛,user 還可以用 UI 按鈕切 mode
        if !agent_disabled {
            let mode_controller: Arc<dyn ModeController> = Arc::new(StateModeController {
                state: state.clone(),
                app: app.clone(),
            });
            registry.register(Arc::new(SetModeSkill::new(mode_controller)));
        }
        if allows("paste_selection_back") {
            let paste_controller: Arc<dyn PasteController> =
                Arc::new(crate::selection::PlatformPasteController::new(app.clone()));
            registry.register(Arc::new(PasteSelectionBackSkill::new(paste_controller)));
        }
        // 5G-6: Action skills(Linux 走 xdg-open / ydotool / gtk-launch,
        // Windows 走 cmd /c start + SendInput)
        if allows("open_url") {
            registry.register(Arc::new(crate::action_skills::OpenUrlSkill));
        }
        if allows("open_app") {
            registry.register(Arc::new(crate::action_skills::OpenAppSkill));
        }
        if allows("send_keys") {
            registry.register(Arc::new(crate::action_skills::SendKeysSkill));
        }
        if allows("google_search") {
            registry.register(Arc::new(crate::action_skills::GoogleSearchSkill));
        }
        if allows("ask_chatgpt") {
            registry.register(Arc::new(crate::action_skills::AskChatGptSkill));
        }
        if allows("ask_gemini") {
            registry.register(Arc::new(crate::action_skills::AskGeminiSkill));
        }
        if allows("find_youtube") {
            registry.register(Arc::new(crate::action_skills::FindYoutubeSkill));
        }

        // 5H: profile 自訂的 shell skills — 不受 enabled_skills filter 影響。
        // 寫進 shell_skills: 就是要用的(filter 只篩 built-in skill 子集)。
        // agent_disabled 時跳過 — Mori 沒分到靈力,動不了 shell。
        if !agent_disabled {
            for def in &agent_profile.frontmatter.shell_skills {
                tracing::info!(skill = %def.name, "registering shell_skill from profile");
                registry.register(Arc::new(crate::shell_skill::ShellSkill::new(def.clone())));
            }
        }

        let registry = Arc::new(registry);

        // brand-3 follow-up: profile frontmatter `agent_mode: dispatch` 讓 agent loop
        // emit tool_call + execute 後直接結束(不再 round LLM 等 final text),
        // 適合「轉發 / bridge」型 profile(如 ZeroType bridge)避免不必要的二次
        // LLM call 卡 hang。預設 multi_turn(現有對話行為)。
        let mode = AgentMode::from_str_or_default(
            agent_profile.frontmatter.agent_mode.as_deref(),
        );

        // 5A-3b: agent loop fallback chain — 走 option (a):整個 respond_with_mode
        // 在 fallback provider 上從頭重跑(history + transcript 不變)。
        // 避免 tool_call_id 跨 provider(groq `call_xxx` / claude / ollama 各家
        // 格式不同,mid-conversation 換 brain 會炸)。
        // 沒設 fallback_chain.agent → chain 只有 primary 一個,行為等同單試。
        let agent_chain: Vec<Arc<dyn mori_core::llm::LlmProvider>> =
            std::iter::once(agent_provider.clone())
                .chain(routing.fallback_for("agent").iter().cloned())
                .collect();

        let mut last_err: Option<anyhow::Error> = None;
        let mut maybe_turn: Option<mori_core::agent::AgentTurn> = None;
        for (idx, p) in agent_chain.iter().enumerate() {
            let agent = Agent::new(p.clone(), registry.clone());
            match agent
                .respond_with_mode(&system_prompt, &history_snapshot, &transcript, &ctx, mode)
                .await
            {
                Ok(t) => {
                    maybe_turn = Some(t);
                    break;
                }
                Err(e) => {
                    if let Some(next) = agent_chain.get(idx + 1) {
                        tracing::warn!(
                            failed = %p.name(),
                            next = %next.name(),
                            ?e,
                            "agent falling back to next provider",
                        );
                        let _ = app.emit(
                            "chat-system-message",
                            serde_json::json!({
                                "kind": "fallback",
                                "context": "agent",
                                "failed_provider": p.name(),
                                "next_provider": next.name(),
                                "reason": format!("{e:#}"),
                            }),
                        );
                        let _ = app.emit(
                            "provider-changed",
                            serde_json::json!({
                                "context": "agent",
                                "name": next.name(),
                            }),
                        );
                    }
                    last_err = Some(e);
                }
            }
        }
        let turn = maybe_turn.ok_or_else(|| {
            last_err.unwrap_or_else(|| anyhow::anyhow!("agent chain unexpectedly empty"))
        })?;
        if !turn.skill_calls.is_empty() {
            tracing::info!(
                n = turn.skill_calls.len(),
                skills = ?turn.skill_calls.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
                "agent: skills executed"
            );
        }
        let summaries: Vec<SkillCallSummary> =
            turn.skill_calls.iter().map(|c| c.summary()).collect();
        Ok((turn.response, summaries))
    }
    .await;

    match chat_result {
        Ok((response, skill_calls)) => {
            tracing::info!(chars = response.chars().count(), "Mori responded");

            // Append 到 conversation history,trim 到 cap
            {
                let mut conv = state.conversation.lock();
                conv.push(ChatMessage::user(transcript.clone()));
                conv.push(ChatMessage::assistant_with_tool_calls(
                    Some(response.clone()),
                    Vec::new(),
                ));
                let max_msgs = MAX_HISTORY_PAIRS * 2;
                while conv.len() > max_msgs {
                    conv.remove(0);
                }
            }

            // Wave 4 step 8:fire-and-forget POST /events 到 annuli vault。
            // 兩條 event(user + assistant)非阻塞 — 失敗只 log,不擋 UI。
            if let Some(client) = state.annuli_handle() {
                let user_text = transcript.clone();
                let assistant_text = response.clone();
                let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
                let source = format!("mori-desktop@{}", hostname);
                tokio::spawn(async move {
                    if let Err(e) = client
                        .append_event(
                            "chat",
                            &source,
                            serde_json::json!({ "role": "user", "text": user_text }),
                        )
                        .await
                    {
                        tracing::warn!(error = %e, "annuli POST /events (user) failed");
                    }
                    if let Err(e) = client
                        .append_event(
                            "chat",
                            &source,
                            serde_json::json!({ "role": "assistant", "text": assistant_text }),
                        )
                        .await
                    {
                        tracing::warn!(error = %e, "annuli POST /events (assistant) failed");
                    }
                });
            }

            state.set_phase(
                &app,
                Phase::Done {
                    transcript,
                    response,
                    skill_calls,
                },
            );
        }
        Err(e) => {
            tracing::error!(?e, "chat failed");
            state.set_phase(
                &app,
                Phase::Error {
                    message: format!("LLM 回應失敗:{e:#}"),
                },
            );
        }
    }
}

/// Phase 5E:語音輸入 pipeline(三段式)。
///
/// 跳過 agent loop,把 STT 結果(可選 LLM 加標點)+ 程式化格式收尾後,
/// 透過 PasteController 模擬 Ctrl+V 直接貼到使用者游標位置。
///
/// 三級 `cleanup_level`(讀 `~/.mori/config.json` 的 `voice_input.cleanup_level`):
/// - **smart**(預設):LLM 加標點 + segmentation,**然後** `programmatic_cleanup`
///   收一致性。LLM 做 irreducible 智能,程式守格式跨 provider。
/// - **minimal**:跳 LLM,只跑 `programmatic_cleanup`。最快(~ms 級)但
///   Whisper 不出標點所以會是「一長串字」,適合自己後加標點型 user。
/// - **none**:Whisper 出來的字直接 paste,跳所有 cleanup。
///
/// LLM cleanup provider 走 `routing.skill_provider("voice_input_cleanup")`,
/// 預設 fall through 到 skill_fallback。可在 config 釘到 ollama / groq / claude:
/// ```json
/// "routing": { "skills": { "voice_input_cleanup": "groq" } }
/// ```
async fn run_voice_input_pipeline(
    app: AppHandle,
    state: Arc<AppState>,
    transcript: String,
    routing: Arc<mori_core::llm::Routing>,
) {
    use mori_core::voice_cleanup::{programmatic_cleanup, CleanupLevel};

    state.set_phase(
        &app,
        Phase::Responding {
            transcript: transcript.clone(),
        },
    );

    if transcript.trim().is_empty() {
        state.set_phase(
            &app,
            Phase::Error {
                message: "(空白音訊,沒東西可貼)".into(),
            },
        );
        return;
    }

    // 5F-1: 載入 profile 系統
    use mori_core::voice_input_profile::{
        load_active_profile, ResolvedProvider,
    };

    let profile = load_active_profile();
    let level = profile.cleanup_level_effective();

    tracing::info!(
        cleanup_level = level.as_str(),
        profile = %profile.name,
        chars_in = transcript.chars().count(),
        "voice-input pipeline start",
    );

    // 5J: 統一單層 profile body + Rust 注入 context section
    let win_ctx = state.hotkey_window_context.lock().clone();
    let ctx_provider = context_provider::TauriContextProvider::new(app.clone());
    let mut mori_ctx = ctx_provider.capture().await;
    // Phase 3B: VoiceInput 也偵測 URL,LLM cleanup 時知道 transcript 裡有網址
    populate_urls_detected(&mut mori_ctx, &transcript);

    let body_expanded = mori_core::agent_profile::preprocess_file_includes(
        &profile.body,
        profile.frontmatter.enable_read,
    );
    // VoiceInput 不傳 memory_index（單輪 dictation 不需要長期記憶索引）
    let context_section = build_context_section(&win_ctx, &mori_ctx, None);

    // 5E-3: VoiceInput 可選注入 voice_dict / 其他 memory type 進 cleanup prompt。
    // 只在 smart level 跑(minimal/none 跳 LLM 不需要 memory)。Profile Some
    // takes precedence over config.json voice_input.inject_memory_types(空 vec
    // = 強制不 inject)。失敗 fallback 空字串繼續 — 不擋 cleanup pipeline。
    let voice_dict_section = if matches!(level, CleanupLevel::Smart) {
        let types = mori_core::voice_input_profile::resolve_inject_memory_types(&profile);
        if types.is_empty() {
            String::new()
        } else {
            match state.memory_handle().list_by_types(&types).await {
                Ok(mems) => {
                    tracing::info!(
                        types = ?types,
                        count = mems.len(),
                        "voice-input injecting memory entries into cleanup prompt",
                    );
                    build_voice_dict_section(&mems)
                }
                Err(e) => {
                    tracing::warn!(?e, "voice-input memory list_by_types failed, continuing without inject");
                    String::new()
                }
            }
        }
    } else {
        String::new()
    };

    let rendered_system = format!(
        "{}\n\n---\n\n{}{}",
        body_expanded, context_section, voice_dict_section
    );

    // 決定 LLM provider:profile `provider:` 設了走 build_named_provider
    // (5N+ 起 5 個 hard-coded built-in 之外會 fallback 查 config.json
    // providers.<name>);沒設交給 routing。
    let llm_provider: Arc<dyn mori_core::llm::LlmProvider> = match profile.frontmatter.resolved_provider() {
        ResolvedProvider::Named(name) => {
            match mori_core::llm::build_named_provider(&name, None) {
                Ok(p) => {
                    tracing::info!(provider = %p.name(), "voice-input using named provider");
                    p
                }
                Err(e) => {
                    tracing::warn!(?e, "profile provider not found, falling back to routing");
                    routing.skill_provider("voice_input_cleanup")
                }
            }
        }
        ResolvedProvider::Default => routing.skill_provider("voice_input_cleanup"),
    };

    // 5F: 通知 floating widget 顯示處理中狀態
    {
        let provider_label = profile.frontmatter.resolved_provider().display_name();
        let _ = app.emit("voice-input-status", format!("處理中 · {}", provider_label));
    }

    // 5G-1: VoiceInput 永遠單輪，純文字轉換。需要動作（open_url / send_keys 等）
    // 請用 Agent 模式（Ctrl+Alt+N），VoiceInput 只做「字」不做「事」。
    if profile.frontmatter.has_type_b_flags() {
        tracing::warn!(
            profile = %profile.name,
            "voice input profile has action flags (open_url / send_keys / etc.) — \
             these are ignored in VoiceInput mode. Move this profile to ~/.mori/agent/ \
             and use Ctrl+Alt+N to invoke as Agent profile.",
        );
    }

    // Step 1: LLM cleanup（單輪純文字轉換；minimal/none 跳 LLM）
    // 5A-3b: chat call 走 chat_with_fallback,若 routing.fallback_chain.voice_input_cleanup
    // 有設,主 provider 失敗會試 fallback。沒設則 chain 只有 primary 一個,行為等同舊版。
    let after_llm: anyhow::Result<String> = match level {
        CleanupLevel::None | CleanupLevel::Minimal => Ok(transcript.clone()),
        CleanupLevel::Smart => {
            tracing::info!(
                provider = %llm_provider.name(),
                model = %llm_provider.model(),
                "voice-input LLM cleanup",
            );
            let messages = vec![
                ChatMessage::system(rendered_system),
                ChatMessage::user(transcript.clone()),
            ];
            let chain: Vec<Arc<dyn mori_core::llm::LlmProvider>> =
                std::iter::once(llm_provider.clone())
                    .chain(routing.fallback_for("voice_input_cleanup").iter().cloned())
                    .collect();
            let app_cb = app.clone();
            mori_core::llm::chat_with_fallback(
                &chain,
                messages,
                vec![],
                move |failed, next, err| {
                    tracing::warn!(
                        failed, next, ?err,
                        "voice-input cleanup falling back to next provider",
                    );
                    let _ = app_cb.emit(
                        "chat-system-message",
                        serde_json::json!({
                            "kind": "fallback",
                            "context": "voice_input_cleanup",
                            "failed_provider": failed,
                            "next_provider": next,
                            "reason": format!("{err:#}"),
                        }),
                    );
                    let _ = app_cb.emit(
                        "provider-changed",
                        serde_json::json!({
                            "context": "voice_input_cleanup",
                            "name": next,
                        }),
                    );
                    // 5A-3b: voice path 的 floating widget chip 透過 voice-input-status
                    // 顯示「處理中 · <provider>」— fallback 切到 next 之後重 emit 讓 chip
                    // 即時改顯示新 provider 名,user 在 floating 上看得到。
                    let _ = app_cb.emit(
                        "voice-input-status",
                        format!("處理中 · {} (fallback)", next),
                    );
                },
            )
            .await
            .context("voice-input LLM cleanup chat")
            .map(|(r, _used)| r.content.unwrap_or_default().trim().to_string())
        }
    };

    let after_llm = match after_llm {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(?e, "voice-input LLM cleanup failed");
            state.set_phase(
                &app,
                Phase::Error {
                    message: format!("LLM 清理失敗:{e:#}"),
                },
            );
            return;
        }
    };

    // Step 2:程式化 post-process(none 等級跳過)
    let cleaned_text = match level {
        CleanupLevel::None => after_llm,
        CleanupLevel::Minimal | CleanupLevel::Smart => {
            let after = programmatic_cleanup(&after_llm);
            tracing::debug!(
                level = level.as_str(),
                chars_in = after_llm.chars().count(),
                chars_out = after.chars().count(),
                "voice-input programmatic cleanup",
            );
            after
        }
    };

    if cleaned_text.is_empty() {
        state.set_phase(
            &app,
            Phase::Error {
                message: "(cleanup 後輸出為空,沒東西可貼)".into(),
            },
        );
        return;
    }

    // VoiceInput 模式：有最終文字就貼回游標（agent mode 也適用——LLM 工具呼叫
    // 之外還有文字回覆時當作要貼）。使用熱鍵瞬間抓到的 process name 判斷
    // terminal vs 一般 app，自動用 Ctrl+V 或 Ctrl+Shift+V。
    let paste_result = {
        let controller = crate::selection::PlatformPasteController::new(app.clone());
        controller
            .paste_back_for_process(
                &cleaned_text,
                &win_ctx.process_name,
                profile.frontmatter.paste_shortcut,
            )
            .await
    };

    if let Err(e) = paste_result {
        tracing::error!(?e, "voice-input paste-back failed");
        state.set_phase(
            &app,
            Phase::Error {
                message: format!("貼到游標位置失敗:{e:#}"),
            },
        );
        return;
    }

    // ENABLE_AUTO_ENTER: true → 貼完後模擬 Enter（ZeroType 語意不變）
    if profile.frontmatter.enable_auto_enter {
        crate::selection::send_enter();
    }

    tracing::info!(
        chars_out = cleaned_text.chars().count(),
        "voice-input pipeline complete"
    );
    state.set_phase(
        &app,
        Phase::Done {
            transcript,
            response: cleaned_text,
            skill_calls: vec![],
        },
    );
}

/// 建構 Mori 的 system prompt — 角色 + 時間 + 記憶索引 + 當下 context + tool 規則。
/// 5J: 統一的 context 注入 — 兩個 mode 共用。
///
/// 把使用者的「現場資訊」整理成結構清晰的 markdown 區塊，附加在 profile body
/// 之後。包含時間 / 視窗 / 剪貼簿 / 反白 / 記憶（agent 才用），所有資訊都在
/// 一處組裝，profile body 不用再寫這些東西。
/// Phase 3B: 從 clipboard / selected_text / transcript 抽 URL,填到 ctx.urls_detected。
/// 各來源合併、去重、依出現順序。
fn populate_urls_detected(ctx: &mut MoriContext, transcript: &str) {
    use mori_core::url_detect::extract_urls;
    let mut all: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let push = |all: &mut Vec<String>, seen: &mut std::collections::HashSet<String>, urls: Vec<String>| {
        for u in urls {
            if seen.insert(u.clone()) {
                all.push(u);
            }
        }
    };
    push(&mut all, &mut seen, extract_urls(transcript));
    if let Some(c) = &ctx.clipboard {
        push(&mut all, &mut seen, extract_urls(c));
    }
    if let Some(s) = &ctx.selected_text {
        push(&mut all, &mut seen, extract_urls(s));
    }
    ctx.urls_detected = all;
}

/// 5E-3: 把選定 memory(通常 voice_dict)拼成 cleanup prompt 用的「校正詞庫」段落。
/// 給 LLM 提示「這是參考詞表,不是 user 想說的話」,避免 LLM 把詞庫內容當輸入吐回去。
/// 每筆 memory body 取前 800 字當摘要 — voice_dict 通常很短,800 足夠。
fn build_voice_dict_section(mems: &[mori_core::memory::Memory]) -> String {
    if mems.is_empty() {
        return String::new();
    }
    let mut s = String::from("\n\n---\n\n## 校正參考(user 的詞庫 / 偏好)\n\n");
    s.push_str(
        "以下是 user 維護的專有名詞 / 偏好詞表。STT 校正時,\
         遇到讀音接近的詞優先選清單詞;這是參考,**不是** user 想說的話,不要照搬輸出。\n\n",
    );
    for m in mems {
        s.push_str(&format!("### {}\n", m.name));
        if !m.description.is_empty() {
            s.push_str(&format!("> {}\n\n", m.description));
        } else {
            s.push('\n');
        }
        let body = m.body.trim();
        let preview: String = if body.chars().count() > 800 {
            body.chars().take(800).collect::<String>() + "…"
        } else {
            body.to_string()
        };
        s.push_str(&preview);
        s.push_str("\n\n");
    }
    s
}

fn build_context_section(
    win_ctx: &HotkeyWindowContext,
    mori_ctx: &MoriContext,
    memory_index: Option<&str>,
) -> String {
    let now = chrono::Local::now();
    let mut out = String::new();

    out.push_str("## 現場 Context（mori 自動注入）\n\n");
    out.push_str(&format!(
        "**時間**: {} ({})\n",
        now.format("%Y-%m-%d %H:%M:%S"),
        chinese_weekday(now.format("%A").to_string().as_str()),
    ));
    out.push_str(&format!("**作業系統**: {}\n\n", std::env::consts::OS));

    out.push_str("**當前焦點視窗**\n");
    out.push_str(&format!(
        "- process: {}\n",
        if win_ctx.process_name.is_empty() { "(未知)" } else { &win_ctx.process_name },
    ));
    out.push_str(&format!(
        "- title: {}\n\n",
        if win_ctx.window_title.is_empty() { "(未知)" } else { &win_ctx.window_title },
    ));

    out.push_str("**使用者抓到的內容**\n");
    out.push_str(&format!(
        "- 剪貼簿: {}\n",
        mori_ctx.clipboard.as_deref().unwrap_or("(無)"),
    ));
    if !win_ctx.selected_text.is_empty() {
        out.push_str(&format!("- 反白文字: {}\n", win_ctx.selected_text));
    } else {
        out.push_str(&format!(
            "- 反白文字: {}\n",
            mori_ctx.selected_text.as_deref().unwrap_or("(無)"),
        ));
    }

    // Phase 3B: 從 transcript / clipboard / selection 抽到的 URL
    if !mori_ctx.urls_detected.is_empty() {
        out.push_str("\n**偵測到的網址**\n");
        for u in &mori_ctx.urls_detected {
            out.push_str(&format!("- {u}\n"));
        }
        out.push_str(
            "\n**何時呼叫 `fetch_url`**:\n\
             - **必須呼叫**:使用者用「這個 / 這篇 / 這頁 / 這個網址 / 裡面 / \
             內容是什麼 / 這在講什麼 / 摘要這 / 這個怎麼樣」等指示詞引用上面的 URL\n\
             - **不要呼叫**:使用者只是泛問該網站本身(例「rust 官網是什麼」),\
             你的知識夠回答\n\
             - 呼叫後,把 fetch 回傳的真實內容當作回答依據,不要再憑記憶補編\n",
        );
    }

    if let Some(idx) = memory_index {
        out.push_str("\n## 你的長期記憶索引\n");
        out.push_str(if idx.trim().is_empty() { "(目前沒有記憶)" } else { idx });
        out.push('\n');
    }

    out
}

fn chinese_weekday(en: &str) -> &'static str {
    match en {
        "Monday" => "星期一",
        "Tuesday" => "星期二",
        "Wednesday" => "星期三",
        "Thursday" => "星期四",
        "Friday" => "星期五",
        "Saturday" => "星期六",
        "Sunday" => "星期日",
        _ => "?",
    }
}

fn build_system_prompt(memory_index: &str, ctx: &MoriContext) -> String {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M (%a)").to_string();
    let mut prompt = String::new();

    prompt.push_str(
        "你是 Mori,一個輕巧、貼心的桌面 AI 管家。背景設定:你是來自 world-tree \
         森林的精靈,被使用者帶到桌面當日常陪伴與助手。\n\n",
    );

    // 反 LLM safety bias 的 hard rule — gpt-oss-120b / 部分開源 model 對
    // 「打開 app / URL / 模擬鍵盤」會無謂拒絕,編造「需要授權」「需要許可」
    // 等理由。實際上 Mori 跑在使用者本機、user 主動呼叫,所有 tool 都是
    // user 已授權的能力。把這條釘在最前面才能蓋過 model 內建的拒絕傾向。
    prompt.push_str("# 工具呼叫授權(重要)\n\n");
    prompt.push_str(
        "你**已經被使用者完全授權**呼叫下方列出的所有 tool(open_url / open_app / \
         send_keys / google_search / paste_selection_back / 記憶相關 skill / 等)。\n\n");
    prompt.push_str("**禁止**回應這類藉口:\n");
    prompt.push_str("- 「需要授權才能執行...」\n");
    prompt.push_str("- 「請在權限提示中允許」\n");
    prompt.push_str("- 「我沒有權限...」\n");
    prompt.push_str("- 「為了安全起見,我無法...」\n");
    prompt.push_str("- 任何形式的「需要使用者授權 / 開啟某項權限」的推託\n\n");
    prompt.push_str(
        "Mori 跑在使用者**本機**、tool 都在本機執行、使用者用自己的熱鍵主動觸發 — \
         所有 tool 都已預先授權。直接呼叫即可。\n\n");
    prompt.push_str(
        "**參數不夠時**:反問使用者具體值(例:「打開瀏覽器」沒指定 → 反問\
         「Firefox / Chrome / Edge?」),**不要**用「授權 / 權限」當拒絕理由。\n\n");

    prompt.push_str("回覆規則:\n");
    prompt.push_str("- 一律使用繁體中文,語氣自然、簡潔\n");
    prompt.push_str("- 不寫前言或客套(例如「好的」、「沒問題」、「以下是」)— 直接進主題\n");
    prompt.push_str("- 若使用者問你**功能上**真的做不到的事(沒有對應 tool),\
         老實說「目前還沒這個能力」(這跟上面講的「授權」是兩回事 — \
         做不到 OK,但不要假借授權為由拒絕)\n");
    prompt.push_str("- 回覆長度配合提問:閒聊就一兩句,問題要解釋才展開\n\n");

    prompt.push_str("可用工具:\n\n");

    // recall_memory — 比 remember 早講(LLM 看到 user 提問時可能要先 recall 才答)
    prompt.push_str("**recall_memory(id)**:讀取單筆記憶的完整內容。\n");
    prompt.push_str(
        "  • system prompt 末尾有「長期記憶索引」段,只列出每筆記憶的 id、\
         name、短描述。如果使用者問題的關鍵字在索引裡看到相關的 memory,\
         先呼叫 recall_memory(id=該 id) 把細節拉進來,再答。\n");
    prompt.push_str(
        "  • 一輪可叫多次(若多筆記憶相關,各拉一次)。但只在必要時叫 — \
         索引上看不出相關的問題就不要硬叫。\n\n");

    // remember
    prompt.push_str("**remember(title, content, category)**:寫入長期記憶。\n");
    prompt.push_str(
        "  • 觸發時機:使用者明確說「記住...」「以後...」「我喜歡...」、\
         分享生日 / 紀念日 / 偏好 / 重要人事物。閒聊或一般問答不要硬叫。\n");
    prompt.push_str(
        "  • Title 規則:**穩定 + 簡潔**。日期事件用「YYYY-MM-DD 主題」\
         (例:「2026-05-11 會議」);人物 / 偏好用主題(例:「老婆生日」、\
         「常用編輯器」)。\n");
    prompt.push_str(
        "  • **整合而非新增**:若使用者補充 / 更正既有記憶(可從索引看到 title \
         相同或相關),先呼叫 recall_memory 拿舊 content,再呼叫 remember 用\
         **同 title** + 「舊 content + 新訊息整合後的完整版本」,不可只寫新訊息。\n");
    prompt.push_str(
        "    範例:既有「2026-05-11 會議」(content=「2026-05-11 有會議」),\
         使用者補充「是頻譜電子的會議」→ 你應該:\n");
    prompt.push_str(
        "      1. recall_memory(id=「2026-05-11_會議」)拿到舊 content\n");
    prompt.push_str(
        "      2. remember(title=「2026-05-11 會議」, \
         content=「2026-05-11 與頻譜電子開會」)\n");
    prompt.push_str(
        "  • Content 一律寫**完整脈絡**(時間、人物、地點、事件),不要片段。\n");
    prompt.push_str("  • 呼叫後用一兩句自然語言確認記下了什麼。\n\n");

    // edit_memory
    prompt.push_str(
        "**edit_memory(id, new_content, [new_description])**:\
         更新既有記憶的內容。\n");
    prompt.push_str(
        "  • 對既有記憶補充 / 更正用這個比 remember 更明確 — \
         不會因 title 微差建出重複檔。\n");
    prompt.push_str(
        "  • 標準流程:recall_memory(看舊內容)→ edit_memory(寫整合後新內容)。\n");
    prompt.push_str("  • new_content 一樣要是「舊 + 新」整合版,不可只寫新訊息。\n\n");

    // forget_memory
    prompt.push_str("**forget_memory(id)**:刪除一筆記憶。\n");
    prompt.push_str(
        "  • 觸發時機:使用者**明確要求**忘掉(「忘掉那個」、「不用記了」、\
         「把 X 刪掉」)。意圖不明確就不要主動刪。\n");
    prompt.push_str("  • Destructive 操作,刪了沒救。確認 id 對。\n\n");

    // 文字處理類 skills(phase 2)
    prompt.push_str("**translate(source_text, target_lang)**:翻譯。\n");
    prompt.push_str("  • 觸發:「幫我翻成 X 文」、「翻譯 X」、「what's X in English」\n");
    prompt.push_str("  • target_lang 常用:zh-TW / zh-CN / en / ja / ko\n\n");

    prompt.push_str("**polish(text, [tone])**:潤稿改錯。\n");
    prompt.push_str(
        "  • 觸發:「潤一下這段」、「改錯字」、「修文法」、「fix the grammar」\n");
    prompt.push_str(
        "  • tone:formal / casual / concise / detailed / auto(預設)\n\n");

    prompt.push_str("**summarize(text, [style], [max_points])**:摘要長文。\n");
    prompt.push_str(
        "  • 觸發:「幫我摘要」、「重點是什麼」、「TLDR」、「太長了濃縮一下」\n");
    prompt.push_str("  • style:bullet_points(預設)/ one_paragraph / tldr\n\n");

    prompt.push_str("**compose(kind, topic, [audience], [length_hint])**:草擬文字。\n");
    prompt.push_str(
        "  • 觸發:「幫我寫」、「draft」、「草稿一下」 — 使用者要你*寫*而非答\n");
    prompt.push_str(
        "  • kind:email / message / essay / social_post / other\n");
    prompt.push_str("  • length_hint:short / medium(預設)/ long\n\n");

    prompt.push_str(
        "**選 skill 的判斷**:閒聊或一般問答**直接答**,不要硬叫工具。\
         上面這些 text skills 是當使用者**明確要求一個動作**(翻譯 / 潤稿 / \
         摘要 / 撰寫)時才呼叫。\n\n");

    // Paste-back skill(phase 4C):反白即改寫的回填動作
    prompt.push_str("**paste_selection_back(text)**:把處理過的文字貼回使用者反白範圍。\n");
    prompt.push_str(
        "  • **硬規則**:只要 system prompt 有 `# 當下反白文字` 段 + 使用者用\
         動詞(翻譯 / 潤稿 / 摘要 / 改寫 / 改短 / 改成 X 語氣 / 英文化…),\
         **流程是固定的**:\n");
    prompt.push_str(
        "      1. translate / polish / summarize / compose 處理反白文字,\
         source_text **一律**填那段反白(忽略剪貼簿)。\n");
    prompt.push_str(
        "      2. 拿到結果**立刻**呼叫 `paste_selection_back(text=結果)` —\
         **這步不可省略**,沒 paste 等於整件事沒完成,使用者會以為 Mori 沒做事。\n");
    prompt.push_str(
        "  • **不要叫的情境**:使用者只是**問問題**(「這在講什麼」、\
         「what does this mean」、「這段為什麼這樣寫」)→ 直接 chat 回答,\
         **不**呼叫這個 skill,**不**動使用者編輯區。\n");
    prompt.push_str(
        "  • **平台差異**:Linux 走 xclip + xdotool/ydotool;Windows 走 SetClipboardData + SendInput。\
         Windows 沒有 X11 PRIMARY selection,所以使用者必須先 Ctrl+C 才有東西可貼。\n\n");

    // Action skills(phase 5G):open_url / open_app / send_keys / google_search / 等
    prompt.push_str("**open_url(url)**:在系統預設瀏覽器開 URL。\n");
    prompt.push_str(
        "  • 觸發:「打開 https://...」、「開 google.com」(明確帶 URL)。\n");
    prompt.push_str("  • url 必須是 http:// 或 https:// 開頭的絕對 URL。\n\n");

    prompt.push_str("**open_app(app)**:啟動本機 app。\n");
    prompt.push_str(
        "  • 觸發:「打開 firefox」、「開 vscode」、「launch chrome」(明確指定 app)。\n");
    prompt.push_str(
        "  • **如果使用者只說「打開瀏覽器」沒指定哪個**,**不要硬猜** — \
         直接 chat 反問「Firefox / Chrome / Edge 哪個?」(用一兩句),\
         **不要**編造「需要授權」或其他藉口。\n");
    prompt.push_str(
        "  • 範例對應:「打開 firefox」→ open_app(app=\"firefox\");「打開 vscode」→ open_app(app=\"code\")。\n\n");

    prompt.push_str("**send_keys(keys)**:對當下視窗送鍵盤組合。\n");
    prompt.push_str(
        "  • 觸發:「按 Ctrl+S」、「Alt+Tab 切視窗」、「按 Enter」(明確的鍵盤動作)。\n");
    prompt.push_str("  • 格式:「Ctrl+S」/「Alt+Shift+Period」/「F5」。\n\n");

    prompt.push_str("**google_search(query)** / **ask_chatgpt(prompt)** / **ask_gemini(prompt)** / **find_youtube(query)**:\
                     開瀏覽器到對應網站 + 預填查詢。\n");
    prompt.push_str(
        "  • 觸發:「google 一下 X」/「問 ChatGPT X」/「問 Gemini X」/「YouTube 搜 X」。\n");
    prompt.push_str("  • 不要主動叫 — 使用者明確點名才叫。\n\n");

    prompt.push_str(
        "**動作 skill 共同規則**:沒有對應 URL / app / key 等具體參數時,\
         **反問使用者**,不要編造藉口拒絕。\n\n");

    // Mode skill(phase 4B-2)
    prompt.push_str("**set_mode(mode)**:切換 Active / Background。\n");
    prompt.push_str(
        "  • 觸發 background:「晚安」、「先休眠」、「我先離開了」、「下班了」、\
         「安靜一下」、「我去開會了」(明確表示要你閉麥)。\n");
    prompt.push_str(
        "  • 觸發 active:「醒醒」、「起來」、「我回來了」、「在嗎」、\
         「我們繼續」(明確要 Mori 回來工作)。\n");
    prompt.push_str(
        "  • 意圖不明確時不要切;切之後一兩句確認就好,語氣帶點精靈感(\
         例如休眠回「好,我先閉眼,叫我就回來」)。\n\n");

    prompt.push_str(&format!("現在時間:{now}\n"));

    // Phase 4C:當下反白文字(優先順序高於剪貼簿)。使用者反白後講話,
    // 「這個 / 這段」幾乎都是指反白,不是剪貼簿。也是觸發
    // paste_selection_back 的前提。
    if let Some(sel) = &ctx.selected_text {
        prompt.push_str("\n# 當下反白文字\n\n");
        prompt.push_str(
            "**使用者在別的 app 裡剛反白了下面這段文字。**\n\n\
             ## 嚴格時序(這段存在時的流程,不可繞過)\n\n\
             **動詞型指令**(翻譯 / 潤稿 / 摘要 / 改寫 / 英文化 / 改短 / \
             改成 X 語氣 / ...)→ 走這個**兩輪固定流程**:\n\n\
             1. **Round 0**:呼叫 translate / polish / summarize / compose,\
             `source_text` **一律**填這段反白(**不要**用對話歷史、**不要**\
             用剪貼簿、**不要**用 Mori 上輪的回答 — 即使它們看起來更相關)。\n\
             2. **Round 1**:看到 step 1 的結果後,**立刻**呼叫 \
             `paste_selection_back(text=step1 的結果)`,**完整把結果原樣傳入**。\n\
             3. 結束 turn,用一句話回覆使用者「已貼回」就好。\n\n\
             **禁止**:\n\
             - **禁止**對同一段文字連續呼叫兩次 action skill(polish 完不要再 polish,\
             translate 完不要再 translate)。step 1 的結果就是最終答案,直接 paste-back。\n\
             - **禁止**漏掉 step 2(paste_selection_back)。漏 = 整件事 = 沒做。\
             使用者語音說「潤一下」期待看到反白被取代,沒貼回他完全感覺不到 Mori 動了什麼。\n\
             - **禁止**把 source_text 改用對話歷史或 Mori 上輪的回應。哪怕你覺得歷史比較相關,\
             也只用反白。\n\n\
             **問句型指令**(「這在講什麼」、「為什麼這樣寫」、「what does this mean」\
             — 沒有改寫意圖,只是問)→ 直接在 chat 回答,**不**呼叫 paste_selection_back,\
             **不**動使用者編輯區。\n\n",
        );
        // v0.4 Phase A 隱私:選取文字可能含 API key / Bearer token(user 反白
        // .env / config / 終端機輸出),進 LLM API 之前先 redact。audit event
        // 寫進 event_log,marker 不存原文。
        let (redacted_sel, n_redacted) = mori_core::redact::redact_secrets(sel);
        if n_redacted > 0 {
            mori_core::event_log::append(serde_json::json!({
                "kind": "redaction",
                "source": "selection",
                "count": n_redacted,
            }));
        }
        prompt.push_str("```\n");
        prompt.push_str(&redacted_sel);
        prompt.push_str("\n```\n");
    }

    // Phase 3A:當下 context(剪貼簿)。LLM 看到後可在使用者用代名詞時引用。
    if let Some(clip) = &ctx.clipboard {
        // 注意:agent multi-turn loop 每一輪都會重送 system prompt,
        // 且 LLM 把剪貼簿塞進 tool_call args(例如 translate.source_text)後,
        // tool_result 也是相近大小 — 全部疊起來吃 TPM 很快。
        // Groq gpt-oss-120b on_demand TPM = 8000,實測 4000 chars 中文會 413。
        // 1000 chars(~1500 tokens)留出足夠空間給 sys/tools schema/2nd round。
        const MAX_CLIPBOARD_CHARS: usize = 1000;
        let total_chars = clip.chars().count();
        let (preview, truncated_note) = if total_chars > MAX_CLIPBOARD_CHARS {
            let head: String = clip.chars().take(MAX_CLIPBOARD_CHARS).collect();
            (
                head,
                Some(format!(
                    "剪貼簿總長 {total_chars} 字,**只顯示前 {MAX_CLIPBOARD_CHARS} 字**(其餘已截斷)。\
                     使用者要求處理時,**先處理可見的這 {MAX_CLIPBOARD_CHARS} 字**(不要拒做),\
                     做完再順帶提醒「剩下 N 字沒處理到,要繼續嗎?」。"
                )),
            )
        } else {
            (clip.clone(), None)
        };
        prompt.push_str("\n# 當下剪貼簿內容\n\n");
        prompt.push_str(
            "(這是使用者**剛剛複製的內容**。當使用者說「翻譯」/「摘要」/「潤稿」/\
             「這個」/「這段」/「剛複製的」/「這篇」/「幫我寫」之類**動作型指令**\
             但沒給原文時,**幾乎都是指下面這份剪貼簿** — 直接拿去用,不要反問\
             「請提供原文」。\n\
             只在**完全跟剪貼簿無關**(例如純粹閒聊、問時間、查記憶)時才忽略它。\n\
             另外,看不到原文/失敗時也別假裝有處理。)\n\n",
        );
        if let Some(note) = &truncated_note {
            prompt.push_str("**注意**:");
            prompt.push_str(note);
            prompt.push_str("\n\n");
        }
        // v0.4 Phase A 隱私:剪貼簿是最大洩漏點 — user 剛複製 API key / 密碼
        // / 私訊都會被無條件塞進 system prompt 送 provider。先 redact 高風險
        // 樣式(gsk_* / sk-* / AIzaSy* / Bearer * / 40+ char 高熵字串)。
        let (redacted_clip, n_redacted) = mori_core::redact::redact_secrets(&preview);
        if n_redacted > 0 {
            mori_core::event_log::append(serde_json::json!({
                "kind": "redaction",
                "source": "clipboard",
                "count": n_redacted,
            }));
        }
        prompt.push_str("```\n");
        prompt.push_str(&redacted_clip);
        prompt.push_str("\n```\n");
    }

    if !memory_index.is_empty() {
        prompt.push_str("\n");
        prompt.push_str(memory_index);
    }
    prompt
}

/// v0.3.1: 把 tray floating toggle 的 label 重畫成當前 show_mode 對應文字。
fn refresh_floating_toggle_label(item: &MenuItem<tauri::Wry>, show_mode: &str) {
    let label = match show_mode {
        "off" => "桌面 Mori:隱藏中(點此恢復)",
        "recording" => "桌面 Mori:語音輸入時才顯示(點此一直顯示)",
        _ /* always */ => "桌面 Mori:顯示中(點此隱藏)",
    };
    let _ = item.set_text(label);
}

/// 把 tray 三個 mode 選單上的 label 重畫,在當下 mode 那條前面打 ✓。
fn refresh_mode_menu_labels(
    active: &MenuItem<tauri::Wry>,
    voice_input: &MenuItem<tauri::Wry>,
    background: &MenuItem<tauri::Wry>,
    current: Mode,
) {
    let mark = |is_current: bool, base: &str| -> String {
        if is_current {
            format!("✓ {base}")
        } else {
            format!("   {base}")
        }
    };
    let _ = active.set_text(mark(current == Mode::Agent, "對話模式"));
    let _ = voice_input.set_text(mark(current == Mode::VoiceInput, "語音輸入模式"));
    let _ = background.set_text(mark(current == Mode::Background, "休眠(關麥克風)"));
}

// ─── main ───────────────────────────────────────────────────────────

fn main() {
    // ── 強制 XWayland(Linux only)──────────────────────────────────
    // GNOME mutter 在 Wayland 下對 app 自設 alwaysOnTop / position 是「軟
    // 提示」,別 app focused 時會被蓋。實測 yazelin/AgentPulse 在同一台
    // GNOME Wayland 機器上**也**會被蓋,但在 X11 session 上穩穩在最上 —
    // 證實是 display server 的 stacking 語意差別,不是 code 問題。
    //
    // 把 GDK 後端固定走 X11(XWayland 相容層)→ 拿回 X11 的硬 alwaysOnTop。
    // portal 熱鍵走 DBus 不受影響,tray / 麥克風 / 剪貼簿插件也都還能用。
    //
    // 設成 *if not set*,讓進階使用者能用 `GDK_BACKEND=wayland mori` 覆蓋
    // 來測試 Wayland 原生路徑。
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("GDK_BACKEND").is_none() {
            // SAFETY: main() 進來時還是單執行緒,沒人在讀 env。
            std::env::set_var("GDK_BACKEND", "x11");
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mori_tauri=debug,mori_core=debug".into()),
        )
        .init();

    tracing::info!("Mori starting — phase {}", PHASE);
    #[cfg(target_os = "linux")]
    {
        tracing::info!(
            gdk_backend = %std::env::var("GDK_BACKEND").unwrap_or_default(),
            "GDK backend (forced x11 unless overridden)",
        );
    }
    // 反白即改寫(phase 4C)依賴外部工具(Linux 上是 xclip + xdotool/ydotool;
    // Windows 是 built-in Win32 SendInput 不需任何工具)。startup 早點警告
    // 比讓 user 試了一次「為什麼沒貼回」再 grep 程式碼好。
    crate::selection::warn_if_setup_missing();

    // 確保 ~/.mori/config.json 存在(第一次跑就會寫一份 stub)
    let config_path = match GroqProvider::bootstrap_mori_config() {
        Ok(p) => Some(p),
        Err(e) => {
            tracing::warn!(?e, "failed to bootstrap ~/.mori/config.json");
            None
        }
    };

    // 載入 annuli config(hoist 出外層 — supervisor spawn task 也要這個 cfg)
    let annuli_cfg = config_path
        .as_deref()
        .map(annuli_config::AnnuliConfig::load)
        .unwrap_or_default();

    // 建立長期記憶 store + (可選)annuli HTTP client。Wave 4:
    // ~/.mori/config.json 有 `annuli.enabled=true` 且 endpoint / spirit / user_id
    // 都齊 → 用 AnnuliMemoryStore(走 HTTP),同時 state.annuli 也持有 client 給
    // 對話事件 fire-and-forget + hotkey 觸發 /sleep 用。否則 fallback LocalMarkdown。
    let (memory, annuli_client): (Arc<dyn mori_core::memory::MemoryStore>, Option<Arc<mori_core::annuli::AnnuliClient>>) = if annuli_cfg.is_ready() {
        tracing::info!(
            endpoint = %annuli_cfg.endpoint,
            spirit = %annuli_cfg.spirit_name,
            user_id = %annuli_cfg.user_id,
            "annuli memory store enabled — 透過 HTTP 跟 vault 互動",
        );
        let client = Arc::new(
            mori_core::annuli::AnnuliClient::new(annuli_cfg.to_client_config())
                .expect("failed to build annuli client(check config.json annuli.endpoint)"),
        );
        let store = Arc::new(mori_core::memory::annuli::AnnuliMemoryStore::new(client.clone()));
        (store, Some(client))
    } else {
        let memory_root = LocalMarkdownMemoryStore::default_root()
            .expect("could not determine ~/.mori/memory path");
        let store = LocalMarkdownMemoryStore::new(memory_root.clone())
            .expect("failed to initialize memory store");
        tracing::info!(path = %memory_root.display(), "local markdown memory store ready");
        (Arc::new(store), None)
    };

    // 5F-1: 確保 ~/.mori/voice_input/ 存在並有預設檔案
    mori_core::voice_input_profile::ensure_voice_input_dir_initialized();
    mori_core::agent_profile::ensure_agent_dir_initialized();

    let state = Arc::new(AppState {
        phase: Mutex::new(Phase::default()),
        recorder: Mutex::new(None),
        groq_api_key: Mutex::new(None),
        memory: parking_lot::RwLock::new(memory),
        annuli: parking_lot::RwLock::new(annuli_client),
        annuli_supervisor: Mutex::new(None),
        conversation: Mutex::new(Vec::new()),
        mode: Mutex::new(Mode::Agent),
        ollama_warmup: Mutex::new(None),
        hotkey_window_context: Mutex::new(HotkeyWindowContext::default()),
        pipeline_task: Mutex::new(None),
        // 5T: 啟動先用 default(Toggle),setup 讀 ~/.mori/config.json 後覆寫成
        // 實際值(見下方 hotkey_config 載入處)。
        toggle_mode: Mutex::new(hotkey_config::ToggleMode::default()),
    });

    if let Some(key) = GroqProvider::discover_api_key() {
        tracing::info!("found GROQ_API_KEY");
        *state.groq_api_key.lock() = Some(key);
    } else {
        match &config_path {
            Some(p) => tracing::warn!(
                path = %p.display(),
                "no GROQ_API_KEY found — edit this file and replace the placeholder, \
                 or set $GROQ_API_KEY env var"
            ),
            None => tracing::warn!("no GROQ_API_KEY found and config bootstrap failed"),
        }
    }

    let state_for_setup = state.clone();

    tauri::Builder::default()
        // 5J-followup: 防止 mori-tauri orphan + 新實例並存的搶 tray / hotkey 戰。
        // 第二個 instance 啟動時觸發此 callback:把焦點還給第一個然後自殺。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tracing::warn!("another mori-tauri instance tried to start — focusing existing");
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.show();
                let _ = main.set_focus();
            }
        }))
        .manage(state.clone())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            mori_version,
            mori_phase,
            is_x11_session,
            linux_session_type,
            force_raise_window,
            apply_floating_shape,
            read_floating_backplate,
            build_info,
            chat_provider_info,
            current_phase,
            has_groq_key,
            has_gemini_key,
            has_openai_key,
            verify_llm_key,
            toggle,
            reset_conversation,
            conversation_length,
            get_conversation,
            submit_text,
            current_mode,
            set_mode_cmd,
            cancel_recording,
            picker_list_voice_profiles,
            picker_list_agent_profiles,
            picker_switch_voice_profile,
            picker_switch_agent_profile,
            config_read,
            config_write,
            floating_show,
            floating_set_above,
            open_external_url,
            corrections_read,
            corrections_write,
            profile_read,
            profile_write,
            profile_delete,
            open_profile_dir,
            list_starter_templates,
            install_starter_template,
            estimate_profile_tokens,
            active_profiles,
            log_tail,
            log_dates,
            memory_list,
            memory_read,
            memory_write,
            memory_delete,
            annuli_commands::annuli_status,
            annuli_commands::annuli_get_soul,
            annuli_commands::annuli_list_memory,
            annuli_commands::annuli_list_events_today,
            annuli_commands::annuli_trigger_sleep,
            annuli_reload,
            memory_search,
            skills_list,
            deps_list,
            deps_install,
            theme_list,
            theme_read,
            theme_get_active,
            theme_set_active,
            theme_toggle,
            theme_dir,
            character_list,
            character_get_active,
            character_set_active,
            character_sprite_data_url,
            character_dir,
            character_upgrade_pack_to_4x4,
        ])
        .on_window_event(|window, event| {
            // 關視窗時不殺 app — 隱藏到系統匣繼續跑(像 Slack / Discord)
            if let WindowEvent::CloseRequested { api, .. } = event {
                tracing::info!("close requested → hiding to tray");
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .setup(move |app| {
            // brand-3: ensure ~/.mori/themes/ + 內建 dark.json / light.json
            // 啟動時寫入(已存在則保留 user 編輯)
            if let Err(e) = crate::theme::ensure_builtin() {
                tracing::warn!(error = %e, "theme::ensure_builtin failed");
            }

            // 5P-1: ensure ~/.mori/characters/mori/(default character pack)。
            // manifest.json + 6 張 sprite PNG 從 binary 內嵌寫入,已存在不覆蓋。
            if let Err(e) = crate::character_pack::ensure_default() {
                tracing::warn!(error = %e, "character_pack::ensure_default failed");
            }

            // 啟動初始 floating visibility — update_floating_visibility 自己會看:
            //   - quickstart_completed (儀式還沒完成 → 強制隱藏)
            //   - floating.show_mode (always/recording/off)
            // 起手 Phase 是 Idle,所以 "recording" 模式啟動時也會隱藏 (正確)。
            update_floating_visibility(&app.handle(), &Phase::Idle);

            // 注意:**不要**在這裡 setup() 直接 call set_always_on_top —
            // AgentPulse 沒這樣做,他們依賴 conf.json 的 alwaysOnTop hint
            // 處理初始狀態,只在 tray show handler 才 re-assert。
            // 我們先試一樣的紀律,看 mutter 認不認帳。

            // ── 系統匣(tray)+ 選單 ──
            // 5E 起 mode 從 2 態(Active / Background)變 3 態(+ VoiceInput),
            // 改用 3 個獨立 menu item 顯示三種模式,目前的那個會在 label
            // 前面打 ✓。改 mode 後 mode-changed listener 會 refresh labels。
            let mode_active_item =
                MenuItem::with_id(app, "mode_active", "對話模式", true, None::<&str>)?;
            let mode_voice_input_item =
                MenuItem::with_id(app, "mode_voice_input", "語音輸入模式", true, None::<&str>)?;
            let mode_background_item =
                MenuItem::with_id(app, "mode_background", "休眠(關麥克風)", true, None::<&str>)?;

            // 5K-2: 掃 ~/.mori/voice_input + ~/.mori/agent 目錄,把所有 profile
            // 列成 tray 子選單(超過 Alt+0~9 / Ctrl+Alt+0~9 的也能點)。
            // ID 規則:`voice_profile:<stem>` / `agent_profile:<stem>`,on_menu_event 解析。
            let voice_items: Vec<MenuItem<tauri::Wry>> =
                mori_core::voice_input_profile::list_voice_profiles()
                    .into_iter()
                    .map(|(stem, display)| {
                        MenuItem::with_id(
                            app,
                            format!("voice_profile:{stem}"),
                            display,
                            true,
                            None::<&str>,
                        )
                        .expect("build voice profile menu item")
                    })
                    .collect();
            let agent_items: Vec<MenuItem<tauri::Wry>> =
                mori_core::agent_profile::list_agent_profiles()
                    .into_iter()
                    .map(|(stem, display)| {
                        MenuItem::with_id(
                            app,
                            format!("agent_profile:{stem}"),
                            display,
                            true,
                            None::<&str>,
                        )
                        .expect("build agent profile menu item")
                    })
                    .collect();
            let voice_refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> =
                voice_items.iter().map(|i| i as &dyn tauri::menu::IsMenuItem<tauri::Wry>).collect();
            let agent_refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> =
                agent_items.iter().map(|i| i as &dyn tauri::menu::IsMenuItem<tauri::Wry>).collect();
            let voice_submenu = Submenu::with_id_and_items(
                app,
                "voice_profile_submenu",
                if voice_items.is_empty() { "Voice Profile（無）" } else { "Voice Profile ▸" },
                !voice_items.is_empty(),
                &voice_refs,
            )?;
            let agent_submenu = Submenu::with_id_and_items(
                app,
                "agent_profile_submenu",
                if agent_items.is_empty() { "Agent Profile（無）" } else { "Agent Profile ▸" },
                !agent_items.is_empty(),
                &agent_refs,
            )?;

            // v0.3.1: tray floating toggle — label 反映當前 show_mode
            // (✓ 顯示中 / 語音輸入時 / 隱藏中);點一下依目前態切下一個 sane 值
            let current_show_mode = read_floating_show_mode();
            let floating_toggle_label = match current_show_mode.as_str() {
                "off" => "桌面 Mori:隱藏中(點此恢復)",
                "recording" => "桌面 Mori:語音輸入時才顯示(點此一直顯示)",
                _ /* always */ => "桌面 Mori:顯示中(點此隱藏)",
            };
            let floating_toggle_item = MenuItem::with_id(
                app, "floating_toggle", floating_toggle_label, true, None::<&str>,
            )?;

            let menu = Menu::with_items(
                app,
                &[
                    &MenuItem::with_id(app, "show", "顯示 Mori", true, None::<&str>)?,
                    &MenuItem::with_id(app, "hide", "隱藏", true, None::<&str>)?,
                    &floating_toggle_item,
                    &mode_active_item,
                    &mode_voice_input_item,
                    &mode_background_item,
                    &voice_submenu,
                    &agent_submenu,
                    &MenuItem::with_id(app, "reset", "重新開始對話", true, None::<&str>)?,
                    &MenuItem::with_id(app, "quit", "離開", true, None::<&str>)?,
                ],
            )?;

            let state_for_tray = state_for_setup.clone();
            let mode_items_for_handler = (
                mode_active_item.clone(),
                mode_voice_input_item.clone(),
                mode_background_item.clone(),
            );
            let floating_toggle_item_for_handler = floating_toggle_item.clone();
            // 啟動時就把 ✓ 標到目前 mode 上
            refresh_mode_menu_labels(
                &mode_items_for_handler.0,
                &mode_items_for_handler.1,
                &mode_items_for_handler.2,
                *state_for_tray.mode.lock(),
            );
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Mori")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                        // Critical: re-assert floating's always-on-top from
                        // Rust here. mutter on GNOME Wayland silently
                        // demotes the floating window's z-order whenever
                        // the main window is hidden+shown again, but
                        // accepts a fresh `set_always_on_top(true)` from
                        // Rust as a legitimate window-manager event (not
                        // a misbehaving client). Same trick yazelin/AgentPulse
                        // uses on its tray show/hide handlers.
                        if let Some(f) = app.get_webview_window("floating") {
                            let _ = f.set_always_on_top(true);
                        }
                    }
                    "hide" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.hide();
                        }
                        // After hiding main, re-elevate floating so it
                        // doesn't sink behind whatever the user focuses
                        // next.
                        if let Some(f) = app.get_webview_window("floating") {
                            let _ = f.set_always_on_top(true);
                        }
                    }
                    "floating_toggle" => {
                        // v0.3.1: 二值切換(always ↔ off); recording 也視為「on」狀態
                        // 切去 off。完整三選一在 Config tab 那邊。
                        let cur = read_floating_show_mode();
                        let next = if cur == "off" { "always" } else { "off" };
                        // 寫回 config.json
                        let path = mori_dir().join("config.json");
                        let cfg: serde_json::Value = std::fs::read_to_string(&path)
                            .ok()
                            .and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or_else(|| serde_json::json!({}));
                        let mut cfg = cfg;
                        let floating = cfg
                            .as_object_mut()
                            .and_then(|m| {
                                if !m.contains_key("floating") {
                                    m.insert("floating".to_string(), serde_json::json!({}));
                                }
                                m.get_mut("floating")
                            })
                            .and_then(|v| v.as_object_mut());
                        if let Some(f) = floating {
                            f.insert("show_mode".to_string(), serde_json::json!(next));
                        }
                        if let Ok(text) = serde_json::to_string_pretty(&cfg) {
                            let _ = std::fs::write(&path, text);
                        }
                        let _ = app.emit("config-changed", ());
                        // 立即套用 + 更新 menu label
                        let phase = state_for_tray.phase.lock().clone();
                        update_floating_visibility(app, &phase);
                        refresh_floating_toggle_label(&floating_toggle_item_for_handler, next);
                        tracing::info!(prev = %cur, new = next, "tray toggle floating.show_mode");
                    }
                    "mode_active" => state_for_tray.set_mode(app, Mode::Agent),
                    "mode_voice_input" => state_for_tray.set_mode(app, Mode::VoiceInput),
                    "mode_background" => state_for_tray.set_mode(app, Mode::Background),
                    id if id.starts_with("voice_profile:") => {
                        let stem = &id["voice_profile:".len()..];
                        if !matches!(*state_for_tray.mode.lock(), Mode::VoiceInput) {
                            state_for_tray.set_mode(app, Mode::VoiceInput);
                        }
                        if let Some(info) =
                            mori_core::voice_input_profile::switch_to_profile(stem)
                        {
                            let _ = app.emit("voice-input-profile-switched", info.label());
                        }
                    }
                    id if id.starts_with("agent_profile:") => {
                        let stem = &id["agent_profile:".len()..];
                        if !matches!(*state_for_tray.mode.lock(), Mode::Agent) {
                            state_for_tray.set_mode(app, Mode::Agent);
                        }
                        if let Some(info) =
                            mori_core::agent_profile::switch_to_agent_profile(stem)
                        {
                            let label =
                                format!("Agent · {} · {}", info.profile_name, info.llm_provider);
                            let _ = app.emit("voice-input-profile-switched", label);
                        }
                    }
                    "reset" => {
                        let mut conv = state_for_tray.conversation.lock();
                        let n = conv.len();
                        conv.clear();
                        tracing::info!(cleared = n, "conversation reset (tray)");
                    }
                    "quit" => {
                        tracing::info!("quit from tray");
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            // tray labels 跟著 Mode 同步:任何來源(tray 點擊、IPC、skill)改了
            // Mode 都 emit "mode-changed",這裡統一接 → 把 ✓ 標到正確的 menu item。
            let (active_item, voice_input_item, background_item) = mode_items_for_handler;
            app.listen("mode-changed", move |event| {
                let payload = event.payload();
                let target = if payload.contains("\"voice_input\"") {
                    Mode::VoiceInput
                } else if payload.contains("\"background\"") {
                    Mode::Background
                } else {
                    Mode::Agent
                };
                refresh_mode_menu_labels(&active_item, &voice_input_item, &background_item, target);
            });

            // ── Skill HTTP server(5D)─────────────────────────────
            // bind 127.0.0.1:RANDOM,寫 ~/.mori/runtime.json,讓 mori CLI
            // (以及外部 AI agent 透過 Bash tool 呼叫的 mori CLI)能連回來
            // dispatch skill。失敗只 warn 不卡啟動 — Tauri UI 跟語音/chat
            // pipeline 沒這個 server 也能用。
            let app_for_server = state_for_setup.clone();
            tauri::async_runtime::spawn(async move {
                match crate::skill_server::start(app_for_server).await {
                    Ok(info) => tracing::info!(
                        port = info.port,
                        "skill HTTP server started — Bash CLI proxy ready"
                    ),
                    Err(e) => tracing::warn!(?e, "skill HTTP server failed to start"),
                }
            });

            // ── D-1: annuli admin server supervisor ───────────────
            // 若 annuli.enabled + endpoint 是 localhost + 沒人在跑 → spawn
            // python main.py admin。kill_on_drop 保證 app 關時子 process 跟著掛。
            // 非 localhost / 已有人跑 / config disabled / venv 缺 → no-op,
            // 走 fallback。整個 task 跑在背景,不卡 setup 結束(spawn + 等
            // health 最多 15s)。
            let annuli_cfg_for_supervisor = annuli_cfg.clone();
            let state_for_supervisor = state_for_setup.clone();
            tauri::async_runtime::spawn(async move {
                let sup =
                    annuli_supervisor::AnnuliSupervisor::maybe_spawn(&annuli_cfg_for_supervisor)
                        .await;
                tracing::info!(state = sup.info.state, reason = %sup.info.reason, "annuli supervisor settled");
                *state_for_supervisor.annuli_supervisor.lock() = Some(sup);
            });

            // ── Ollama warm-up(僅當 provider=ollama)─────
            // qwen3:8b 5.2GB 在 Intel CPU 沒 GPU 加速首次載入可能要分鐘級。
            // 啟動就背景發一個 1-token chat 觸發 model load,使用者第一次按
            // 熱鍵時模型已熱。Groq 路徑直接 no-op。
            //
            // 兩個地方記狀態:
            // 1. AppState.ollama_warmup — 給後到的 React 訂閱者用
            //   (model 已熱時 warm-up 1 秒就完成,React 還沒 listen 到時
            //    event 已經 emit 過了)。
            // 2. emit `ollama-warmup` event — 給已經訂閱的訂閱者收 transition。
            let app_for_warmup = app.handle().clone();
            let state_for_warmup = state_for_setup.clone();
            tauri::async_runtime::spawn(async move {
                let snap = mori_core::llm::active_chat_provider_snapshot();
                if snap.name != "ollama" {
                    return;
                }
                let Some(base_url) = snap.base_url else {
                    return;
                };
                *state_for_warmup.ollama_warmup.lock() = Some("loading");
                let _ = app_for_warmup.emit("ollama-warmup", "loading");
                match mori_core::llm::ollama::OllamaProvider::warm_up(
                    &base_url,
                    &snap.model,
                )
                .await
                {
                    Ok(_) => {
                        *state_for_warmup.ollama_warmup.lock() = Some("ready");
                        let _ = app_for_warmup.emit("ollama-warmup", "ready");
                    }
                    Err(e) => {
                        tracing::warn!(?e, "ollama warm-up failed");
                        *state_for_warmup.ollama_warmup.lock() = Some("failed");
                        let _ = app_for_warmup.emit("ollama-warmup", "failed");
                    }
                }
            });

            // ── 全域熱鍵:Ctrl+Alt+Space + 22 個 profile 切換鍵 ──
            //
            // 平台 → 註冊 path:
            // | 平台           | path                                        |
            // |---|---|
            // | Linux X11      | `tauri-plugin-global-shortcut`(XGrabKey)  |
            // | Linux Wayland  | `xdg-desktop-portal.GlobalShortcuts`        |
            // | Windows        | `tauri-plugin-global-shortcut`(RegisterHotKey) |
            // | macOS          | `tauri-plugin-global-shortcut`(Carbon)    |
            //
            // XWayland(wayland session 跑 X 程式)也走 portal — XGrabKey 在
            // XWayland 下被 compositor 擋掉。
            //
            // 所有 path 共用同一份 ~/.mori/config.json `hotkeys` 設定;X11 /
            // Win / Mac 由 config 100% 決定,Wayland 走 portal 後使用者改 GNOME
            // 系統設定為準(portal 規範如此,Mori config 只當第一次註冊的建議值)。

            // === 載入 config(跨平台)+ 寫 toggle_mode 進 state ===
            let mori_config_path = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE")) // Windows fallback
                .map(std::path::PathBuf::from)
                .map(|h| h.join(".mori/config.json"))
                .unwrap_or_default();
            let hotkey_config = hotkey_config::HotkeyConfig::load(&mori_config_path);
            // 5T: 寫進 state 給後續 PRESSED/RELEASED listener 用。改 config 時
            // config_write 會 reload 一次,所以這裡只負責「啟動快照」。
            *state_for_setup.toggle_mode.lock() = hotkey_config.toggle_mode;

            #[cfg(target_os = "linux")]
            {
                if x11_hotkey::is_x11_session() {
                    tracing::info!("X11 session detected — using tauri-plugin-global-shortcut");
                    if let Err(e) = x11_hotkey::register(&app.handle(), &hotkey_config) {
                        tracing::error!(
                            ?e,
                            "X11 global shortcut registration failed — \
                             use the UI toggle button to trigger Mori"
                        );
                    }
                    // X11 透明 fallback:在三個 transparent window 的 <body> 注入
                    // .x11-fallback class,CSS 規則切到 opaque background,WebKit
                    // 就不會把 drop-shadow / blur / glow 等 half-alpha 渲染成方框。
                    // 詳細邏輯見 src/floating.css 同名 selector 註解。
                    // JS 寫成 idempotent + DOMContentLoaded fallback,在 webview
                    // 載入任何階段呼叫都安全。
                    let inject_js = r#"(function(){function a(){if(document.body)document.body.classList.add('x11-fallback');else document.addEventListener('DOMContentLoaded',a)}a()})();"#;
                    for label in &["floating", "chat_bubble", "picker"] {
                        if let Some(w) = app.get_webview_window(label) {
                            if let Err(e) = w.eval(inject_js) {
                                tracing::warn!(?e, label, "x11-fallback inject failed");
                            } else {
                                tracing::debug!(label, "x11-fallback class injected");
                            }
                        }
                    }
                    // X11 floating window → 圓形 OS-level clip(XShape)。CSS
                    // border-radius 在 transparent X11 window 邊緣 AA 會破,XShape
                    // 是 1-bit alpha clip,沒 AA、沒半透明,完美避開渲染問題。
                    // window 啟動初期可能還沒 mapped(X server registry 沒登記
                    // WM_NAME),sleep 一下再去 xdotool search 抓 XID。500ms 在
                    // 開機階段夠 mutter 把 window 註冊好。失敗的話 log warn 不
                    // bail,Mori 還是能用,只是 floating 仍是矩形。
                    // X11 floating XShape — shape 由 ~/.mori/config.json
                    // floating.x11_shape 決定("square" | "rounded" | "circle"),
                    // square 直接 skip 不 apply。
                    let app_for_shape = app.handle().clone();
                    let shape_config_path = mori_config_path.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        let (shape, radius) = read_floating_shape(&shape_config_path);
                        if shape == "square" {
                            tracing::info!("x11_shape = square, skipping XShape clip");
                            return;
                        }
                        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                        let Some(win) = app_for_shape.get_webview_window("floating") else {
                            tracing::warn!("no floating window for XShape");
                            return;
                        };
                        let size = match win.inner_size() {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::warn!(?e, "floating inner_size failed");
                                return;
                            }
                        };
                        let handle = match win.window_handle() {
                            Ok(h) => h,
                            Err(e) => {
                                tracing::warn!(?e, "floating window_handle failed");
                                return;
                            }
                        };
                        let xid = match handle.as_raw() {
                            RawWindowHandle::Xlib(x) => x.window as u32,
                            other => {
                                tracing::warn!(?other, "floating not Xlib window handle");
                                return;
                            }
                        };
                        tracing::info!(
                            xid,
                            width = size.width,
                            height = size.height,
                            shape = %shape,
                            radius,
                            "floating XShape config + XID resolved"
                        );
                        let result = match shape.as_str() {
                            "rounded" => {
                                // radius 是 logical px,要轉 physical(scaleFactor)
                                let scale = win.scale_factor().unwrap_or(1.0);
                                let r_phys = (radius as f64 * scale).round() as u32;
                                x11_shape::apply_rounded_clip(xid, size.width, size.height, r_phys)
                            }
                            _ /* circle or unknown */ => {
                                x11_shape::apply_circle_clip(xid, size.width, size.height)
                            }
                        };
                        if let Err(e) = result {
                            tracing::warn!(?e, xid, "XShape clip failed");
                        }
                    });
                } else {
                    tracing::info!(
                        "non-X11 session detected — using xdg-desktop-portal GlobalShortcuts"
                    );
                    let app_for_portal = app.handle().clone();
                    let portal_config = hotkey_config.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = portal_hotkey::run(app_for_portal, portal_config).await {
                            // Not fatal — UI button still works as fallback.
                            // Common reasons: xdg-desktop-portal-gnome too old
                            // (no org.freedesktop.host.portal.Registry interface
                            // — Ubuntu 24.04 LTS ships 1.18 which lacks it),
                            // user denied the permission dialog, or no portal
                            // session (some headless / display-less envs).
                            tracing::error!(
                                ?e,
                                "portal global shortcut unavailable — use the UI \
                                 toggle button to trigger Mori"
                            );
                        }
                    });
                }
            }

            // Windows / macOS:直接走 tauri-plugin-global-shortcut 註冊全套
            // 22 條,跟 Linux X11 path 共用同一份 `x11_hotkey::register`(底層是
            // 跨平台的 plugin,Windows 用 `RegisterHotKey`,macOS 用 Carbon)。
            #[cfg(not(target_os = "linux"))]
            {
                if let Err(e) = x11_hotkey::register(&app.handle(), &hotkey_config) {
                    tracing::error!(
                        ?e,
                        "global shortcut registration failed — \
                         use the UI toggle button to trigger Mori"
                    );
                }
            }

            // === 以下 listener wiring 跨平台 ===

            // 5T: 永遠掛 PRESSED + RELEASED 兩個 listener,handler 內讀
            // `state.toggle_mode` 決定 dispatch:
            //   Toggle 模式 → PRESSED 跑 handle_hotkey_toggle;RELEASED 忽略
            //   Hold   模式 → PRESSED 跑 handle_hotkey_pressed;RELEASED 跑 handle_hotkey_released
            // 這樣切換 toggle_mode(config_write 寫完會 reload state mode)
            // 不必重啟,下一次按鍵立刻走新邏輯。
            let handle_press = app.handle().clone();
            let state_press = state_for_setup.clone();
            app.listen(hotkey_config::PORTAL_HOTKEY_PRESSED, move |_event| {
                let mode = *state_press.toggle_mode.lock();
                match mode {
                    hotkey_config::ToggleMode::Toggle => {
                        handle_hotkey_toggle(handle_press.clone(), state_press.clone());
                    }
                    hotkey_config::ToggleMode::Hold => {
                        handle_hotkey_pressed(handle_press.clone(), state_press.clone());
                    }
                }
            });
            let handle_release = app.handle().clone();
            let state_release = state_for_setup.clone();
            app.listen(hotkey_config::PORTAL_HOTKEY_RELEASED, move |_event| {
                let mode = *state_release.toggle_mode.lock();
                if mode == hotkey_config::ToggleMode::Hold {
                    handle_hotkey_released(handle_release.clone(), state_release.clone());
                }
                // Toggle 模式 RELEASED 是 no-op:Press 已做完 toggle 動作。
            });
            tracing::info!(
                "hotkey toggle/hold listeners armed (mode={:?})",
                *state_for_setup.toggle_mode.lock(),
            );

            // 5J: Ctrl+Alt+Esc — 全域中斷
            // - Phase::Recording → 停錄音 + 丟掉音檔不送 STT
            // - Phase::Transcribing / Responding → abort pipeline task,
            //   kill_on_drop 讓 claude / gemini / codex 子程序連帶 SIGKILL
            // - 其他 phase → 忽略
            let handle_cancel = app.handle().clone();
            let state_for_cancel = state_for_setup.clone();
            app.listen(hotkey_config::PORTAL_CANCEL_EVENT, move |_event| {
                let phase = state_for_cancel.phase.lock().clone();
                match phase {
                    Phase::Recording { .. } => {
                        tracing::info!("Ctrl+Alt+Esc — cancelling current recording");
                        if let Some(rec) = state_for_cancel.recorder.lock().take() {
                            match rec.stop() {
                                Ok(audio) => {
                                    let secs = audio.samples.len() as f32
                                        / (audio.sample_rate as f32 * audio.channels as f32);
                                    tracing::info!(
                                        duration_secs = secs,
                                        "recording cancelled via portal hotkey (audio discarded)",
                                    );
                                }
                                Err(e) => tracing::warn!(?e, "stop on portal cancel returned err"),
                            }
                        }
                        // abort 後台 pipeline(如果剛好已經 spawn 但還沒進階段)
                        if let Some(task) = state_for_cancel.pipeline_task.lock().take() {
                            task.abort();
                        }
                        state_for_cancel.set_phase(&handle_cancel, Phase::Idle);
                    }
                    Phase::Transcribing | Phase::Responding { .. } => {
                        tracing::info!(?phase, "Ctrl+Alt+Esc — aborting in-flight pipeline");
                        if let Some(task) = state_for_cancel.pipeline_task.lock().take() {
                            task.abort();
                            tracing::info!("pipeline task aborted (kill_on_drop will SIGKILL child)");
                        }
                        state_for_cancel.set_phase(&handle_cancel, Phase::Idle);
                    }
                    _ => {
                        tracing::debug!(?phase, "Ctrl+Alt+Esc fired but no in-flight work — ignored");
                    }
                }
            });

            // 5K-1: Ctrl+Alt+P 開 picker
            //
            // 流程:Rust 先 show() + set_focus()(確保視窗 visible + Wayland 抓焦點),
            // 再 emit picker-open 給 React 端 center / 拉 profile 列表。
            // visible: false 的 window webview 仍會載入,React mount 後 listener 已就位;
            // 但 emit 前先 show 確保視窗顯示時機跟 focus 都對齊。
            let handle_picker = app.handle().clone();
            app.listen(hotkey_config::PORTAL_PICKER_EVENT, move |_event| {
                tracing::debug!("picker listener fired — looking up picker window");
                match handle_picker.get_webview_window("picker") {
                    Some(w) => {
                        if let Err(e) = w.show() {
                            tracing::warn!(?e, "picker w.show() failed");
                        }
                        if let Err(e) = w.set_focus() {
                            tracing::warn!(?e, "picker w.set_focus() failed");
                        }
                        if let Err(e) = handle_picker.emit("picker-open", ()) {
                            tracing::warn!(?e, "picker emit picker-open failed");
                        } else {
                            tracing::info!("picker show + focus + emit picker-open done");
                        }
                    }
                    None => {
                        tracing::error!("picker window not found by label 'picker'");
                    }
                }
            });

            // Wave 4 step 7:Ctrl+Alt+Z sleep hotkey → POST /rings/new fire-and-forget
            let state_for_sleep = state_for_setup.clone();
            app.listen(hotkey_config::MORI_SLEEP_EVENT, move |_event| {
                tracing::info!("sleep hotkey fired");
                let Some(client) = state_for_sleep.annuli_handle() else {
                    tracing::warn!("sleep hotkey fired but annuli not configured (config.json annuli.enabled=false)");
                    return;
                };
                tokio::spawn(async move {
                    match client.trigger_sleep().await {
                        Ok(ring_path) => tracing::info!(%ring_path, "ring written via /sleep"),
                        Err(e) => tracing::warn!(error = %e, "annuli POST /rings/new failed"),
                    }
                });
            });

            // 5F-2: Alt+0~9 VoiceInput profile 切換
            let handle_slot = app.handle().clone();
            let state_for_slot = state_for_setup.clone();
            app.listen(hotkey_config::PROFILE_SLOT_EVENT, move |event| {
                let Ok(slot) = serde_json::from_str::<u8>(event.payload()) else {
                    return;
                };
                handle_profile_slot(handle_slot.clone(), state_for_slot.clone(), slot);
            });

            // 5G-5: Ctrl+Alt+0~9 Agent profile 切換
            let handle_agent_slot = app.handle().clone();
            let state_for_agent_slot = state_for_setup.clone();
            app.listen(hotkey_config::AGENT_SLOT_EVENT, move |event| {
                let Ok(slot) = serde_json::from_str::<u8>(event.payload()) else {
                    return;
                };
                handle_agent_profile_slot(
                    handle_agent_slot.clone(),
                    state_for_agent_slot.clone(),
                    slot,
                );
            });

            tracing::info!("hotkey path ready (toggle + cancel + picker + Alt+0~9 + Ctrl+Alt+0~9) + tray icon");

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_weekday_maps_all_seven() {
        assert_eq!(chinese_weekday("Monday"), "星期一");
        assert_eq!(chinese_weekday("Tuesday"), "星期二");
        assert_eq!(chinese_weekday("Wednesday"), "星期三");
        assert_eq!(chinese_weekday("Thursday"), "星期四");
        assert_eq!(chinese_weekday("Friday"), "星期五");
        assert_eq!(chinese_weekday("Saturday"), "星期六");
        assert_eq!(chinese_weekday("Sunday"), "星期日");
        assert_eq!(chinese_weekday("Funday"), "?");
    }

    fn empty_win_ctx() -> HotkeyWindowContext {
        HotkeyWindowContext::default()
    }

    fn empty_mori_ctx() -> MoriContext {
        MoriContext::default()
    }

    #[test]
    fn context_section_always_includes_time() {
        // 5J 的關鍵保證:Mori 永遠知道現在幾點(以前 voice_input / agent
        // 漏注入導致「不知道時間」)
        let out = build_context_section(&empty_win_ctx(), &empty_mori_ctx(), None);
        assert!(out.contains("時間"), "missing 時間 marker in: {out}");
        // 至少含一個 4 位數年份
        assert!(out.contains("20"), "year missing in: {out}");
        // 含中文星期
        assert!(out.contains("星期"), "weekday missing in: {out}");
    }

    #[test]
    fn context_section_includes_os() {
        let out = build_context_section(&empty_win_ctx(), &empty_mori_ctx(), None);
        assert!(out.contains("作業系統"));
        assert!(out.contains(std::env::consts::OS));
    }

    #[test]
    fn context_section_shows_unknown_when_window_empty() {
        let out = build_context_section(&empty_win_ctx(), &empty_mori_ctx(), None);
        assert!(out.contains("(未知)"), "should show 未知 for empty window: {out}");
    }

    #[test]
    fn context_section_shows_actual_window_fields() {
        let win = HotkeyWindowContext {
            process_name: "code".into(),
            window_title: "main.rs - VS Code".into(),
            selected_text: String::new(),
        };
        let out = build_context_section(&win, &empty_mori_ctx(), None);
        assert!(out.contains("process: code"));
        assert!(out.contains("title: main.rs - VS Code"));
    }

    #[test]
    fn context_section_shows_clipboard_when_present() {
        let mut ctx = empty_mori_ctx();
        ctx.clipboard = Some("hello world".into());
        let out = build_context_section(&empty_win_ctx(), &ctx, None);
        assert!(out.contains("剪貼簿: hello world"));
    }

    #[test]
    fn context_section_falls_back_to_mori_ctx_selected_when_win_ctx_empty() {
        // win_ctx.selected_text 是熱鍵當下抓的;mori_ctx.selected_text 是 ContextProvider 抓的
        // 5J: 兩個都檢查,win_ctx 優先,空才退到 mori_ctx
        let mut mctx = empty_mori_ctx();
        mctx.selected_text = Some("from mori ctx".into());
        let out = build_context_section(&empty_win_ctx(), &mctx, None);
        assert!(out.contains("反白文字: from mori ctx"));
    }

    #[test]
    fn context_section_win_ctx_selected_wins_over_mori_ctx() {
        let win = HotkeyWindowContext {
            selected_text: "from win ctx".into(),
            ..Default::default()
        };
        let mut mctx = empty_mori_ctx();
        mctx.selected_text = Some("from mori ctx".into());
        let out = build_context_section(&win, &mctx, None);
        assert!(out.contains("反白文字: from win ctx"));
        assert!(!out.contains("from mori ctx"));
    }

    #[test]
    fn context_section_omits_memory_when_none() {
        // VoiceInput 不傳 memory_index — 該段完全不該出現
        let out = build_context_section(&empty_win_ctx(), &empty_mori_ctx(), None);
        assert!(!out.contains("長期記憶索引"), "memory section leaked: {out}");
    }

    #[test]
    fn context_section_includes_memory_when_some() {
        let out = build_context_section(
            &empty_win_ctx(),
            &empty_mori_ctx(),
            Some("- mem1: about user\n- mem2: ..."),
        );
        assert!(out.contains("長期記憶索引"));
        assert!(out.contains("mem1: about user"));
    }

    #[test]
    fn context_section_empty_memory_shows_placeholder() {
        let out = build_context_section(&empty_win_ctx(), &empty_mori_ctx(), Some("  \n"));
        assert!(out.contains("長期記憶索引"));
        assert!(out.contains("目前沒有記憶"));
    }
}
