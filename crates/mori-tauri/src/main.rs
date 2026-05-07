// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod recording;

use std::sync::Arc;

use anyhow::Context as _;
use mori_core::llm::groq::GroqProvider;
use mori_core::llm::{ChatMessage, LlmProvider};
use mori_core::memory::markdown::LocalMarkdownMemoryStore;
use mori_core::{PHASE, VERSION};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
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
    /// 完整一輪結束 — 同時帶 transcript 跟 LLM 回應
    Done {
        transcript: String,
        response: String,
    },
    /// 錯誤(任何階段都可以掉到這)
    Error { message: String },
}

impl Default for Phase {
    fn default() -> Self {
        Phase::Idle
    }
}

pub struct AppState {
    pub phase: Mutex<Phase>,
    pub recorder: Mutex<Option<Recorder>>,
    /// 透過 GroqProvider::discover_api_key() 在啟動時嘗試取得;
    /// 若無,transcribe 階段會回 Error。
    pub groq_api_key: Mutex<Option<String>>,
    /// 長期記憶 store。Phase 1C 是 LocalMarkdownMemoryStore;
    /// phase 7+ 換成 SyncedMemoryStore 不重寫上層程式碼。
    pub memory: Arc<LocalMarkdownMemoryStore>,
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

        let chat_result: anyhow::Result<String> = async {
            let memory_context = memory.read_all_as_context().unwrap_or_default();
            let system_prompt = build_system_prompt(&memory_context);
            tracing::debug!(
                memory_chars = memory_context.chars().count(),
                "calling LLM with system prompt"
            );

            let messages = vec![
                ChatMessage {
                    role: "system".into(),
                    content: system_prompt,
                },
                ChatMessage {
                    role: "user".into(),
                    content: transcript.clone(),
                },
            ];
            let resp = provider.chat(messages, vec![]).await.context("groq chat")?;
            let text = resp
                .content
                .ok_or_else(|| anyhow::anyhow!("LLM returned no content"))?;
            Ok(text)
        }
        .await;

        match chat_result {
            Ok(response) => {
                tracing::info!(chars = response.chars().count(), "Mori responded");
                state.set_phase(&app, Phase::Done { transcript, response });
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

/// 建構 Mori 的 system prompt — 角色設定 + 當前時間 + 長期記憶。
fn build_system_prompt(memory_context: &str) -> String {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M (%a)").to_string();
    let mut prompt = String::new();
    prompt.push_str(
        "你是 Mori,一個輕巧、貼心的桌面 AI 管家。背景設定:你是來自 world-tree \
         森林的精靈,被使用者帶到桌面當日常陪伴與助手。\n\n",
    );
    prompt.push_str("回覆規則:\n");
    prompt.push_str("- 一律使用繁體中文,語氣自然、簡潔\n");
    prompt.push_str("- 不寫前言或客套(例如「好的」、「沒問題」、「以下是」)— 直接進主題\n");
    prompt.push_str("- 若使用者問你做不到的事(例如操作系統、開檔案),老實說「目前還沒這個能力」\n");
    prompt.push_str("- 回覆長度配合提問:閒聊就一兩句,問題要解釋才展開\n\n");
    prompt.push_str(&format!("現在時間:{now}\n"));
    if !memory_context.is_empty() {
        prompt.push_str("\n");
        prompt.push_str(memory_context);
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
            toggle
        ])
        .setup(move |app| {
            // 全域熱鍵:F8(單鍵,衝突最少;Wayland 上單鍵攔截行為跟 combo 可能不同)
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

            tracing::info!("registered global shortcut: F8");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
