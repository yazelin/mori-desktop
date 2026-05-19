//! Speaker verification(聲紋辨識,Phase 3E)。
//!
//! Wake event 觸發 + user 講話完(VAD silence-stop 後),把 recording 音檔
//! 跟 enrolled user embedding 比對 cosine similarity:
//! - score >= threshold → 確認是 user 本人 → 走 agent
//! - score < threshold → 別人聲音,silent reject
//!
//! 預設 OFF — config `speaker_id.enabled=true` 才會 gate。沒 enrolled
//! (user 還沒錄聲紋)→ 直接 pass(避免 deadlock 鎖住所有人)。
//!
//! 兩個入口:
//! - [`enroll_user_voice`] — 一次性 enrollment,~30s 錄音抽聲紋存到 .npy
//! - [`verify_audio_file`] — 每次 wake 後驗 audio file 是不是 user
//!
//! 共用 wake-venv Python(`~/.mori/wake-venv/bin/python`),deps:resemblyzer。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::mori_dir;

const DEFAULT_THRESHOLD: f32 = 0.7;
const DEFAULT_ENROLL_SECONDS: f32 = 30.0;

// ── Bundled Python scripts(deploy 到 ~/.mori/bin/)──────────────────────────

const BUNDLED_ENROLL_SCRIPT: &[u8] =
    include_bytes!("../../../examples/scripts/mori-voice-enroll.py");
const BUNDLED_VERIFY_SCRIPT: &[u8] =
    include_bytes!("../../../examples/scripts/mori-voice-verify.py");

/// 開機呼叫:確保 enrollment + verify 兩個 Python script 在 user dir。
pub fn ensure_scripts_deployed(mori_dir: &Path) {
    let bin = mori_dir.join("bin");
    if let Err(e) = fs::create_dir_all(&bin) {
        tracing::warn!(error = %e, dir = %bin.display(), "speaker_id: mkdir bin failed");
        return;
    }
    for (name, bytes) in &[
        ("mori-voice-enroll.py", BUNDLED_ENROLL_SCRIPT),
        ("mori-voice-verify.py", BUNDLED_VERIFY_SCRIPT),
    ] {
        let path = bin.join(name);
        if path.exists() {
            continue;
        }
        if let Err(e) = fs::write(&path, *bytes) {
            tracing::warn!(error = %e, path = %path.display(), "speaker_id: write script failed");
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = fs::metadata(&path) {
                let mut perms = meta.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(&path, perms);
            }
        }
        tracing::info!(name, "speaker_id: deployed bundled script");
    }
}

// ── Config ──────────────────────────────────────────────────────────────────

fn default_python() -> PathBuf {
    let venv = mori_dir().join("wake-venv");
    if cfg!(target_os = "windows") {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    }
}

fn enrollment_path() -> PathBuf {
    mori_dir().join("voiceid").join("user_embedding.npy")
}

fn enroll_script_path() -> PathBuf {
    mori_dir().join("bin").join("mori-voice-enroll.py")
}

fn verify_script_path() -> PathBuf {
    mori_dir().join("bin").join("mori-voice-verify.py")
}

#[derive(Debug, Clone)]
struct SpeakerIdConfig {
    enabled: bool,
    threshold: f32,
    python: PathBuf,
}

fn read_config() -> SpeakerIdConfig {
    let default = SpeakerIdConfig {
        enabled: false,
        threshold: DEFAULT_THRESHOLD,
        python: default_python(),
    };
    let path = mori_dir().join("config.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return default;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return default;
    };
    let enabled = json
        .pointer("/speaker_id/enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let threshold = json
        .pointer("/speaker_id/threshold")
        .and_then(|v| v.as_f64())
        .map(|v| v.clamp(0.3, 0.99) as f32)
        .unwrap_or(DEFAULT_THRESHOLD);
    let python = json
        .pointer("/speaker_id/python")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(default_python);
    SpeakerIdConfig {
        enabled,
        threshold,
        python,
    }
}

// ── Verify(每次 wake 後跑)──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResult {
    pub score: f32,
    pub pass: bool,
    pub threshold: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerifyOutcome {
    /// 不在 enabled 狀態 → 不 gate,直接放行
    Disabled,
    /// User 還沒 enroll → 不 gate(避免鎖死),只 log 提示
    NotEnrolled,
    /// 驗證跑了 + 結果
    Verified(VerifyResult),
    /// Python 跑失敗 → log 但放行(寧可 false positive 也不要 user 用不了)
    Error(String),
}

/// 驗 audio file 是不是 enrolled user。
///
/// **Blocking** — 內部 spawn Python subprocess + wait。run from `spawn_blocking`
/// task,別在 async runtime 直接 call。
pub fn verify_audio_file(audio_path: &Path) -> VerifyOutcome {
    let cfg = read_config();
    if !cfg.enabled {
        return VerifyOutcome::Disabled;
    }
    let user_emb = enrollment_path();
    if !user_emb.exists() {
        tracing::info!(
            path = %user_emb.display(),
            "speaker_id enabled but user not enrolled — skipping gate",
        );
        return VerifyOutcome::NotEnrolled;
    }
    let script = verify_script_path();
    if !script.exists() {
        return VerifyOutcome::Error(format!(
            "verify script not found: {}",
            script.display()
        ));
    }
    if !cfg.python.exists() {
        return VerifyOutcome::Error(format!(
            "python not found: {}(跑 DepsTab → 聲紋辨識 runtime 一鍵裝)",
            cfg.python.display()
        ));
    }

    let output = match Command::new(&cfg.python)
        .arg(&script)
        .arg(&user_emb)
        .arg(audio_path)
        .arg("--threshold")
        .arg(cfg.threshold.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(o) => o,
        Err(e) => return VerifyOutcome::Error(format!("spawn python: {e}")),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return VerifyOutcome::Error(format!(
            "verify exited {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let last_line = stdout.lines().last().unwrap_or("").trim();
    match serde_json::from_str::<VerifyResult>(last_line) {
        Ok(r) => VerifyOutcome::Verified(r),
        Err(e) => VerifyOutcome::Error(format!("parse verify output: {e} — got: {last_line}")),
    }
}

// ── Enrollment(IPC command,UI 觸發)────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct EnrollmentStatus {
    pub enrolled: bool,
    pub path: String,
    pub size_bytes: u64,
    pub enabled: bool,
    pub threshold: f32,
}

/// IPC command — UI 開啟時 / 每次操作後查狀態。
#[tauri::command]
pub fn speaker_id_status() -> EnrollmentStatus {
    let cfg = read_config();
    let path = enrollment_path();
    let (enrolled, size_bytes) = match fs::metadata(&path) {
        Ok(m) => (true, m.len()),
        Err(_) => (false, 0),
    };
    EnrollmentStatus {
        enrolled,
        path: path.to_string_lossy().into_owned(),
        size_bytes,
        enabled: cfg.enabled,
        threshold: cfg.threshold,
    }
}

/// IPC command — UI 點「錄音註冊我的聲音」觸發。Blocking ~30s。
///
/// 跑 Python enrollment script,output 寫到 `~/.mori/voiceid/user_embedding.npy`。
///
/// **Phase 3 polish A2**:讀 Python stdout line-by-line,JSON event 用
/// Tauri `app.emit("speaker-id-enroll-progress", json)` forward 到前端,
/// modal 用真實進度(non setInterval 估算)。
#[tauri::command]
pub async fn speaker_id_enroll(
    app: tauri::AppHandle,
    seconds: Option<f32>,
) -> Result<EnrollmentStatus, String> {
    use std::io::{BufRead, BufReader};
    use tauri::Emitter;

    let cfg = read_config();
    let script = enroll_script_path();
    if !script.exists() {
        return Err(format!(
            "enroll script not found: {} — 重啟 mori-tauri 自動 deploy",
            script.display()
        ));
    }
    if !cfg.python.exists() {
        return Err(format!(
            "Python venv 沒裝(找不到 {}),先跑 DepsTab → 聲紋辨識 runtime",
            cfg.python.display()
        ));
    }
    let secs = seconds.unwrap_or(DEFAULT_ENROLL_SECONDS).clamp(5.0, 120.0);
    let out_path = enrollment_path();
    if let Some(parent) = out_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let python = cfg.python.clone();
    let app_for_thread = app.clone();
    let exit_status = tokio::task::spawn_blocking(move || -> Result<std::process::ExitStatus, String> {
        let mut child = Command::new(&python)
            .arg(&script)
            .arg(&out_path)
            .arg("--seconds")
            .arg(secs.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn python: {e}"))?;

        // Stream stdout — 每行解析 JSON,有 event 欄位就 emit 給前端
        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(line) = line else { continue };
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                    if value.get("event").is_some() {
                        let _ = app_for_thread.emit("speaker-id-enroll-progress", &value);
                    }
                } else {
                    tracing::debug!(line, "enroll: non-JSON stdout line");
                }
            }
        }

        // Stream done — wait for exit
        let status = child
            .wait()
            .map_err(|e| format!("wait python: {e}"))?;
        Ok(status)
    })
    .await
    .map_err(|e| format!("join: {e}"))?
    .map_err(|e| e)?;

    if !exit_status.success() {
        return Err(format!("enrollment exited with {exit_status}"));
    }

    Ok(speaker_id_status())
}

/// IPC command — 清掉 enrollment(刪 .npy),user 想重 enroll 前用。
#[tauri::command]
pub fn speaker_id_clear() -> Result<EnrollmentStatus, String> {
    let path = enrollment_path();
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("remove enrollment: {e}"))?;
    }
    Ok(speaker_id_status())
}
