//! Per-pipeline recordings archive(Phase B — 之前一直留 backlog 的)。
//!
//! 每次完整 voice pipeline run(wake → record → STT → speaker_id → evaluator
//! → agent → response)存一個 session 資料夾到 `~/.mori/recordings/<timestamp>/`,
//! 包含完整 I/O:
//!
//! ```text
//! ~/.mori/recordings/2026-05-19T17-12-34/
//! ├── audio-raw.wav         錄音原檔(silence-trim 前)
//! ├── audio-trimmed.wav     送 STT 的版本(silence-trim 後)
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::mori_dir;

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

        // Audio
        if let Some(bytes) = self.audio_raw_bytes {
            write_file(&dir.join("audio-raw.wav"), &bytes, "audio-raw.wav");
        }
        if let Some(bytes) = self.audio_trimmed_bytes {
            write_file(&dir.join("audio-trimmed.wav"), &bytes, "audio-trimmed.wav");
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
    Ok(SessionDetail {
        timestamp,
        meta: read_json("meta.json"),
        context: read_json("context.json"),
        history: read_json("history.json"),
        transcript: read_text("transcript.txt"),
        response: read_text("response.txt"),
        system_prompt: read_text("system-prompt.txt"),
        has_audio_raw: dir.join("audio-raw.wav").exists(),
        has_audio_trimmed: dir.join("audio-trimmed.wav").exists(),
    })
}

/// 回 audio bytes — UI 用 blob URL 播放。
/// `which`: "raw" or "trimmed"
#[tauri::command]
pub fn recordings_audio_bytes(timestamp: String, which: String) -> Result<Vec<u8>, String> {
    let file = match which.as_str() {
        "raw" => "audio-raw.wav",
        "trimmed" => "audio-trimmed.wav",
        _ => return Err(format!("unknown audio variant: {which}(只接 raw/trimmed)")),
    };
    let dir = recordings_root().join(sanitize_timestamp(&timestamp)?);
    let path = dir.join(file);
    fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))
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

/// 防 path traversal — timestamp 只能是 dir name format,不能含 `/` / `..` / abs path。
fn sanitize_timestamp(ts: &str) -> Result<String, String> {
    if ts.is_empty() || ts.contains('/') || ts.contains('\\') || ts.starts_with('.') || ts.contains("..") {
        return Err(format!("invalid timestamp: {ts}"));
    }
    Ok(ts.to_string())
}
