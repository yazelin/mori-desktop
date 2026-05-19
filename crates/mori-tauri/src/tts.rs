//! TTS speak-back — Mori 講話(Phase 3D)。
//!
//! Agent 回應完成後(若 `tts.enabled=true`),呼叫 Python edge-tts bridge
//! (`examples/scripts/mori-tts-edge.py`)合成 MP3 → rodio 播放。預設用免費
//! Microsoft Edge TTS endpoint(無 quota / 無 API key,zh-TW native voice)。
//!
//! ## 為什麼用 edge-tts 不用 Gemini
//!
//! Gemini TTS free tier 100 req/day,聊個幾句就破。edge-tts 借 MS Edge
//! browser endpoint(非官方支援但廣用),實質無限額 + native zh-TW + 免費。
//!
//! ## Lifecycle
//!
//! `speak_async(text, app)` 立刻 return,實際合成 + 播放在 background:
//!   1. tokio::spawn → 寫 stdin text 給 Python subprocess
//!   2. subprocess 把 MP3 寫到 `/tmp/mori-tts-<uuid>.mp3`
//!   3. rodio 讀那 MP3 + 開新 OutputStream + sleep_until_end blocking 跑完
//!   4. 結束後 unlink temp file
//!
//! ## 中斷
//!
//! `speak_async` 把 `Sink` 包 `Arc` 存進 `AppState::tts_sink` slot,Ctrl+Alt+Esc
//! 全域 abort handler 在 Phase 跟 recording / pipeline 都沒事時,take +
//! `sink.stop()` 中斷正在播的 TTS。Sink 內部是 `Arc<Mutex<…>>`,Send+Sync 安全。

use std::fs;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::Mutex;
use rodio::{Decoder, OutputStream, Sink, Source};
use tauri::AppHandle;

use crate::mori_dir;

/// 共享 `Sink` slot 的型別 alias — `AppState::tts_sink` 用。
pub type TtsSinkSlot = Arc<Mutex<Option<Arc<Sink>>>>;

/// Bundled mori-tts-edge.py(Phase 3D)— Python edge-tts bridge script。
/// `ensure_script_deployed` 在 user dir 沒檔時寫一份過去當預設,user 可覆寫。
const BUNDLED_TTS_SCRIPT: &[u8] = include_bytes!("../../../examples/scripts/mori-tts-edge.py");

/// 確保 `~/.mori/bin/mori-tts-edge.py` 存在。沒檔 → 寫 bundled。
/// User 改過或自己版本 → 不覆寫(`!path.exists()` gate)。
///
/// Linux/macOS 順手 chmod +x。
pub fn ensure_script_deployed(mori_dir: &Path) {
    let bin = mori_dir.join("bin");
    if let Err(e) = fs::create_dir_all(&bin) {
        tracing::warn!(error = %e, dir = %bin.display(), "tts ensure_script: mkdir failed");
        return;
    }
    let path = bin.join("mori-tts-edge.py");
    if path.exists() {
        return;
    }
    if let Err(e) = fs::write(&path, BUNDLED_TTS_SCRIPT) {
        tracing::warn!(error = %e, path = %path.display(), "tts ensure_script: write failed");
        return;
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
    tracing::info!(path = %path.display(), "tts: deployed bundled mori-tts-edge.py");
}

/// Default Python(對齊 wake-listener 用同 venv)。
fn default_python() -> PathBuf {
    let venv = mori_dir().join("wake-venv");
    if cfg!(target_os = "windows") {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    }
}

/// 預設 voice — Mori 形象偏年輕女精靈,HsiaoYu 比 HsiaoChen 偏年輕清亮。
const DEFAULT_VOICE: &str = "zh-TW-HsiaoChenNeural";

/// 默認 TTS bridge script(deploy 到 ~/.mori/bin/mori-tts-edge.py 或 repo
/// examples/scripts/ 直接跑都可)。
fn default_script_path() -> PathBuf {
    let mori_bin = mori_dir().join("bin").join("mori-tts-edge.py");
    if mori_bin.exists() {
        return mori_bin;
    }
    // dev fallback:repo examples
    PathBuf::from("examples/scripts/mori-tts-edge.py")
}

struct TtsConfig {
    enabled: bool,
    python: PathBuf,
    script_path: PathBuf,
    voice: String,
}

fn read_config() -> TtsConfig {
    let default = TtsConfig {
        enabled: false, // 預設 OFF — user 主動 enable 才會講話
        python: default_python(),
        script_path: default_script_path(),
        voice: DEFAULT_VOICE.to_string(),
    };
    let path = mori_dir().join("config.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return default;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return default;
    };
    let enabled = json
        .pointer("/tts/enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let python = json
        .pointer("/tts/python")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(default_python);
    let script_path = json
        .pointer("/tts/script_path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(default_script_path);
    let voice = json
        .pointer("/tts/voice")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| DEFAULT_VOICE.to_string());
    TtsConfig {
        enabled,
        python,
        script_path,
        voice,
    }
}

/// 公開入口 — agent response 完成時呼叫。立刻 return,實際合成 + 播放在
/// tokio task 內背景跑。失敗只 log warn,不影響 Phase::Done UI 更新。
///
/// `sink_slot` — 共享的 `AppState::tts_sink`。synth_and_play 會把當前 Sink 塞
/// 進去,Ctrl+Alt+Esc abort handler 拿到後就能 stop。
pub fn speak_async(text: String, _app: AppHandle, sink_slot: TtsSinkSlot) {
    let cfg = read_config();
    if !cfg.enabled {
        return;
    }
    if text.trim().is_empty() {
        return;
    }
    if !cfg.python.exists() {
        tracing::warn!(
            python = %cfg.python.display(),
            "tts.speak_async: python not found (跑 DepsTab → TTS runtime 安裝 wake-venv + edge-tts)",
        );
        return;
    }
    if !cfg.script_path.exists() {
        tracing::warn!(
            script = %cfg.script_path.display(),
            "tts.speak_async: bridge script not found",
        );
        return;
    }

    tokio::task::spawn_blocking(move || {
        if let Err(e) = synth_and_play(&cfg, &text, &sink_slot) {
            tracing::warn!(error = %e, "tts: synth_and_play failed");
        }
        // 跑完(或 stop 後 sleep_until_end 返回)把 slot 清空,給下一輪 TTS 用
        *sink_slot.lock() = None;
    });
}

fn synth_and_play(cfg: &TtsConfig, text: &str, sink_slot: &TtsSinkSlot) -> anyhow::Result<()> {
    // 用 timestamp 避免並發 collision。/tmp 上 OS 開機重啟自動清。
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let out_mp3 = std::env::temp_dir().join(format!("mori-tts-{ts}.mp3"));

    tracing::info!(
        text_len = text.len(),
        voice = cfg.voice,
        out = %out_mp3.display(),
        "tts: synthesizing",
    );

    // 1. Spawn Python subprocess,stdin 餵 text
    let mut child = Command::new(&cfg.python)
        .arg(&cfg.script_path)
        .arg(&out_mp3)
        .arg(&cfg.voice)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("spawn python: {e}"))?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| anyhow::anyhow!("write stdin: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| anyhow::anyhow!("wait subprocess: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = fs::remove_file(&out_mp3);
        return Err(anyhow::anyhow!(
            "edge-tts exited {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    if !out_mp3.exists() || fs::metadata(&out_mp3).map(|m| m.len()).unwrap_or(0) == 0 {
        return Err(anyhow::anyhow!("MP3 output 沒寫成功或空檔"));
    }

    // 2. rodio play MP3
    let file = fs::File::open(&out_mp3).map_err(|e| anyhow::anyhow!("open mp3: {e}"))?;
    let source =
        Decoder::new(BufReader::new(file)).map_err(|e| anyhow::anyhow!("decode mp3: {e}"))?;

    let (_stream, handle) =
        OutputStream::try_default().map_err(|e| anyhow::anyhow!("no audio output: {e}"))?;
    let sink = Arc::new(Sink::try_new(&handle).map_err(|e| anyhow::anyhow!("sink create: {e}"))?);
    sink.append(source.convert_samples::<i16>());
    // 把 sink 放進共享 slot,abort handler 可以 stop。先放再 sleep,確保
    // 即使 sleep 一開始就被 stop(極短音檔)也能正確返回。
    *sink_slot.lock() = Some(sink.clone());
    sink.sleep_until_end();
    // sleep 返回後不管是自然結束或 stop() 都把 slot 拿掉(speak_async 那邊也會
    // 兜底清,雙保險)。
    *sink_slot.lock() = None;

    // 3. cleanup
    let _ = fs::remove_file(&out_mp3);
    tracing::info!("tts: playback done");
    Ok(())
}

/// IPC command — 試聽 voice。給 ConfigTab UI 用。
///
/// Preview 走的 sink 也塞共享 slot,所以 Ctrl+Alt+Esc 也能中斷預覽。
#[tauri::command]
pub async fn tts_preview(
    state: tauri::State<'_, std::sync::Arc<crate::AppState>>,
    text: Option<String>,
    voice: Option<String>,
) -> Result<(), String> {
    let mut cfg = read_config();
    if let Some(v) = voice {
        cfg.voice = v;
    }
    let sample_text = text.unwrap_or_else(|| {
        "嗨,我是 Mori。試聽聲音這樣 OK 嗎?".to_string()
    });
    if !cfg.python.exists() {
        return Err(format!(
            "Python venv 沒裝(找不到 {}),先跑 DepsTab → TTS runtime",
            cfg.python.display()
        ));
    }
    if !cfg.script_path.exists() {
        return Err(format!(
            "TTS bridge script 沒部署({}),跑 examples/scripts/mori-tts-edge.py 應該要在 ~/.mori/bin/ 或 repo 內",
            cfg.script_path.display()
        ));
    }
    let sink_slot = state.tts_sink.clone();
    tokio::task::spawn_blocking(move || synth_and_play(&cfg, &sample_text, &sink_slot))
        .await
        .map_err(|e| format!("join: {e}"))?
        .map_err(|e| format!("synth/play: {e}"))?;
    Ok(())
}

/// IPC command — 立刻中斷正在播的 TTS(若有)。回 `true` 表示有 sink 被 stop。
/// Ctrl+Alt+Esc abort handler 跟未來 UI 「停止講話」按鈕都呼這個。
#[tauri::command]
pub fn tts_stop(state: tauri::State<'_, std::sync::Arc<crate::AppState>>) -> bool {
    let taken = state.tts_sink.lock().take();
    match taken {
        Some(sink) => {
            sink.stop();
            tracing::info!("tts: sink stopped via tts_stop");
            true
        }
        None => false,
    }
}
