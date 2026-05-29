//! 轉錄頁 IPC commands。
//!
//! Surface:
//! - `transcribe_file(path)` — 單檔轉錄(音檔/影片皆可,ffmpeg 抽音軌)
//! - `transcribe_paths(paths)` — 批次,逐個跑;個別失敗不擋整批
//! - `transcribe_check_deps()` — UI 入口檢查 ffmpeg + whisper-server + model
//!   是否齊備,給 UI 顯示「該裝什麼」hint。
//!
//! 進度事件透過 `tauri::Emitter` 推給前端:
//! - `transcribe-file-progress` — 批次過程,payload `{ index, total, path, status }`
//! - `transcribe-chunk-progress` — 單檔長檔分塊,payload `{ chunk, total, path }`

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use mori_core::transcribe_media::{
    check_ffmpeg, has_supported_extension, transcribe_media_file, transcribe_paths, TranscribeOpts,
    TranscribeResult,
};

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

