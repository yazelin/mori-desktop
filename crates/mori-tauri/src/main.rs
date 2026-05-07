// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod recording;

use std::sync::Arc;

use anyhow::Context as _;
use mori_core::agent::{Agent, SkillCallSummary};
use mori_core::context::Context as MoriContext;
use mori_core::llm::groq::GroqProvider;
use mori_core::llm::{ChatMessage, LlmProvider};
use mori_core::memory::markdown::LocalMarkdownMemoryStore;
use mori_core::memory::MemoryStore;
use mori_core::skill::{
    EditMemorySkill, ForgetMemorySkill, RecallMemorySkill, RememberSkill, SkillRegistry,
};
use mori_core::{PHASE, VERSION};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut, ShortcutState};

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
}

impl AppState {
    fn set_phase(&self, app: &AppHandle, new_phase: Phase) {
        tracing::info!(?new_phase, "phase change");
        *self.phase.lock() = new_phase.clone();
        if let Err(e) = app.emit("phase-changed", &new_phase) {
            tracing::warn!(?e, "failed to emit phase-changed");
        }
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

// ─── 熱鍵 / toggle 處理 ─────────────────────────────────────────────

fn handle_hotkey_toggle(app: AppHandle, state: Arc<AppState>) {
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

    let memory = state.memory.clone();

    tauri::async_runtime::spawn(async move {
        // Stage 1: Whisper
        let transcribe_result: anyhow::Result<(String, GroqProvider)> = async {
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
            let provider =
                GroqProvider::new(key, GroqProvider::DEFAULT_CHAT_MODEL.to_string());
            let transcript = provider.transcribe(wav).await.context("groq transcribe")?;
            tracing::info!(chars = transcript.chars().count(), "transcribed");
            Ok((transcript, provider))
        }
        .await;

        let (transcript, provider) = match transcribe_result {
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

        // Stage 2: Mori 用 LLM 回應
        state.set_phase(
            &app,
            Phase::Responding {
                transcript: transcript.clone(),
            },
        );

        // 把當前 history snapshot 出來給 agent 用(避免拿著 lock 跑 await)
        let history_snapshot = state.conversation.lock().clone();

        let chat_result: anyhow::Result<(String, Vec<SkillCallSummary>)> = async {
            let memory_index = memory.read_index_as_context().unwrap_or_default();
            let system_prompt = build_system_prompt(&memory_index);
            tracing::debug!(
                index_chars = memory_index.chars().count(),
                history_msgs = history_snapshot.len(),
                "calling agent"
            );

            let provider: Arc<dyn LlmProvider> = Arc::new(provider);

            // 註冊 phase 1F skills:remember / recall / forget / edit
            let memory_for_skills: Arc<dyn MemoryStore> = memory.clone();
            let mut registry = SkillRegistry::new();
            registry.register(Arc::new(RememberSkill::new(memory_for_skills.clone())));
            registry.register(Arc::new(RecallMemorySkill::new(memory_for_skills.clone())));
            registry.register(Arc::new(ForgetMemorySkill::new(memory_for_skills.clone())));
            registry.register(Arc::new(EditMemorySkill::new(memory_for_skills.clone())));
            let registry = Arc::new(registry);

            let agent = Agent::new(provider, registry);
            let ctx = MoriContext::default();
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
    });
}

/// 建構 Mori 的 system prompt — 角色 + 時間 + 記憶索引 + tool 規則。
fn build_system_prompt(memory_index: &str) -> String {
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

    prompt.push_str(&format!("現在時間:{now}\n"));
    if !memory_index.is_empty() {
        prompt.push_str("\n");
        prompt.push_str(memory_index);
    }
    prompt
}

// ─── main ───────────────────────────────────────────────────────────

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mori_tauri=debug,mori_core=debug".into()),
        )
        .init();

    tracing::info!("Mori starting — phase {}", PHASE);

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
            // ── 系統匣(tray)+ 選單 ──
            let menu = Menu::with_items(
                app,
                &[
                    &MenuItem::with_id(app, "show", "顯示 Mori", true, None::<&str>)?,
                    &MenuItem::with_id(app, "hide", "隱藏", true, None::<&str>)?,
                    &MenuItem::with_id(app, "reset", "重新開始對話", true, None::<&str>)?,
                    &MenuItem::with_id(app, "quit", "離開", true, None::<&str>)?,
                ],
            )?;

            let state_for_tray = state_for_setup.clone();
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
                    }
                    "hide" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.hide();
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

            // ── 全域熱鍵:F8(Wayland 上常被擋,有 toggle 按鈕當 fallback)──
            let shortcut = Shortcut::new(None, Code::F8);

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

            tracing::info!("registered global shortcut: F8 + tray icon");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
