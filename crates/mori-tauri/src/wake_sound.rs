//! Wake-ack 音效播放 — Phase 3A.1.2。
//!
//! Wake-word 偵測到時,在 start_recording **之前**放一段 Mori 的應答音,user
//! 不用盯畫面就知道「Mori 在聽,可以下指令了」。
//!
//! ## 為什麼先播完再錄
//!
//! 麥克風跟喇叭同一房間,ack 從喇叭出來會被 mic 收進去污染 STT。沒做 echo
//! cancellation,簡單方案就是「先放完 ack(blocking)→ 才 start_recording」。
//!
//! ## 音檔模型(故意保持簡單)
//!
//! - 固定路徑 `~/.mori/wakeword/sounds/wake-ack.wav` ← **這個檔就是 wake 時播的音**
//! - 換音的方法 = **換這個檔**(cp、自己錄、什麼都行)
//! - 提供 5 個 bundled 備選 ── 開機時解壓到 `~/.mori/wakeword/sounds/wake-ack-alternates/`,
//!   user 自己 `cp alternates/<voice>.wav ../wake-ack.wav` 切換
//! - Config(`listening_mode.wake_ack_path`)只是讓 user 把固定路徑指到別處
//!   (例:自錄音檔在桌面,不想搬)
//!
//! 不做 preset name 選單 / 不做隨機輪播 / 不做 fallback 鏈,user 想要什麼自己擺。

use std::fs;
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};

use rodio::{Decoder, OutputStream, Sink, Source};

// ── Bundled 備選(編進 binary,首次開機解壓到 user dir 給 user 自己 cp)──────

const BUNDLED_ALTERNATES: &[(&str, &[u8])] = &[
    ("leda-嗯我在聽.wav", include_bytes!("../assets/wake-ack/leda-嗯我在聽.wav")),
    ("v5-erinome-嗯.wav", include_bytes!("../assets/wake-ack/v5-erinome-嗯.wav")),
    ("v6-嗯.wav", include_bytes!("../assets/wake-ack/v6-嗯.wav")),
    ("v8a-嗨.wav", include_bytes!("../assets/wake-ack/v8a-嗨.wav")),
    ("v9d-嗨.wav", include_bytes!("../assets/wake-ack/v9d-嗨.wav")),
];

/// 開機時(或 Listening mode 第一次進入時)呼叫:確保 user dir 有 ack 檔可用。
///
/// 1. `wake-ack-alternates/` 不存在 → 建立 + 解壓 5 個 bundled .wav 進去
///    (已存在的 skip,不覆寫 user 改動)
/// 2. `wake-ack.wav` 不存在 → 從 bundled leda 拷一份當預設
///
/// 失敗只 log warn,不阻斷啟動。
pub fn ensure_files(mori_dir: &Path) {
    let sounds_dir = mori_dir.join("wakeword").join("sounds");
    let alt_dir = sounds_dir.join("wake-ack-alternates");
    let active = sounds_dir.join("wake-ack.wav");

    if let Err(e) = fs::create_dir_all(&alt_dir) {
        tracing::warn!(error = %e, dir = %alt_dir.display(), "wake-ack: mkdir alternates failed");
        return;
    }

    for (name, bytes) in BUNDLED_ALTERNATES {
        let path = alt_dir.join(name);
        if path.exists() {
            continue;
        }
        if let Err(e) = fs::write(&path, *bytes) {
            tracing::warn!(error = %e, path = %path.display(), "wake-ack: write alternate failed");
        }
    }

    if !active.exists() {
        let default_bytes = BUNDLED_ALTERNATES
            .iter()
            .find(|(n, _)| *n == "leda-嗯我在聽.wav")
            .map(|(_, b)| *b);
        if let Some(bytes) = default_bytes {
            if let Err(e) = fs::write(&active, bytes) {
                tracing::warn!(error = %e, path = %active.display(), "wake-ack: write default failed");
            } else {
                tracing::info!(path = %active.display(), "wake-ack: installed default file");
            }
        }
    }
}

// ── 公開入口 ────────────────────────────────────────────────────────────────

/// wake event handler 在 start_recording 前呼叫。
///
/// **Blocking** — 設計就是要等播完才讓 caller 開錄音(避免 ack 被 mic 收回)。
///
/// 找不到檔 / 解碼失敗 / 沒喇叭都只 log warn,絕不 panic、絕不擋 recording 開始。
pub fn play_wake_ack(mori_dir: &Path) {
    if !is_enabled(mori_dir) {
        return;
    }
    let path = ack_path(mori_dir);
    if !path.exists() {
        tracing::warn!(path = %path.display(), "wake-ack: file not found, skipping");
        return;
    }
    match play_file(&path) {
        Ok(()) => tracing::info!(path = %path.display(), "wake-ack played"),
        Err(e) => tracing::warn!(error = %e, path = %path.display(), "wake-ack: play failed"),
    }
}

// ── Config / path 解析 ─────────────────────────────────────────────────────

/// 預設路徑:`<mori_dir>/wakeword/sounds/wake-ack.wav`。
/// Config `listening_mode.wake_ack_path`(絕對路徑)覆寫。
fn ack_path(mori_dir: &Path) -> PathBuf {
    let default = mori_dir.join("wakeword").join("sounds").join("wake-ack.wav");
    let config = mori_dir.join("config.json");
    let Ok(text) = fs::read_to_string(&config) else {
        return default;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return default;
    };
    json.pointer("/listening_mode/wake_ack_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or(default)
}

fn is_enabled(mori_dir: &Path) -> bool {
    let path = mori_dir.join("config.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return true;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return true;
    };
    json.pointer("/listening_mode/wake_ack_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

// ── rodio 播放 ──────────────────────────────────────────────────────────────

fn play_file(path: &Path) -> anyhow::Result<()> {
    let file = fs::File::open(path)?;
    let source = Decoder::new(BufReader::new(file))?;
    play_blocking(source.convert_samples());
    Ok(())
}

fn play_blocking(source: impl Source<Item = i16> + Send + 'static) {
    // 每次新開 OutputStream — rodio OutputStream 不 Send,不能存進 AppState 共用。
    // 開喇叭 stream cost 微小(<10ms),wake event 不是高頻路徑。
    let (_stream, handle) = match OutputStream::try_default() {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "wake-ack: no audio output device");
            return;
        }
    };
    let sink = match Sink::try_new(&handle) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "wake-ack: sink create failed");
            return;
        }
    };
    sink.append(source);
    sink.sleep_until_end();
}

#[allow(dead_code)]
fn read_cursor(bytes: &'static [u8]) -> BufReader<Cursor<&'static [u8]>> {
    BufReader::new(Cursor::new(bytes))
}

// ── Tauri IPC commands(給 ConfigTab UI)─────────────────────────────────────
//
// Settings page 用這些 command 列備選、切換、試聽、上傳自錄音檔。

use serde::Serialize;

#[derive(Serialize)]
pub struct AlternateEntry {
    pub filename: String,
    pub size_bytes: u64,
    /// 是否就是當前 active(用 config 記錄的 selected alternate filename 比對)。
    pub is_active: bool,
    /// 是否 bundled(內建,UI 不給刪)。
    pub is_bundled: bool,
}

#[derive(Serialize)]
pub struct WakeAckStatus {
    pub enabled: bool,
    /// 當前 active alternate filename(從 config `listening_mode.wake_ack_active` 讀)。
    /// `null` 表示 user 直接編輯 wake-ack.wav 或自訂 wake_ack_path,UI 顯示「自訂」。
    pub active_filename: Option<String>,
    /// Config override path(`listening_mode.wake_ack_path`)。
    pub custom_path: Option<String>,
    pub alternates: Vec<AlternateEntry>,
}

fn alternates_dir(mori_dir: &Path) -> PathBuf {
    mori_dir.join("wakeword").join("sounds").join("wake-ack-alternates")
}

fn is_bundled(name: &str) -> bool {
    BUNDLED_ALTERNATES.iter().any(|(n, _)| *n == name)
}

#[tauri::command]
pub fn wake_ack_status() -> Result<WakeAckStatus, String> {
    let mori = crate::mori_dir();
    let alt_dir = alternates_dir(&mori);
    let mut alternates: Vec<AlternateEntry> = Vec::new();
    if let Ok(entries) = fs::read_dir(&alt_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".wav") {
                continue;
            }
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            alternates.push(AlternateEntry {
                filename: name.to_string(),
                size_bytes: size,
                is_active: false,
                is_bundled: is_bundled(name),
            });
        }
    }
    alternates.sort_by(|a, b| a.filename.cmp(&b.filename));

    let cfg_path = mori.join("config.json");
    let cfg_json: serde_json::Value = fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    let active_filename = cfg_json
        .pointer("/listening_mode/wake_ack_active")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if let Some(active) = &active_filename {
        for a in alternates.iter_mut() {
            if &a.filename == active {
                a.is_active = true;
            }
        }
    }

    let custom_path = cfg_json
        .pointer("/listening_mode/wake_ack_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let enabled = cfg_json
        .pointer("/listening_mode/wake_ack_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    Ok(WakeAckStatus {
        enabled,
        active_filename,
        custom_path,
        alternates,
    })
}

/// 從 alternates/<filename> 拷到 wake-ack.wav,同時 update config 記住選了哪個。
#[tauri::command]
pub fn wake_ack_set_active(filename: String) -> Result<(), String> {
    let mori = crate::mori_dir();
    let src = alternates_dir(&mori).join(&filename);
    if !src.exists() {
        return Err(format!("alternate not found: {filename}"));
    }
    if !filename.ends_with(".wav") {
        return Err("filename must end with .wav".into());
    }
    let dst = mori.join("wakeword").join("sounds").join("wake-ack.wav");
    fs::copy(&src, &dst).map_err(|e| format!("copy failed: {e}"))?;
    update_config(&mori, |cfg| {
        let lm = cfg
            .pointer_mut("/listening_mode")
            .cloned()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        let mut lm = serde_json::Value::Object(lm);
        lm["wake_ack_active"] = serde_json::Value::String(filename.clone());
        cfg["listening_mode"] = lm;
    })?;
    Ok(())
}

#[tauri::command]
pub fn wake_ack_set_enabled(enabled: bool) -> Result<(), String> {
    let mori = crate::mori_dir();
    update_config(&mori, |cfg| {
        let lm = cfg
            .pointer_mut("/listening_mode")
            .cloned()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        let mut lm = serde_json::Value::Object(lm);
        lm["wake_ack_enabled"] = serde_json::Value::Bool(enabled);
        cfg["listening_mode"] = lm;
    })?;
    Ok(())
}

/// 試聽某個 alternate(或當前 active 檔,filename=None)。spawn_blocking 跑 rodio。
#[tauri::command]
pub async fn wake_ack_preview(filename: Option<String>) -> Result<(), String> {
    let mori = crate::mori_dir();
    let path = if let Some(name) = filename {
        if !name.ends_with(".wav") {
            return Err("filename must end with .wav".into());
        }
        alternates_dir(&mori).join(&name)
    } else {
        ack_path(&mori)
    };
    if !path.exists() {
        return Err(format!("file not found: {}", path.display()));
    }
    tokio::task::spawn_blocking(move || play_file(&path))
        .await
        .map_err(|e| format!("join: {e}"))?
        .map_err(|e| format!("play: {e}"))?;
    Ok(())
}

/// 把 user 上傳的 .wav 寫進 alternates/。bytes 是 raw wav binary。
/// 防呆:檢 RIFF header,擋 bundled 同名覆寫(避免 user 不小心蓋掉內建)。
#[tauri::command]
pub fn wake_ack_upload(filename: String, bytes: Vec<u8>) -> Result<(), String> {
    if !filename.ends_with(".wav") {
        return Err("filename must end with .wav".into());
    }
    if filename.contains('/') || filename.contains('\\') || filename.starts_with('.') {
        return Err("invalid filename".into());
    }
    if is_bundled(&filename) {
        return Err(format!("'{filename}' 是內建檔名,請換個名字"));
    }
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("不是有效的 WAV 檔(header 不對)".into());
    }
    let mori = crate::mori_dir();
    let dir = alternates_dir(&mori);
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir: {e}"))?;
    let path = dir.join(&filename);
    fs::write(&path, &bytes).map_err(|e| format!("write: {e}"))?;
    Ok(())
}

#[tauri::command]
pub fn wake_ack_delete_alternate(filename: String) -> Result<(), String> {
    if !filename.ends_with(".wav") {
        return Err("filename must end with .wav".into());
    }
    if is_bundled(&filename) {
        return Err(format!("'{filename}' 是內建檔不能刪"));
    }
    let mori = crate::mori_dir();
    let path = alternates_dir(&mori).join(&filename);
    if !path.exists() {
        return Err(format!("not found: {filename}"));
    }
    fs::remove_file(&path).map_err(|e| format!("delete: {e}"))?;
    Ok(())
}

/// 寫 config.json — read + mutate via closure + write。
fn update_config<F>(mori_dir: &Path, mutate: F) -> Result<(), String>
where
    F: FnOnce(&mut serde_json::Value),
{
    let path = mori_dir.join("config.json");
    let mut cfg: serde_json::Value = fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    mutate(&mut cfg);
    let serialized =
        serde_json::to_string_pretty(&cfg).map_err(|e| format!("serialize: {e}"))?;
    fs::write(&path, serialized).map_err(|e| format!("write: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_alternates_have_content() {
        for (name, bytes) in BUNDLED_ALTERNATES {
            assert!(!bytes.is_empty(), "bundled wav {name} is empty");
            // WAV header sanity
            assert_eq!(&bytes[..4], b"RIFF", "bundled wav {name} not RIFF");
            assert_eq!(&bytes[8..12], b"WAVE", "bundled wav {name} not WAVE");
        }
    }

    #[test]
    fn default_path_uses_mori_dir() {
        let tmp = std::env::temp_dir();
        let path = ack_path(&tmp);
        assert!(path.ends_with("wake-ack.wav"));
        assert!(path.to_string_lossy().contains("wakeword"));
    }
}
