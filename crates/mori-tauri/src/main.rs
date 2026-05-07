// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod recording;

use std::sync::Arc;

use anyhow::Context as _;
use mori_core::llm::groq::GroqProvider;
use mori_core::llm::LlmProvider;
use mori_core::{PHASE, VERSION};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
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
    /// 拿到逐字稿
    Done { transcript: String },
    /// 錯誤(任何階段都可以掉到這)
    Error { message: String },
}

impl Default for Phase {
    fn default() -> Self {
        Phase::Idle
    }
}

#[derive(Default)]
pub struct AppState {
    pub phase: Mutex<Phase>,
    pub recorder: Mutex<Option<Recorder>>,
    /// 透過 GroqProvider::discover_api_key() 在啟動時嘗試取得;
    /// 若無,transcribe 階段會回 Error。
    pub groq_api_key: Mutex<Option<String>>,
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
        Phase::Transcribing => {
            tracing::info!("toggle while transcribing — ignored");
        }
    }
}

fn start_recording(app: &AppHandle, state: &Arc<AppState>) {
    match Recorder::start() {
        Ok(rec) => {
            *state.recorder.lock() = Some(rec);
            let now_ms = chrono::Utc::now().timestamp_millis();
            state.set_phase(
                app,
                Phase::Recording {
                    started_at_ms: now_ms,
                },
            );
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

    tauri::async_runtime::spawn(async move {
        let result: anyhow::Result<String> = async {
            let audio = recorder.stop().context("stop recorder")?;
            let duration = audio.duration_secs();
            // 計算音量:RMS,讓我們知道是不是麥克風根本沒收到聲音
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

            // Debug:把最後一次錄音存到 /tmp,使用者可以播聽看
            let debug_path = std::env::temp_dir().join("mori-last-recording.wav");
            if let Err(e) = std::fs::write(&debug_path, &wav) {
                tracing::warn!(?e, "failed to write debug WAV");
            } else {
                tracing::info!(path = %debug_path.display(), "wrote debug WAV");
            }

            let key = api_key.context(
                "no GROQ_API_KEY configured. \
                 Edit ~/.mori/config.json or set $GROQ_API_KEY",
            )?;
            let provider =
                GroqProvider::new(key, GroqProvider::DEFAULT_CHAT_MODEL.to_string());
            let text = provider.transcribe(wav).await.context("groq transcribe")?;
            Ok(text)
        }
        .await;

        match result {
            Ok(transcript) => {
                tracing::info!(chars = transcript.chars().count(), "transcribed");
                state.set_phase(&app, Phase::Done { transcript });
            }
            Err(e) => {
                tracing::error!(?e, "transcribe pipeline failed");
                state.set_phase(
                    &app,
                    Phase::Error {
                        message: format!("{e:#}"),
                    },
                );
            }
        }
    });
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

    let state = Arc::new(AppState::default());

    // 確保 ~/.mori/config.json 存在(第一次跑就會寫一份 stub)
    let config_path = match GroqProvider::bootstrap_mori_config() {
        Ok(p) => Some(p),
        Err(e) => {
            tracing::warn!(?e, "failed to bootstrap ~/.mori/config.json");
            None
        }
    };

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
            // 註冊全域熱鍵:Ctrl+Alt+M
            let shortcut = Shortcut::new(
                Some(Modifiers::CONTROL | Modifiers::ALT),
                Code::KeyM,
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

            tracing::info!("registered global shortcut: Ctrl+Alt+M");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
