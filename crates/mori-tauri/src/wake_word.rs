//! Wake-word listener — Hey Mori 待命的核心。
//!
//! ## 架構
//!
//! Mori-tauri spawn 一個 Python subprocess(`examples/scripts/mori-wake-listener.py`)
//! 用 openWakeWord 監聽麥克風。偵測到 wake phrase → stdout 印 JSON event。
//! Rust 端 background thread 讀 stdout、parse JSON、呼叫 callback 觸發
//! recording pipeline。
//!
//! ```text
//!   spawn python ──── stdin: <none>
//!                ──── stdout: line-delimited JSON events
//!                       {"event":"ready", "model":"..."}
//!                       {"event":"wake", "word":"hey_mori", "score":0.81}
//!                       {"event":"error", "msg":"..."}
//!                ──── stderr: log diagnostic(吞掉,不污染 stdout protocol)
//! ```
//!
//! ## 為什麼 shell-out 不是 in-process
//!
//! 跟 [`crate::wake_word`] 的 sibling [`whisper_local`](mori-core) 同樣理由:
//! - openWakeWord 是 Python lib,要綁 onnxruntime / tflite / sounddevice
//! - 在 Rust 端整 ONNX inference + ALSA stream 是大工程,純 Rust 替代品成熟度
//!   不夠(`vosk` 太重、`whisper.cpp` mini 不適合 hot-word use case)
//! - subprocess 隔離:crash 不會炸 mori-tauri,kill_on_drop 直接收回。
//!
//! ## Lifecycle
//!
//! `WakeWordListener::spawn(config, on_wake)` →
//!   1. spawn python(uv tool run / 系統 python 都試)
//!   2. background reader thread 讀 stdout、parse、把 wake event 給 on_wake 跑
//!   3. Drop → kill child + thread cooperatively 結束
//!
//! ## Phase 3A 範圍
//!
//! 只做「偵測 wake → callback」。Callback 端決定怎麼接(目前接到 start_recording
//! + 10s 後 stop_and_transcribe)。多輪 ask / confirm-before-act 在 Phase 3B+。

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::{anyhow, bail, Context as _, Result};
use serde::Deserialize;

/// Bundled hey-mori.onnx(TTS-only generic 訓練版,205 KB,對英文發音「Hey Mori」
/// 通用適用)。`ensure_default_model` 在 user dir 沒檔時寫一份過去當預設,
/// **不覆寫 user 自訓過的 model**(自訓對個人聲線命中率更高)。
const BUNDLED_HEY_MORI_ONNX: &[u8] = include_bytes!("../assets/wakeword/hey-mori.onnx");

/// 確保 `<mori_dir>/wakeword/hey-mori.onnx` 存在。沒檔 → 解壓 bundled。
/// 已存在 → 完全不動(user 可能訓過自己的)。
pub fn ensure_default_model(mori_dir: &Path) {
    let path = mori_dir.join("wakeword").join("hey-mori.onnx");
    if path.exists() {
        return;
    }
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            tracing::warn!(error = %e, dir = %parent.display(), "ensure_default_model: mkdir failed");
            return;
        }
    }
    match fs::write(&path, BUNDLED_HEY_MORI_ONNX) {
        Ok(()) => tracing::info!(
            path = %path.display(),
            size = BUNDLED_HEY_MORI_ONNX.len(),
            "installed bundled hey-mori.onnx (TTS-only generic)",
        ),
        Err(e) => tracing::warn!(
            error = %e,
            path = %path.display(),
            "ensure_default_model: write failed",
        ),
    }
}

/// 啟動 listener 的設定。從 `~/.mori/config.json` 的 `listening_mode` 區塊讀。
#[derive(Debug, Clone)]
pub struct WakeWordConfig {
    /// Python 解譯器路徑 — 預設 "python3"(用 PATH 解析)。
    /// 若 user 在 config 寫成 uv tool run 的環境路徑可直接覆蓋。
    pub python: PathBuf,
    /// `mori-wake-listener.py` 絕對路徑。預設 `~/.mori/bin/mori-wake-listener.py`。
    pub script_path: PathBuf,
    /// Wake-word model `.onnx` 檔。openWakeWord pre-trained 或 user 自訓。
    pub model_path: PathBuf,
    /// Detection threshold(0~1)— 越高越嚴格,越低越敏感(誤觸多)。
    /// 預設 0.5。
    pub threshold: f32,
    /// Custom verifier `.joblib`(可選)— 用 user 自己錄音 fine-tune 過的二階段 model。
    /// 有設且檔存在就用 base + verifier 兩階段判定,對個人聲線命中率高很多。
    /// None / 檔不存在 → 只跑 base model。
    pub verifier_path: Option<PathBuf>,
}

/// 從 Python stdout 解出來的 event。
#[derive(Debug, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum WakeEvent {
    /// Python 啟動好了,wake-word model 載完。後續才會發 wake / error。
    Ready { model: String },
    /// 偵測到 wake-word。score 是 model confidence,可給 log。
    Wake {
        word: String,
        score: f32,
    },
    /// Python 端錯誤(model 載不到 / mic 開不起來)。
    Error { msg: String },
}

/// Spawn 著的 listener。Drop 會 kill 子程序 + 等 reader thread 收尾。
pub struct WakeWordListener {
    child: Option<Child>,
    reader_thread: Option<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
}

impl WakeWordListener {
    /// 啟動 listener。`on_wake` 在 reader thread 上被呼叫(別在 callback 內做
    /// blocking 工作 — 至少 spawn 一個 tokio task 把活帶走)。
    ///
    /// 失敗回 Err — 通常是 python / script / model 路徑壞。
    pub fn spawn<F>(config: WakeWordConfig, on_wake: F) -> Result<Self>
    where
        F: Fn(WakeEvent) + Send + Sync + 'static,
    {
        // Defensive checks — Python 不存在的話 Command::spawn 才會失敗,
        // 但 script / model 路徑我們先驗,給更精準的錯誤訊息。
        if !config.script_path.exists() {
            bail!(
                "wake-word script not found: {}\n\
                 從 examples/scripts/mori-wake-listener.py 複製過去:\n\
                   cp examples/scripts/mori-wake-listener.py ~/.mori/bin/\n\
                   chmod +x ~/.mori/bin/mori-wake-listener.py",
                config.script_path.display()
            );
        }
        if !config.model_path.exists() {
            bail!(
                "wake-word model not found: {}\n\
                 從 openWakeWord pre-trained 抓一個放進去,或自訓 custom 「Hey Mori」\n\
                 模型(見 https://github.com/dscripka/openWakeWord)。",
                config.model_path.display()
            );
        }

        let mut cmd = Command::new(&config.python);
        cmd.arg(&config.script_path)
            .arg(&config.model_path)
            .arg(config.threshold.to_string());
        // 第 3 positional arg(可選):verifier `.joblib` 路徑。listener.py 看
        // sys.argv 長度判斷有沒有給。
        if let Some(v) = &config.verifier_path {
            if v.exists() {
                cmd.arg(v);
            } else {
                tracing::warn!(path = %v.display(), "verifier path configured but file missing — falling back to base-only");
            }
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()); // stderr 吞掉避免污染 stdout protocol

        let mut child = cmd.spawn().with_context(|| {
            format!(
                "spawn wake-word listener: python={} script={}\n\
                 確認:\n\
                 1. `python3 --version` 跑得起來\n\
                 2. `pip show openwakeword` 有裝(或 uv tool 對應環境)\n\
                 3. script 跟 model 路徑對\n",
                config.python.display(),
                config.script_path.display(),
            )
        })?;

        tracing::info!(
            pid = child.id(),
            model = %config.model_path.display(),
            threshold = config.threshold,
            "wake-word listener spawned",
        );

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("wake-word: child stdout missing"))?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_for_thread = shutdown.clone();
        let on_wake = Arc::new(on_wake);

        let reader_thread = std::thread::Builder::new()
            .name("mori-wake-reader".into())
            .spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    if shutdown_for_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    let line = match line {
                        Ok(l) => l,
                        Err(e) => {
                            tracing::warn!(?e, "wake-word: stdout read error");
                            break;
                        }
                    };
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<WakeEvent>(trimmed) {
                        Ok(ev) => {
                            tracing::debug!(?ev, "wake-word event");
                            on_wake(ev);
                        }
                        Err(e) => {
                            tracing::warn!(?e, line = %trimmed, "wake-word: bad JSON line");
                        }
                    }
                }
                tracing::debug!("wake-word reader thread exiting");
            })
            .context("spawn wake-word reader thread")?;

        Ok(Self {
            child: Some(child),
            reader_thread: Some(reader_thread),
            shutdown,
        })
    }
}

impl Drop for WakeWordListener {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            tracing::debug!(pid = child.id(), "wake-word listener killed on drop");
        }
        // reader thread 因為 child.stdout 被 kill 後 EOF 自然會結束。
        // join 等它,給 ~50ms 上限 — 不該等太久卡住 mode switch。
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
    }
}

// ─── Config readers ────────────────────────────────────────────────────

/// 從 `~/.mori/config.json` `listening_mode.*` 區塊讀 WakeWordConfig。
/// 預設值對齊 Phase 3A 設計 — phrase = Hey Mori, threshold 0.5。
pub fn config_from_disk(mori_dir: &Path) -> WakeWordConfig {
    let cfg_text = std::fs::read_to_string(mori_dir.join("config.json")).ok();
    let json: serde_json::Value = cfg_text
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);

    let python = json
        .pointer("/listening_mode/python")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("python3"));

    let script_path = json
        .pointer("/listening_mode/script_path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| mori_dir.join("bin").join("mori-wake-listener.py"));

    let model_path = json
        .pointer("/listening_mode/model_path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| mori_dir.join("wakeword").join("hey-mori.onnx"));

    let threshold = json
        .pointer("/listening_mode/threshold")
        .and_then(|v| v.as_f64())
        .map(|v| v.clamp(0.05, 0.95) as f32)
        .unwrap_or(0.5);

    let verifier_path = json
        .pointer("/listening_mode/verifier_path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from);

    WakeWordConfig {
        python,
        script_path,
        model_path,
        threshold,
        verifier_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_event_parses_ready() {
        let ev: WakeEvent =
            serde_json::from_str(r#"{"event":"ready","model":"/tmp/hey-mori.onnx"}"#).unwrap();
        assert!(matches!(ev, WakeEvent::Ready { .. }));
    }

    #[test]
    fn wake_event_parses_wake() {
        let ev: WakeEvent =
            serde_json::from_str(r#"{"event":"wake","word":"hey_mori","score":0.81}"#).unwrap();
        match ev {
            WakeEvent::Wake { word, score } => {
                assert_eq!(word, "hey_mori");
                assert!((score - 0.81).abs() < 1e-5);
            }
            other => panic!("expected Wake, got {other:?}"),
        }
    }

    #[test]
    fn wake_event_parses_error() {
        let ev: WakeEvent =
            serde_json::from_str(r#"{"event":"error","msg":"model load failed"}"#).unwrap();
        assert!(matches!(ev, WakeEvent::Error { .. }));
    }

    #[test]
    fn config_from_disk_defaults_when_no_file() {
        let tmp = std::env::temp_dir().join(format!(
            "mori-wake-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let cfg = config_from_disk(&tmp);
        assert_eq!(cfg.python, PathBuf::from("python3"));
        assert_eq!(cfg.threshold, 0.5);
        assert!(cfg.model_path.ends_with("wakeword/hey-mori.onnx"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn config_from_disk_reads_overrides() {
        let tmp = std::env::temp_dir().join(format!(
            "mori-wake-test-overrides-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let cfg_path = tmp.join("config.json");
        std::fs::write(
            &cfg_path,
            r#"{"listening_mode":{"python":"/usr/bin/python3.11","threshold":0.7,"model_path":"/x/custom.onnx"}}"#,
        )
        .unwrap();
        let cfg = config_from_disk(&tmp);
        assert_eq!(cfg.python, PathBuf::from("/usr/bin/python3.11"));
        assert!((cfg.threshold - 0.7).abs() < 1e-5);
        assert_eq!(cfg.model_path, PathBuf::from("/x/custom.onnx"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn config_threshold_clamps_out_of_range() {
        let tmp = std::env::temp_dir().join(format!(
            "mori-wake-test-clamp-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("config.json"),
            r#"{"listening_mode":{"threshold":5.0}}"#,
        )
        .unwrap();
        let cfg = config_from_disk(&tmp);
        assert!(cfg.threshold <= 0.95);
        std::fs::write(
            tmp.join("config.json"),
            r#"{"listening_mode":{"threshold":-1.0}}"#,
        )
        .unwrap();
        let cfg = config_from_disk(&tmp);
        assert!(cfg.threshold >= 0.05);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
