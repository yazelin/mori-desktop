//! 轉錄頁 IPC commands。
//!
//! Surface:
//! - `transcribe_file(path)` — 單檔轉錄(音檔/影片皆可,ffmpeg 抽音軌)
//! - `transcribe_paths(paths)` — 批次,逐個跑;個別失敗不擋整批
//! - `meeting_recording_start()` / `meeting_recording_stop()` — 長錄音 +
//!   結束時自動轉錄。錄音狀態存 [`AppState::meeting_recorder`]。
//! - `transcribe_check_deps()` — UI 入口檢查 ffmpeg + whisper-server + model
//!   是否齊備,給 UI 顯示「該裝什麼」hint。
//!
//! 進度事件透過 `tauri::Emitter` 推給前端:
//! - `transcribe-file-progress` — 批次過程,payload `{ index, total, path, status }`
//! - `transcribe-chunk-progress` — 單檔長檔分塊,payload `{ chunk, total, path }`
//! - `meeting-duration` — 錄音中每秒 tick 一次,payload `{ secs }`

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use mori_core::transcribe_media::{
    check_ffmpeg, has_supported_extension, transcribe_media_file, transcribe_paths, TranscribeOpts,
    TranscribeResult,
};

use crate::recording::{RecordedAudio, Recorder};
use crate::AppState;

// ─── State shape ────────────────────────────────────────────────────────

/// AppState 上多掛一個 meeting recorder slot。跟 `recorder`(voice-input 短錄)
/// 分開,避免兩種使用情境互踩 — 你錄會議的時候按熱鍵語音輸入不會把會議中斷。
pub struct MeetingState {
    pub recorder: Mutex<Option<Recorder>>,
    pub started_at: Mutex<Option<std::time::Instant>>,
}

impl Default for MeetingState {
    fn default() -> Self {
        Self {
            recorder: Mutex::new(None),
            started_at: Mutex::new(None),
        }
    }
}

// ─── helpers ────────────────────────────────────────────────────────────

/// 取得 transcribe provider — **強制 whisper-local**。轉錄頁是「拿你已經有的
/// 音檔/影片產出逐字稿」場景,送雲端 Groq Whisper 是不必要的網路 + 隱私風險。
/// 想走雲端的 user 自己丟到 Groq playground 比較直接。
fn get_local_provider(
    language: Option<String>,
) -> Result<Arc<dyn mori_core::llm::transcribe::TranscriptionProvider>, String> {
    let p =
        mori_core::llm::whisper_local::LocalWhisperProvider::from_config_with_language_override(
            language,
        )
        .map_err(|e| {
            format!(
                "whisper-local 未配置好:{e}\n\n\
             先確認:\n\
             1. ~/.mori/bin/whisper-server 存在(從 whisper.cpp release 解壓)\n\
             2. ~/.mori/models/ggml-small.bin 存在(從 HuggingFace 抓)\n\
             或在 Config tab 把 providers.whisper-local 路徑寫成你放的位置。"
            )
        })?;
    Ok(Arc::new(p))
}

// ─── Dep check ──────────────────────────────────────────────────────────

#[derive(Serialize, Debug)]
pub struct TranscribeDepStatus {
    pub ffmpeg_ok: bool,
    pub ffmpeg_version: Option<String>,
    pub whisper_binary_ok: bool,
    pub whisper_binary_path: String,
    pub whisper_model_ok: bool,
    pub whisper_model_path: String,
}

#[tauri::command]
pub async fn transcribe_check_deps() -> TranscribeDepStatus {
    let (ffmpeg_ok, ffmpeg_version) = match check_ffmpeg().await {
        Ok(v) => (true, Some(v)),
        Err(_) => (false, None),
    };
    let cfg = mori_core::llm::whisper_local::LocalWhisperProvider::resolved_config();
    TranscribeDepStatus {
        ffmpeg_ok,
        ffmpeg_version,
        whisper_binary_ok: mori_core::llm::whisper_local::server_binary_available(
            &cfg.server_binary,
        ),
        whisper_binary_path: cfg.server_binary.display().to_string(),
        whisper_model_ok: cfg.model_path.exists(),
        whisper_model_path: cfg.model_path.display().to_string(),
    }
}

// ─── Single file ────────────────────────────────────────────────────────

#[derive(Serialize, Debug)]
pub struct TranscribeOutput {
    pub source_path: String,
    pub text: String,
    pub duration_secs: f32,
    pub chunks: u32,
}

impl From<TranscribeResult> for TranscribeOutput {
    fn from(r: TranscribeResult) -> Self {
        Self {
            source_path: r.source_path.display().to_string(),
            text: r.text,
            duration_secs: r.duration_secs,
            chunks: r.chunks,
        }
    }
}

#[derive(Serialize, Clone)]
struct ChunkProgressPayload {
    chunk: u32,
    total: u32,
    path: String,
}

#[tauri::command]
pub async fn transcribe_file_cmd(
    app: AppHandle,
    path: String,
    language: Option<String>,
) -> Result<TranscribeOutput, String> {
    let path_buf = PathBuf::from(&path);
    let provider = get_local_provider(language.clone())?;
    let app_emit = app.clone();
    let progress = Arc::new(move |chunk: u32, total: u32, p: &std::path::Path| {
        let _ = app_emit.emit(
            "transcribe-chunk-progress",
            ChunkProgressPayload {
                chunk,
                total,
                path: p.display().to_string(),
            },
        );
    }) as mori_core::transcribe_media::ProgressFn;

    let opts = TranscribeOpts {
        language: None,
        chunk_seconds: None, // 走 default 300s
    };

    let result = transcribe_media_file(&path_buf, provider, opts, Some(progress))
        .await
        .map_err(|e| format!("{e:#}"))?;
    Ok(result.into())
}

// ─── Batch ──────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
struct FileProgressPayload {
    index: usize,
    total: usize,
    path: String,
    status: String, // "start" | "ok" | "err"
}

#[derive(Serialize, Debug)]
pub struct BatchEntry {
    pub source_path: String,
    pub ok: bool,
    pub text: String,
    pub error: Option<String>,
    pub duration_secs: f32,
    pub chunks: u32,
}

#[tauri::command]
pub async fn transcribe_paths_cmd(
    app: AppHandle,
    paths: Vec<String>,
    language: Option<String>,
) -> Result<Vec<BatchEntry>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let provider = get_local_provider(language.clone())?;
    let app_emit = app.clone();
    let file_progress = Arc::new(
        move |index: usize, total: usize, p: &std::path::Path, status: &str| {
            let _ = app_emit.emit(
                "transcribe-file-progress",
                FileProgressPayload {
                    index,
                    total,
                    path: p.display().to_string(),
                    status: status.to_string(),
                },
            );
        },
    ) as Arc<dyn Fn(usize, usize, &std::path::Path, &str) + Send + Sync>;

    let app_emit2 = app.clone();
    let chunk_progress = Arc::new(move |chunk: u32, total: u32, p: &std::path::Path| {
        let _ = app_emit2.emit(
            "transcribe-chunk-progress",
            ChunkProgressPayload {
                chunk,
                total,
                path: p.display().to_string(),
            },
        );
    }) as mori_core::transcribe_media::ProgressFn;

    let input_paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let opts = TranscribeOpts {
        language: None,
        chunk_seconds: None,
    };

    let results = transcribe_paths(
        &input_paths,
        provider,
        opts,
        Some(file_progress),
        Some(chunk_progress),
    )
    .await;
    let entries: Vec<BatchEntry> = input_paths
        .iter()
        .zip(results.into_iter())
        .map(|(path, r)| match r {
            Ok(res) => BatchEntry {
                source_path: path.display().to_string(),
                ok: true,
                text: res.text,
                error: None,
                duration_secs: res.duration_secs,
                chunks: res.chunks,
            },
            Err(e) => BatchEntry {
                source_path: path.display().to_string(),
                ok: false,
                text: String::new(),
                error: Some(format!("{e:#}")),
                duration_secs: 0.0,
                chunks: 0,
            },
        })
        .collect();
    Ok(entries)
}

// ─── Folder enumeration ─────────────────────────────────────────────────

#[derive(Serialize, Debug)]
pub struct FolderScanEntry {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
}

/// 列資料夾內所有可轉錄檔(只看副檔名;非遞迴 — user 期望「丟一個會議資料夾」
/// 就把那層裡的檔挑出來,不挖子目錄)。
#[tauri::command]
pub async fn transcribe_scan_folder(folder: String) -> Result<Vec<FolderScanEntry>, String> {
    let dir = PathBuf::from(&folder);
    let read = std::fs::read_dir(&dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    let mut out = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if !has_supported_extension(&path) {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        out.push(FolderScanEntry {
            path: path.display().to_string(),
            name,
            size_bytes: size,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

// ─── Save transcript to .txt next to source ─────────────────────────────

/// 把單檔轉錄結果存成 `<source>.transcript.txt`(同目錄)。批次 UI 用。
#[tauri::command]
pub fn transcribe_save_alongside(source_path: String, text: String) -> Result<String, String> {
    let src = PathBuf::from(&source_path);
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("invalid source path: {source_path}"))?;
    let dir = src
        .parent()
        .ok_or_else(|| format!("no parent dir for {source_path}"))?;
    let out = dir.join(format!("{stem}.transcript.txt"));
    std::fs::write(&out, text).map_err(|e| format!("write {}: {e}", out.display()))?;
    Ok(out.display().to_string())
}

// ─── Meeting recording ──────────────────────────────────────────────────

#[derive(Serialize, Debug)]
pub struct MeetingStatus {
    pub recording: bool,
    pub duration_secs: u64,
}

#[tauri::command]
pub fn meeting_recording_status(state: State<'_, Arc<AppState>>) -> MeetingStatus {
    let active = state.meeting.recorder.lock().is_some();
    let secs = state
        .meeting
        .started_at
        .lock()
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(0);
    MeetingStatus {
        recording: active,
        duration_secs: secs,
    }
}

#[tauri::command]
pub fn meeting_recording_start(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let mut slot = state.meeting.recorder.lock();
    if slot.is_some() {
        return Err("meeting recorder already running".into());
    }
    let rec = Recorder::start().map_err(|e| format!("start meeting recorder: {e:#}"))?;
    *slot = Some(rec);
    *state.meeting.started_at.lock() = Some(std::time::Instant::now());
    tracing::info!("meeting recording started");
    Ok(())
}

/// 停止錄音 → 立刻 spawn task 把 WAV 寫到 temp + 轉錄,結束發 `meeting-transcribed`
/// 事件帶結果。同步回傳「dump 出來的 WAV 路徑 + 預期會在 X 秒後完成」讓 UI
/// 立刻把錄音 UI 切回去 + 顯示「轉錄中」狀態。
#[derive(Serialize, Debug)]
pub struct MeetingStopAck {
    pub wav_path: String,
    pub duration_secs: f32,
}

#[derive(Serialize, Clone)]
struct MeetingTranscribedPayload {
    wav_path: String,
    text: String,
    duration_secs: f32,
    chunks: u32,
    error: Option<String>,
}

#[tauri::command]
pub async fn meeting_recording_stop(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    language: Option<String>,
) -> Result<MeetingStopAck, String> {
    // 1. 取出 recorder,讓 mutex 立刻釋放
    let rec_opt = state.meeting.recorder.lock().take();
    let started_at = state.meeting.started_at.lock().take();
    let rec = rec_opt.ok_or_else(|| "no active meeting recording".to_string())?;
    let audio: RecordedAudio = rec.stop().map_err(|e| format!("stop recorder: {e:#}"))?;
    let duration_secs = audio.duration_secs();
    let elapsed = started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
    tracing::info!(
        duration_secs,
        wall_secs = elapsed,
        samples = audio.samples.len(),
        "meeting stopped, encoding WAV"
    );

    // 2. 編碼成 WAV,寫到 ~/.mori/meetings/YYYYMMDD-HHMMSS.wav
    let wav_bytes = audio
        .to_wav_bytes()
        .map_err(|e| format!("encode WAV: {e:#}"))?;
    let dir = mori_home_dir().join("meetings");
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let wav_path = dir.join(format!("meeting-{stamp}.wav"));
    std::fs::write(&wav_path, &wav_bytes)
        .map_err(|e| format!("write WAV {}: {e}", wav_path.display()))?;
    tracing::info!(path = %wav_path.display(), bytes = wav_bytes.len(), "meeting WAV saved");

    // 3. 同步回 ack(UI 立刻可切換顯示)
    let ack = MeetingStopAck {
        wav_path: wav_path.display().to_string(),
        duration_secs,
    };

    // 4. 背景 task 跑轉錄;完成發事件
    let app_for_task = app.clone();
    let wav_for_task = wav_path.clone();
    tauri::async_runtime::spawn(async move {
        let provider = match get_local_provider(language.clone()) {
            Ok(p) => p,
            Err(e) => {
                let _ = app_for_task.emit(
                    "meeting-transcribed",
                    MeetingTranscribedPayload {
                        wav_path: wav_for_task.display().to_string(),
                        text: String::new(),
                        duration_secs,
                        chunks: 0,
                        error: Some(e),
                    },
                );
                return;
            }
        };
        let app_for_chunk = app_for_task.clone();
        let wav_for_chunk = wav_for_task.clone();
        let progress = Arc::new(move |chunk: u32, total: u32, _p: &std::path::Path| {
            let _ = app_for_chunk.emit(
                "transcribe-chunk-progress",
                ChunkProgressPayload {
                    chunk,
                    total,
                    path: wav_for_chunk.display().to_string(),
                },
            );
        }) as mori_core::transcribe_media::ProgressFn;

        let opts = TranscribeOpts {
            language: None,
            chunk_seconds: None,
        };

        let payload =
            match transcribe_media_file(&wav_for_task, provider, opts, Some(progress)).await {
                Ok(r) => MeetingTranscribedPayload {
                    wav_path: wav_for_task.display().to_string(),
                    text: r.text,
                    duration_secs: r.duration_secs.max(duration_secs),
                    chunks: r.chunks,
                    error: None,
                },
                Err(e) => MeetingTranscribedPayload {
                    wav_path: wav_for_task.display().to_string(),
                    text: String::new(),
                    duration_secs,
                    chunks: 0,
                    error: Some(format!("{e:#}")),
                },
            };
        let _ = app_for_task.emit("meeting-transcribed", payload);
    });

    Ok(ack)
}

// 借用 main.rs 的 mori dir helper;不直接 import 避免循環。複製簡單版。
fn mori_home_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".mori")
}
