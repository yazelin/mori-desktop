//! Per-pipeline recordings archive(Phase B — 之前一直留 backlog 的)。
//!
//! 每次完整 voice pipeline run(wake → record → STT → speaker_id → evaluator
//! → agent → response)存一個 session 資料夾到 `~/.mori/recordings/<timestamp>/`,
//! 包含完整 I/O:
//!
//! ```text
//! ~/.mori/recordings/2026-05-19T17-12-34/
//! ├── audio-raw.flac        錄音原檔(silence-trim 前;lossless 壓縮)
//! ├── audio-trimmed.flac    送 STT 的版本(silence-trim 後;lossless 壓縮)
//! ├── transcript.txt        STT 出來的文字
//! ├── response.txt          Mori final response
//! └── meta.json             完整 metadata(provider / profile / score / 時間軸)
//! ```
//!
//! ## 用途
//!
//! - **Whisper fine-tune dataset** — user 自己錄 + 自己 correct → 訓 personal STT
//! - **Debug** — 哪輪講錯話 / Mori 沒抓到意圖 → 直接回放
//! - **隱私自管** — user 知道 Mori 收了什麼,要刪自己刪
//!
//! ## Config
//!
//! - `recordings.enabled`(預設 true)
//! - `recordings.retention_days`(預設 14;設 0 = 永不清)
//!
//! 開機 + 每次 finalize 後跑一次 cleanup(刪超過 retention_days 的 session)。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::mori_dir;

const AUDIO_COMPRESSION_CODEC: &str = "flac-lossless";

/// 一次 pipeline run 的累積資料。`finalize()` 時一口氣寫進 session dir。
///
/// 設計成 setter-based 累積 — pipeline 各階段(stop_and_transcribe / agent_loop)
/// 拿到對應資料就 set,不依賴順序。最後 `finalize()` 才落盤。
#[derive(Debug, Default)]
pub struct SessionRecord {
    started_at: Option<SystemTime>,
    mode: Option<String>,
    audio_raw_bytes: Option<Vec<u8>>,
    audio_trimmed_bytes: Option<Vec<u8>>,
    transcript: Option<String>,
    response: Option<String>,
    skill_calls: Option<serde_json::Value>,
    profile: Option<String>,
    provider: Option<String>,
    wake_score: Option<f32>,
    speaker_id: Option<SpeakerIdSnapshot>,
    evaluator: Option<EvaluatorSnapshot>,
    timings_ms: TimingsSnapshot,
    /// 組好給 LLM 的完整 system prompt(persona + memory index + context section
    /// + skill rules)。可能 5-50KB,寫獨立 system-prompt.txt 不塞 meta.json。
    system_prompt: Option<String>,
    /// 「按熱鍵那一瞬間」的環境快照(clipboard / selection / active window / URLs)。
    /// 寫獨立 context.json 結構化保存。
    context_snapshot: Option<serde_json::Value>,
    /// History snapshot — LLM 看到的對話歷史條目數 + token 大概值。
    history_summary: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SpeakerIdSnapshot {
    pub enabled: bool,
    pub score: Option<f32>,
    pub threshold: Option<f32>,
    pub pass: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvaluatorSnapshot {
    pub enabled: bool,
    pub intent: Option<String>,
    pub reason: Option<String>,
    pub confidence: Option<f32>,
    pub skipped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TimingsSnapshot {
    pub recording_ms: Option<u64>,
    pub stt_ms: Option<u64>,
    pub speaker_id_ms: Option<u64>,
    pub evaluator_ms: Option<u64>,
    pub agent_ms: Option<u64>,
}

impl SessionRecord {
    pub fn new(mode: &str) -> Self {
        Self {
            started_at: Some(SystemTime::now()),
            mode: Some(mode.to_string()),
            ..Default::default()
        }
    }

    pub fn set_audio_raw(&mut self, bytes: Vec<u8>) {
        self.audio_raw_bytes = Some(bytes);
    }
    pub fn set_audio_trimmed(&mut self, bytes: Vec<u8>) {
        self.audio_trimmed_bytes = Some(bytes);
    }
    pub fn set_transcript(&mut self, t: String) {
        self.transcript = Some(t);
    }
    pub fn set_response(&mut self, r: String) {
        self.response = Some(r);
    }
    pub fn set_skill_calls(&mut self, v: serde_json::Value) {
        self.skill_calls = Some(v);
    }
    pub fn set_profile(&mut self, p: String) {
        self.profile = Some(p);
    }
    pub fn set_provider(&mut self, p: String) {
        self.provider = Some(p);
    }
    pub fn set_wake_score(&mut self, s: f32) {
        self.wake_score = Some(s);
    }
    pub fn set_speaker_id(&mut self, s: SpeakerIdSnapshot) {
        self.speaker_id = Some(s);
    }
    pub fn set_evaluator(&mut self, e: EvaluatorSnapshot) {
        self.evaluator = Some(e);
    }
    pub fn add_recording_ms(&mut self, ms: u64) {
        self.timings_ms.recording_ms = Some(ms);
    }
    pub fn add_stt_ms(&mut self, ms: u64) {
        self.timings_ms.stt_ms = Some(ms);
    }
    pub fn add_speaker_id_ms(&mut self, ms: u64) {
        self.timings_ms.speaker_id_ms = Some(ms);
    }
    pub fn add_evaluator_ms(&mut self, ms: u64) {
        self.timings_ms.evaluator_ms = Some(ms);
    }
    pub fn add_agent_ms(&mut self, ms: u64) {
        self.timings_ms.agent_ms = Some(ms);
    }
    pub fn set_system_prompt(&mut self, p: String) {
        self.system_prompt = Some(p);
    }
    pub fn set_context_snapshot(&mut self, v: serde_json::Value) {
        self.context_snapshot = Some(v);
    }
    pub fn set_history_summary(&mut self, v: serde_json::Value) {
        self.history_summary = Some(v);
    }

    /// 寫進 `~/.mori/recordings/<timestamp>/`。
    ///
    /// 失敗只 log warn,不阻斷 Phase::Done 流程。recordings disabled → 直接 return。
    pub fn finalize(self) {
        let cfg = read_config();
        if !cfg.enabled {
            return;
        }
        let Some(started_at) = self.started_at else {
            tracing::warn!("recordings: SessionRecord.finalize without started_at — skipping");
            return;
        };
        // Empty session(沒 transcript 也沒 response)→ 不存,避免 wake 誤觸的空檔
        let has_content = self.transcript.is_some()
            || self.response.is_some()
            || self.audio_raw_bytes.is_some();
        if !has_content {
            return;
        }

        let dir_name = format_timestamp(started_at);
        let dir = recordings_root().join(&dir_name);
        if let Err(e) = fs::create_dir_all(&dir) {
            tracing::warn!(error = %e, dir = %dir.display(), "recordings: mkdir failed");
            return;
        }

        // Audio: write FLAC when ffmpeg is available. FLAC is lossless, so future
        // voice-training datasets keep the original 16-bit PCM quality with much
        // smaller disk usage than WAV. If encoding fails, keep a WAV fallback.
        let mut audio_raw_file: Option<&'static str> = None;
        let mut audio_trimmed_file: Option<&'static str> = None;
        let mut audio_compression: Option<&'static str> = None;
        if let Some(bytes) = self.audio_raw_bytes {
            let written = write_audio_variant(&dir, "audio-raw", &bytes);
            if written.ends_with(".flac") {
                audio_compression = Some(AUDIO_COMPRESSION_CODEC);
            }
            audio_raw_file = Some(written);
        }
        if let Some(bytes) = self.audio_trimmed_bytes {
            let written = write_audio_variant(&dir, "audio-trimmed", &bytes);
            if written.ends_with(".flac") {
                audio_compression = Some(AUDIO_COMPRESSION_CODEC);
            }
            audio_trimmed_file = Some(written);
        }
        // Transcript & response
        if let Some(t) = self.transcript.as_deref() {
            write_file(&dir.join("transcript.txt"), t.as_bytes(), "transcript.txt");
        }
        if let Some(r) = self.response.as_deref() {
            write_file(&dir.join("response.txt"), r.as_bytes(), "response.txt");
        }
        // System prompt(可能 5-50KB,獨立檔)
        if let Some(p) = self.system_prompt.as_deref() {
            write_file(&dir.join("system-prompt.txt"), p.as_bytes(), "system-prompt.txt");
        }
        // Context snapshot — clipboard / selection / window / urls,結構化
        if let Some(ctx) = &self.context_snapshot {
            if let Ok(text) = serde_json::to_string_pretty(ctx) {
                write_file(&dir.join("context.json"), text.as_bytes(), "context.json");
            }
        }
        if let Some(hist) = &self.history_summary {
            if let Ok(text) = serde_json::to_string_pretty(hist) {
                write_file(&dir.join("history.json"), text.as_bytes(), "history.json");
            }
        }

        // meta.json
        let total_ms = started_at
            .elapsed()
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let meta = serde_json::json!({
            "timestamp": iso_timestamp(started_at),
            "mode": self.mode,
            "profile": self.profile,
            "provider": self.provider,
            "wake_score": self.wake_score,
            "speaker_id": self.speaker_id,
            "evaluator": self.evaluator,
            "skill_calls": self.skill_calls,
            "audio": {
                "raw_file": audio_raw_file,
                "trimmed_file": audio_trimmed_file,
                "compression": audio_compression,
            },
            "timings_ms": serde_json::json!({
                "recording_ms": self.timings_ms.recording_ms,
                "stt_ms": self.timings_ms.stt_ms,
                "speaker_id_ms": self.timings_ms.speaker_id_ms,
                "evaluator_ms": self.timings_ms.evaluator_ms,
                "agent_ms": self.timings_ms.agent_ms,
                "total_ms": total_ms,
            }),
        });
        if let Ok(text) = serde_json::to_string_pretty(&meta) {
            write_file(&dir.join("meta.json"), text.as_bytes(), "meta.json");
        }

        tracing::info!(
            dir = %dir.display(),
            "recordings: session archived",
        );
    }
}

fn write_audio_variant(dir: &Path, stem: &'static str, wav_bytes: &[u8]) -> &'static str {
    let flac_name = match stem {
        "audio-raw" => "audio-raw.flac",
        "audio-trimmed" => "audio-trimmed.flac",
        _ => "audio.flac",
    };
    match encode_wav_to_flac(wav_bytes) {
        Ok(flac) => {
            write_file(&dir.join(flac_name), &flac, flac_name);
            flac_name
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                file = stem,
                "recordings: FLAC encode failed, keeping WAV fallback",
            );
            let wav_name = match stem {
                "audio-raw" => "audio-raw.wav",
                "audio-trimmed" => "audio-trimmed.wav",
                _ => "audio.wav",
            };
            write_file(&dir.join(wav_name), wav_bytes, wav_name);
            wav_name
        }
    }
}

fn encode_wav_to_flac(wav_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let temp_dir = std::env::temp_dir();
    let wav_path = temp_dir.join(format!("mori-recording-{pid}-{nonce}.wav"));
    let flac_path = temp_dir.join(format!("mori-recording-{pid}-{nonce}.flac"));

    fs::write(&wav_path, wav_bytes).map_err(|e| format!("write temp WAV: {e}"))?;
    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
        ])
        .arg(&wav_path)
        .args([
            "-map",
            "0:a:0",
            "-sample_fmt",
            "s16",
            "-compression_level",
            "8",
        ])
        .arg(&flac_path)
        .output()
        .map_err(|e| {
            let _ = fs::remove_file(&wav_path);
            let _ = fs::remove_file(&flac_path);
            format!("spawn ffmpeg: {e}")
        })?;
    let _ = fs::remove_file(&wav_path);

    if !output.status.success() {
        let _ = fs::remove_file(&flac_path);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg exited {}: {}", output.status, stderr.trim()));
    }
    let flac = fs::read(&flac_path).map_err(|e| format!("read temp FLAC: {e}"))?;
    let _ = fs::remove_file(&flac_path);
    if flac.is_empty() {
        return Err("ffmpeg produced empty FLAC".to_string());
    }
    Ok(flac)
}

fn write_file(path: &Path, bytes: &[u8], label: &str) {
    if let Err(e) = fs::write(path, bytes) {
        tracing::warn!(error = %e, file = label, "recordings: write failed");
    }
}

// ── Config ────────────────────────────────────────────────────────────────

struct RecordingsConfig {
    enabled: bool,
    retention_days: u32,
}

fn read_config() -> RecordingsConfig {
    let default = RecordingsConfig {
        enabled: true,
        retention_days: 14,
    };
    let path = mori_dir().join("config.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return default;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return default;
    };
    let enabled = json
        .pointer("/recordings/enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let retention_days = json
        .pointer("/recordings/retention_days")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(14);
    RecordingsConfig {
        enabled,
        retention_days,
    }
}

fn recordings_root() -> PathBuf {
    mori_dir().join("recordings")
}

// ── Timestamp helpers ─────────────────────────────────────────────────────

/// 用作 dir name:`2026-05-19T17-12-34-123` filesystem-safe(`:` 在 Windows 是
/// 路徑分隔符,所以 ISO 8601 的 `T` 後也用 `-`)。
fn format_timestamp(t: SystemTime) -> String {
    let datetime: chrono::DateTime<chrono::Utc> = t.into();
    datetime.format("%Y-%m-%dT%H-%M-%S-%3f").to_string()
}

/// Meta.json 內用 proper ISO 8601(`:` 沒問題 in JSON value)。
fn iso_timestamp(t: SystemTime) -> String {
    let datetime: chrono::DateTime<chrono::Utc> = t.into();
    datetime.to_rfc3339()
}

// ── Cleanup(超過 retention_days 自動刪)──────────────────────────────────

static CLEANUP_RAN_THIS_SESSION: AtomicBool = AtomicBool::new(false);

/// 刪超過 retention_days 的舊 session。一次 mori-tauri 跑只跑一次(idempotent
/// gate),避免每次 finalize 都 IO 掃全資料夾。
pub fn cleanup_old_if_needed() {
    if CLEANUP_RAN_THIS_SESSION.swap(true, Ordering::SeqCst) {
        return;
    }
    let cfg = read_config();
    if cfg.retention_days == 0 {
        tracing::debug!("recordings: retention_days=0,跳過 cleanup");
        return;
    }
    let root = recordings_root();
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    let cutoff = SystemTime::now()
        - std::time::Duration::from_secs(cfg.retention_days as u64 * 24 * 3600);
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        if modified >= cutoff {
            continue;
        }
        if let Err(e) = fs::remove_dir_all(&path) {
            tracing::warn!(
                error = %e,
                dir = %path.display(),
                "recordings: cleanup remove failed",
            );
        } else {
            removed += 1;
        }
    }
    if removed > 0 {
        tracing::info!(
            removed,
            retention_days = cfg.retention_days,
            "recordings: cleanup removed old sessions",
        );
    }
}

// ── IPC commands(給 RecordingsTab UI)─────────────────────────────────────

/// 列表 summary — 一筆 per session,給 UI 列表顯示。詳細內容 lazy load 另一條 IPC。
#[derive(Debug, Serialize)]
pub struct SessionSummary {
    pub timestamp: String,           // dir name(已 url-safe)
    pub iso_time: String,            // ISO 8601(從 meta.json 讀,fallback dir name)
    pub mode: Option<String>,
    pub profile: Option<String>,
    pub provider: Option<String>,
    pub transcript_preview: Option<String>, // 前 60 字
    pub response_preview: Option<String>,
    pub duration_ms: Option<u64>,
    pub size_bytes: u64,             // 整個 dir 大小
}

/// List 全部 session(newest first)。給 RecordingsTab mount + refresh 用。
#[tauri::command]
pub fn recordings_list() -> Vec<SessionSummary> {
    let root = recordings_root();
    let Ok(entries) = fs::read_dir(&root) else {
        return vec![];
    };
    let mut out: Vec<SessionSummary> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let path = e.path();
            let timestamp = path.file_name()?.to_string_lossy().to_string();
            let summary = build_session_summary(&path, &timestamp);
            Some(summary)
        })
        .collect();
    // Newest first(dir name 是 ISO 字典序,直接 reverse 排)
    out.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    out
}

fn build_session_summary(dir: &Path, timestamp: &str) -> SessionSummary {
    let meta_text = fs::read_to_string(dir.join("meta.json")).unwrap_or_default();
    let meta: serde_json::Value = serde_json::from_str(&meta_text).unwrap_or(serde_json::Value::Null);
    let transcript = fs::read_to_string(dir.join("transcript.txt")).ok();
    let response = fs::read_to_string(dir.join("response.txt")).ok();
    let size_bytes = dir_size(dir);
    SessionSummary {
        timestamp: timestamp.to_string(),
        iso_time: meta
            .pointer("/timestamp")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| timestamp.to_string()),
        mode: meta.pointer("/mode").and_then(|v| v.as_str()).map(|s| s.to_string()),
        profile: meta.pointer("/profile").and_then(|v| v.as_str()).map(|s| s.to_string()),
        provider: meta.pointer("/provider").and_then(|v| v.as_str()).map(|s| s.to_string()),
        transcript_preview: transcript.as_ref().map(|s| preview(s, 60)),
        response_preview: response.as_ref().map(|s| preview(s, 60)),
        duration_ms: meta.pointer("/timings_ms/total_ms").and_then(|v| v.as_u64()),
        size_bytes,
    }
}

fn dir_size(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

fn preview(s: &str, max_chars: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max_chars).collect::<String>())
    }
}

/// 完整 session 細節 — meta / context / history / transcript / response。
#[derive(Debug, Serialize, Default)]
pub struct SessionDetail {
    pub timestamp: String,
    pub meta: Option<serde_json::Value>,
    pub context: Option<serde_json::Value>,
    pub history: Option<serde_json::Value>,
    pub transcript: Option<String>,
    pub response: Option<String>,
    pub system_prompt: Option<String>,
    pub has_audio_raw: bool,
    pub has_audio_trimmed: bool,
    pub audio_raw_format: Option<String>,
    pub audio_trimmed_format: Option<String>,
}

#[tauri::command]
pub fn recordings_session_detail(timestamp: String) -> Result<SessionDetail, String> {
    let dir = recordings_root().join(sanitize_timestamp(&timestamp)?);
    if !dir.is_dir() {
        return Err(format!("session not found: {timestamp}"));
    }
    let read_json = |name: &str| -> Option<serde_json::Value> {
        fs::read_to_string(dir.join(name))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
    };
    let read_text = |name: &str| -> Option<String> { fs::read_to_string(dir.join(name)).ok() };
    let audio_raw = find_audio_file(&dir, "audio-raw");
    let audio_trimmed = find_audio_file(&dir, "audio-trimmed");
    Ok(SessionDetail {
        timestamp,
        meta: read_json("meta.json"),
        context: read_json("context.json"),
        history: read_json("history.json"),
        transcript: read_text("transcript.txt"),
        response: read_text("response.txt"),
        system_prompt: read_text("system-prompt.txt"),
        has_audio_raw: audio_raw.is_some(),
        has_audio_trimmed: audio_trimmed.is_some(),
        audio_raw_format: audio_raw.map(|a| a.format.to_string()),
        audio_trimmed_format: audio_trimmed.map(|a| a.format.to_string()),
    })
}

#[derive(Debug, Serialize)]
pub struct AudioBytes {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    pub filename: String,
}

/// 回 audio bytes — UI 用 blob URL 播放。
/// `which`: "raw" or "trimmed"
#[tauri::command]
pub fn recordings_audio_bytes(timestamp: String, which: String) -> Result<AudioBytes, String> {
    let stem = match which.as_str() {
        "raw" => "audio-raw",
        "trimmed" => "audio-trimmed",
        _ => return Err(format!("unknown audio variant: {which}(只接 raw/trimmed)")),
    };
    let dir = recordings_root().join(sanitize_timestamp(&timestamp)?);
    let audio = find_audio_file(&dir, stem)
        .ok_or_else(|| format!("audio file not found: {stem}.flac/.wav"))?;
    let bytes = fs::read(&audio.path)
        .map_err(|e| format!("read {}: {e}", audio.path.display()))?;
    Ok(AudioBytes {
        bytes,
        mime_type: audio.mime_type.to_string(),
        filename: audio.filename,
    })
}

struct AudioFile {
    path: PathBuf,
    filename: String,
    format: &'static str,
    mime_type: &'static str,
}

fn find_audio_file(dir: &Path, stem: &str) -> Option<AudioFile> {
    for (ext, format, mime_type) in [
        ("flac", "flac", "audio/flac"),
        ("wav", "wav", "audio/wav"),
    ] {
        let filename = format!("{stem}.{ext}");
        let path = dir.join(&filename);
        if path.exists() {
            return Some(AudioFile {
                path,
                filename,
                format,
                mime_type,
            });
        }
    }
    None
}

#[tauri::command]
pub fn recordings_delete_session(timestamp: String) -> Result<(), String> {
    let dir = recordings_root().join(sanitize_timestamp(&timestamp)?);
    if !dir.is_dir() {
        return Err(format!("not a session dir: {timestamp}"));
    }
    fs::remove_dir_all(&dir).map_err(|e| format!("rm -rf {}: {e}", dir.display()))
}

#[derive(Debug, Serialize)]
pub struct RecordingsStats {
    pub session_count: usize,
    pub total_bytes: u64,
    pub retention_days: u32,
    pub enabled: bool,
}

#[tauri::command]
pub fn recordings_stats() -> RecordingsStats {
    let cfg = read_config();
    let root = recordings_root();
    let (session_count, total_bytes) = fs::read_dir(&root)
        .ok()
        .map(|entries| {
            let mut count = 0usize;
            let mut bytes = 0u64;
            for e in entries.flatten() {
                if e.path().is_dir() {
                    count += 1;
                    bytes += dir_size(&e.path());
                }
            }
            (count, bytes)
        })
        .unwrap_or((0, 0));
    RecordingsStats {
        session_count,
        total_bytes,
        retention_days: cfg.retention_days,
        enabled: cfg.enabled,
    }
}

#[derive(Debug, Serialize)]
pub struct CleanupResult {
    pub removed: usize,
    pub kept: usize,
    pub retention_days: u32,
}

/// IPC — 立刻跑一次 cleanup,**不**受 `CLEANUP_RAN_THIS_SESSION` gate 限制。
/// 給 RecordingsTab 的「一鍵清舊」按鈕用。
/// `retention_days=0` 時不刪任何東西(同 auto cleanup 邏輯),只回 stats。
#[tauri::command]
pub fn recordings_cleanup_now() -> CleanupResult {
    let cfg = read_config();
    let root = recordings_root();
    let mut removed = 0usize;
    let mut kept = 0usize;
    if cfg.retention_days == 0 {
        // 不刪;直接 count 所有 dir 當 kept 回報
        if let Ok(entries) = fs::read_dir(&root) {
            for e in entries.flatten() {
                if e.path().is_dir() {
                    kept += 1;
                }
            }
        }
        return CleanupResult {
            removed,
            kept,
            retention_days: 0,
        };
    }
    let Ok(entries) = fs::read_dir(&root) else {
        return CleanupResult {
            removed,
            kept,
            retention_days: cfg.retention_days,
        };
    };
    let cutoff = SystemTime::now()
        - std::time::Duration::from_secs(cfg.retention_days as u64 * 24 * 3600);
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if modified >= cutoff {
            kept += 1;
            continue;
        }
        match fs::remove_dir_all(&path) {
            Ok(()) => removed += 1,
            Err(e) => tracing::warn!(
                error = %e,
                dir = %path.display(),
                "recordings: cleanup_now remove failed",
            ),
        }
    }
    tracing::info!(
        removed,
        kept,
        retention_days = cfg.retention_days,
        "recordings_cleanup_now",
    );
    CleanupResult {
        removed,
        kept,
        retention_days: cfg.retention_days,
    }
}

/// IPC — 寫 `recordings.retention_days` 進 `~/.mori/config.json`(設 0 = 不清)。
/// 寫完不自動跑 cleanup(讓 user 自己決定何時清),只更新 config。
#[tauri::command]
pub fn recordings_set_retention_days(days: u32) -> Result<(), String> {
    let path = mori_dir().join("config.json");
    // 讀原本的 config(找不到就空 object 起步)
    let mut json: serde_json::Value = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let rec = json
        .as_object_mut()
        .ok_or_else(|| "config.json 不是 object".to_string())?
        .entry("recordings".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let rec_obj = rec
        .as_object_mut()
        .ok_or_else(|| "config.json /recordings 不是 object".to_string())?;
    rec_obj.insert("retention_days".to_string(), serde_json::json!(days));
    fs::create_dir_all(mori_dir())
        .map_err(|e| format!("mkdir ~/.mori: {e}"))?;
    fs::write(
        &path,
        serde_json::to_string_pretty(&json).map_err(|e| format!("serialize: {e}"))?,
    )
    .map_err(|e| format!("write {}: {e}", path.display()))?;
    tracing::info!(days, "recordings_set_retention_days saved");
    Ok(())
}

/// 防 path traversal — timestamp 只能是 dir name format,不能含 `/` / `..` / abs path。
fn sanitize_timestamp(ts: &str) -> Result<String, String> {
    if ts.is_empty() || ts.contains('/') || ts.contains('\\') || ts.starts_with('.') || ts.contains("..") {
        return Err(format!("invalid timestamp: {ts}"));
    }
    Ok(ts.to_string())
}
