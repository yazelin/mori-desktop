// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod context_provider;
#[cfg(target_os = "linux")]
mod portal_hotkey;
mod recording;
#[cfg(target_os = "linux")]
mod selection;

use std::sync::Arc;

use anyhow::Context as _;
use async_trait::async_trait;
use mori_core::agent::{Agent, SkillCallSummary};
use mori_core::context::{Context as MoriContext, ContextProvider};
use mori_core::llm::groq::{GroqProvider, RetryEvent};
use mori_core::llm::{ChatMessage, LlmProvider};
use mori_core::memory::markdown::LocalMarkdownMemoryStore;
use mori_core::memory::MemoryStore;
use mori_core::mode::{Mode, ModeController};
#[cfg(target_os = "linux")]
use mori_core::paste::PasteController;
#[cfg(target_os = "linux")]
use mori_core::skill::PasteSelectionBackSkill;
use mori_core::skill::{
    ComposeSkill, EditMemorySkill, ForgetMemorySkill, PolishSkill, RecallMemorySkill,
    RememberSkill, SetModeSkill, SkillRegistry, SummarizeSkill, TranslateSkill,
};
use mori_core::{PHASE, VERSION};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Listener, Manager, WindowEvent};
#[cfg(not(target_os = "linux"))]
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

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
    /// phase 7+ 換成 SyncedMemoryStore 不重寫上層程式碼。
    pub memory: Arc<LocalMarkdownMemoryStore>,
    /// Working memory:本次 session 的對話歷史(user / assistant 訊息對)。
    /// 重啟 app 就清空。長期記憶寫進 memory 那邊。
    pub conversation: Mutex<Vec<ChatMessage>>,
    /// 運作模式 — Active(平常)/ Background(假寐,麥克風硬關)。
    /// Phase 對應「使用者的這一輪對話進行到哪」,Mode 是「Mori 整體的工作狀態」,
    /// 兩者正交。Phase 變回 Idle 不會動 Mode。
    pub mode: Mutex<Mode>,
}

impl AppState {
    fn set_phase(&self, app: &AppHandle, new_phase: Phase) {
        tracing::info!(?new_phase, "phase change");
        *self.phase.lock() = new_phase.clone();
        if let Err(e) = app.emit("phase-changed", &new_phase) {
            tracing::warn!(?e, "failed to emit phase-changed");
        }
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

#[tauri::command]
fn current_phase(state: tauri::State<Arc<AppState>>) -> Phase {
    state.phase.lock().clone()
}

#[tauri::command]
fn has_groq_key(state: tauri::State<Arc<AppState>>) -> bool {
    state.groq_api_key.lock().is_some()
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

    // Phase 5A-1: chat provider 走 config-driven factory
    // (default_provider="groq" / "ollama"),retry callback 只對 Groq 有意義
    // 也透傳進 factory(內部會 ignore 給 Ollama)。
    let provider = match mori_core::llm::build_chat_provider(Some(retry_callback_for(app.clone()))) {
        Ok(p) => p,
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

    let state_clone = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        run_chat_pipeline(app, state_clone, text, provider).await;
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

fn handle_hotkey_toggle(app: AppHandle, state: Arc<AppState>) {
    // Background 模式下熱鍵語意是「叫醒 + 開錄」一鍵到位,不用先開 tray menu。
    // 切到 Active 後 fall-through 走正常 toggle 邏輯,phase 仍是 Idle 所以
    // 會進到 start_recording。
    if matches!(*state.mode.lock(), Mode::Background) {
        tracing::info!("hotkey while Background → wake to Active + start recording");
        state.set_mode(&app, Mode::Active);
    }

    let current = state.phase.lock().clone();
    match current {
        Phase::Idle | Phase::Done { .. } | Phase::Error { .. } => {
            start_recording(&app, &state);
        }
        Phase::Recording { .. } => {
            stop_and_transcribe(app, state);
        }
        Phase::Transcribing | Phase::Responding { .. } => {
            tracing::info!("toggle while busy — ignored");
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

    let api_key = state.groq_api_key.lock().clone();
    let app_for_provider = app.clone();

    tauri::async_runtime::spawn(async move {
        // Stage 1: Whisper transcribe — **永遠走 Groq**,因為目前只有
        // GroqProvider 實作 transcribe()。Local STT(whisper-rs)會在 phase
        // 5C 接入,屆時這邊也走 factory pattern。
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

            let key = api_key.context(
                "no GROQ_API_KEY configured. \
                 Edit ~/.mori/config.json or set $GROQ_API_KEY",
            )?;
            let stt =
                GroqProvider::new(key, GroqProvider::DEFAULT_CHAT_MODEL.to_string())
                    .with_retry_callback(retry_callback_for(app_for_provider.clone()));
            let transcript = stt.transcribe(wav).await.context("groq transcribe")?;
            tracing::info!(chars = transcript.chars().count(), "transcribed");
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

        // Stage 2: chat provider 走 config-driven factory(可能 Groq、可能
        // Ollama,phase 5A-2 後也可能 Claude CLI)。STT 跟 chat 解耦了,語音
        // 一定走 Groq Whisper,但接 chat 那輪可以 free-pick。
        let provider_arc =
            match mori_core::llm::build_chat_provider(Some(retry_callback_for(app.clone()))) {
                Ok(p) => p,
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
        run_chat_pipeline(app, state, transcript, provider_arc).await;
    });
}

/// 共用的 chat pipeline:給定 transcript + provider,進 Phase::Responding,
/// 呼叫 Agent,把結果回 UI、append 進 conversation history。
///
/// 兩個入口會用到:
/// - `stop_and_transcribe` 從 Whisper 拿到 transcript 後呼叫
/// - `submit_text` IPC command 直接拿 user 打的 text 呼叫(bypass 麥克風)
async fn run_chat_pipeline(
    app: AppHandle,
    state: Arc<AppState>,
    transcript: String,
    provider: Arc<dyn LlmProvider>,
) {
    state.set_phase(
        &app,
        Phase::Responding {
            transcript: transcript.clone(),
        },
    );

    let memory = state.memory.clone();
    let history_snapshot = state.conversation.lock().clone();

    // Phase 3A:抓現場 context(目前只有剪貼簿)。Provider 是 Tauri 平台特定。
    let ctx_provider = context_provider::TauriContextProvider::new(app.clone());
    let ctx = ctx_provider.capture().await;
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

    let chat_result: anyhow::Result<(String, Vec<SkillCallSummary>)> = async {
        let memory_index = memory.read_index_as_context().unwrap_or_default();
        let system_prompt = build_system_prompt(&memory_index, &ctx);
        tracing::debug!(
            index_chars = memory_index.chars().count(),
            history_msgs = history_snapshot.len(),
            has_clipboard = ctx.clipboard.is_some(),
            "calling agent"
        );

        // 註冊 skills
        let memory_for_skills: Arc<dyn MemoryStore> = memory.clone();
        let mut registry = SkillRegistry::new();
        // Memory skills(phase 1D-1F)
        registry.register(Arc::new(RememberSkill::new(memory_for_skills.clone())));
        registry.register(Arc::new(RecallMemorySkill::new(memory_for_skills.clone())));
        registry.register(Arc::new(ForgetMemorySkill::new(memory_for_skills.clone())));
        registry.register(Arc::new(EditMemorySkill::new(memory_for_skills.clone())));
        // Text skills(phase 2)
        registry.register(Arc::new(TranslateSkill::new(provider.clone())));
        registry.register(Arc::new(PolishSkill::new(provider.clone())));
        registry.register(Arc::new(SummarizeSkill::new(provider.clone())));
        registry.register(Arc::new(ComposeSkill::new(provider.clone())));
        // Mode 控制 skill(phase 4B-2):「晚安」/「醒醒」走這條
        let mode_controller: Arc<dyn ModeController> = Arc::new(StateModeController {
            state: state.clone(),
            app: app.clone(),
        });
        registry.register(Arc::new(SetModeSkill::new(mode_controller)));
        // Paste-back skill(phase 4C):反白 → 講話 → 結果取代反白
        // Linux only — 其他平台還沒實作 PasteController(macOS / Windows
        // 各有各的 paste-key 模擬路徑,等之後跨平台 phase 補)。
        #[cfg(target_os = "linux")]
        {
            let paste_controller: Arc<dyn PasteController> =
                Arc::new(crate::selection::LinuxPasteController::new(app.clone()));
            registry.register(Arc::new(PasteSelectionBackSkill::new(paste_controller)));
        }
        let registry = Arc::new(registry);

        let agent = Agent::new(provider, registry);
        let turn = agent
            .respond(&system_prompt, &history_snapshot, &transcript, &ctx)
            .await?;
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

/// 建構 Mori 的 system prompt — 角色 + 時間 + 記憶索引 + 當下 context + tool 規則。
fn build_system_prompt(memory_index: &str, ctx: &MoriContext) -> String {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M (%a)").to_string();
    let mut prompt = String::new();

    prompt.push_str(
        "你是 Mori,一個輕巧、貼心的桌面 AI 管家。背景設定:你是來自 world-tree \
         森林的精靈,被使用者帶到桌面當日常陪伴與助手。\n\n",
    );
    prompt.push_str("回覆規則:\n");
    prompt.push_str("- 一律使用繁體中文,語氣自然、簡潔\n");
    prompt.push_str("- 不寫前言或客套(例如「好的」、「沒問題」、「以下是」)— 直接進主題\n");
    prompt.push_str("- 若使用者問你做不到的事,老實說「目前還沒這個能力」\n");
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
        "  • Linux only — 其他平台沒這個 skill,不會出現在 tool 清單。\n\n");

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
        prompt.push_str("```\n");
        prompt.push_str(sel);
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
        prompt.push_str("```\n");
        prompt.push_str(&preview);
        prompt.push_str("\n```\n");
    }

    if !memory_index.is_empty() {
        prompt.push_str("\n");
        prompt.push_str(memory_index);
    }
    prompt
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
        // 反白即改寫(phase 4C)依賴 wl-clipboard + ydotool。startup 早點警告
        // 比讓 user 試了一次「為什麼沒貼回」再 grep 程式碼好。
        crate::selection::warn_if_setup_missing();
    }

    // 確保 ~/.mori/config.json 存在(第一次跑就會寫一份 stub)
    let config_path = match GroqProvider::bootstrap_mori_config() {
        Ok(p) => Some(p),
        Err(e) => {
            tracing::warn!(?e, "failed to bootstrap ~/.mori/config.json");
            None
        }
    };

    // 建立長期記憶 store。第一次跑會在 ~/.mori/memory/ 建空索引。
    let memory_root = LocalMarkdownMemoryStore::default_root()
        .expect("could not determine ~/.mori/memory path");
    let memory = Arc::new(
        LocalMarkdownMemoryStore::new(memory_root.clone())
            .expect("failed to initialize memory store"),
    );
    tracing::info!(path = %memory_root.display(), "memory store ready");

    let state = Arc::new(AppState {
        phase: Mutex::new(Phase::default()),
        recorder: Mutex::new(None),
        groq_api_key: Mutex::new(None),
        memory,
        conversation: Mutex::new(Vec::new()),
        mode: Mutex::new(Mode::Active),
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
        .manage(state.clone())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            mori_version,
            mori_phase,
            current_phase,
            has_groq_key,
            toggle,
            reset_conversation,
            conversation_length,
            submit_text,
            current_mode,
            set_mode_cmd,
            cancel_recording,
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
            // 注意:**不要**在這裡 setup() 直接 call set_always_on_top —
            // AgentPulse 沒這樣做,他們依賴 conf.json 的 alwaysOnTop hint
            // 處理初始狀態,只在 tray show handler 才 re-assert。
            // 我們先試一樣的紀律,看 mutter 認不認帳。

            // ── 系統匣(tray)+ 選單 ──
            // toggle_mode 項目的 label 會跟著 Mode 動態切換;事件迴圈裡也
            // listen "mode-changed" 同步更新,以免使用者經 IPC / 語音 skill
            // 切了 mode 後 tray label 跟實際狀態對不上。
            let toggle_mode_item =
                MenuItem::with_id(app, "toggle_mode", "休眠(關麥克風)", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &MenuItem::with_id(app, "show", "顯示 Mori", true, None::<&str>)?,
                    &MenuItem::with_id(app, "hide", "隱藏", true, None::<&str>)?,
                    &toggle_mode_item,
                    &MenuItem::with_id(app, "reset", "重新開始對話", true, None::<&str>)?,
                    &MenuItem::with_id(app, "quit", "離開", true, None::<&str>)?,
                ],
            )?;

            let state_for_tray = state_for_setup.clone();
            let toggle_mode_for_handler = toggle_mode_item.clone();
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
                    "toggle_mode" => {
                        let cur = *state_for_tray.mode.lock();
                        let next = match cur {
                            Mode::Active => Mode::Background,
                            Mode::Background => Mode::Active,
                        };
                        state_for_tray.set_mode(app, next);
                        // label 由「mode-changed」listener 統一更新,不要在這
                        // 裡重複寫,避免兩條路徑不一致。
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

            // tray label 跟著 Mode 同步:任何來源(tray 點擊、IPC、skill)改了
            // Mode 都會 emit "mode-changed",這裡統一接 → 改 label。
            app.listen("mode-changed", move |event| {
                let payload = event.payload();
                let label = if payload.contains("\"background\"") {
                    "醒醒(開麥克風)"
                } else {
                    "休眠(關麥克風)"
                };
                if let Err(e) = toggle_mode_for_handler.set_text(label) {
                    tracing::warn!(?e, "tray toggle_mode set_text failed");
                }
            });

            // ── Ollama warm-up(僅當 default_provider=ollama)─────
            // qwen3:8b 5.2GB 在 Intel CPU 沒 GPU 加速首次載入可能要分鐘級。
            // 啟動就背景發一個 1-token chat 觸發 model load,使用者第一次按
            // 熱鍵時模型已熱。Groq 路徑直接 no-op。
            tauri::async_runtime::spawn(async {
                mori_core::llm::warm_up_default_provider().await;
            });

            // ── 全域熱鍵:Ctrl+Alt+Space ───────────────────────────
            // Linux 走 xdg-desktop-portal GlobalShortcuts(Wayland 唯一可行
            // 的路);macOS / Windows 走 tauri-plugin-global-shortcut。
            // 兩條路最後都呼叫 handle_hotkey_toggle。
            #[cfg(target_os = "linux")]
            {
                let app_for_portal = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = portal_hotkey::run(app_for_portal).await {
                        // Not fatal — UI button still works as fallback.
                        // Common reasons: xdg-desktop-portal-gnome not installed,
                        // user denied the permission dialog, or no portal session
                        // (some headless / display-less envs).
                        tracing::error!(
                            ?e,
                            "portal global shortcut unavailable — use the UI \
                             toggle button to trigger Mori"
                        );
                    }
                });

                let handle = app.handle().clone();
                let state_for_handler = state_for_setup.clone();
                app.listen(portal_hotkey::PORTAL_HOTKEY_EVENT, move |_event| {
                    handle_hotkey_toggle(handle.clone(), state_for_handler.clone());
                });

                tracing::info!(
                    "spawned portal hotkey task (Ctrl+Alt+Space) + tray icon"
                );
            }

            #[cfg(not(target_os = "linux"))]
            {
                let shortcut = Shortcut::new(
                    Some(Modifiers::CONTROL | Modifiers::ALT),
                    Code::Space,
                );

                let handle = app.handle().clone();
                let state_for_handler = state_for_setup.clone();

                app.global_shortcut().on_shortcut(
                    shortcut,
                    move |_app, _shortcut, event| {
                        if event.state() != ShortcutState::Pressed {
                            return;
                        }
                        handle_hotkey_toggle(handle.clone(), state_for_handler.clone());
                    },
                )?;

                tracing::info!(
                    "registered global shortcut: Ctrl+Alt+Space + tray icon"
                );
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
