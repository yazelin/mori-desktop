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
