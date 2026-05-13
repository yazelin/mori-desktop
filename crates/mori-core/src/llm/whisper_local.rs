//! 本機 Whisper STT — shell-out 到 whisper.cpp 官方 `whisper-server` HTTP 子程序。
//!
//! ## 為什麼是 shell-out 不是 in-process FFI
//!
//! Phase 5C 第一版用 `whisper-rs`(Rust binding 包 whisper.cpp C++ source),
//! cargo build 時自己 cmake + bindgen 編進 mori 執行檔。優點是延遲低,缺點:
//!
//! 1. **跨平台脆**:`whisper-rs-sys` 0.13 在 Windows MSVC bindgen 算錯
//!    `whisper_full_params` struct size,Windows build 直接斷。
//! 2. **build 依賴重**:Linux 也要 `cmake` + `libclang-dev`,新 contributor
//!    要先裝一輪 toolchain。
//! 3. **不能用 GPU 加速版本**:user 想跑 CUDA / Metal / Vulkan whisper.cpp
//!    要 fork Mori 改 dep,沒人做。
//!
//! shell-out 全解:user 自己下載 whisper.cpp release binary(每平台 / 每 GPU
//! 變體都有 pre-built),放 `~/.mori/bin/whisper-server[.exe]` 或寫 config
//! 絕對路徑。Mori spawn 它、HTTP POST WAV、收 JSON 文字。
//!
//! Trade-off:Mori 啟動時 lazy spawn(第一次按熱鍵才起),~500ms warm-up
//! cost 放在第一次錄音前。之後常駐,跟 in-process 同速。
//!
//! ## 模型檔
//!
//! 走 ggml `.bin` 格式,從 huggingface 抓:
//!   <https://huggingface.co/ggerganov/whisper.cpp/tree/main>
//!
//! 中文場景建議 small.bin(466MB);CPU 慢可以用 base(142MB)。
//!
//! ## 預設 binary 來源
//!
//! whisper.cpp Release:<https://github.com/ggml-org/whisper.cpp/releases>
//! 解壓後把 `whisper-server`(Linux)或 `whisper-server.exe`(Windows)
//! 放到 `~/.mori/bin/` 即可。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context as _, Result};
use async_trait::async_trait;
use tokio::sync::Mutex;

use super::transcribe::TranscriptionProvider;

const HEALTHCHECK_TIMEOUT_SECS: u64 = 30;
const INFERENCE_TIMEOUT_SECS: u64 = 120;

/// 公開 API:跟 v1(in-process)同名同方法,呼叫端不用改。
pub struct LocalWhisperProvider {
    /// subprocess + HTTP 細節抽到內部物件。Arc 讓 transcribe() 不必 take
    /// ownership;同時 Drop 在最後一份 Arc 釋放時觸發,清理子程序。
    server: Arc<WhisperServer>,
}

impl LocalWhisperProvider {
    pub const NAME: &'static str = "whisper-local";

    /// 從 `~/.mori/config.json` 蓋出 provider。
    pub fn from_config() -> Result<Self> {
        let cfg = mori_config_path();
        let model_path = cfg
            .as_deref()
            .and_then(|p| super::groq::read_json_pointer(p, "/providers/whisper-local/model_path"))
            .map(PathBuf::from)
            .unwrap_or_else(default_model_path);
        let language = cfg
            .as_deref()
            .and_then(|p| super::groq::read_json_pointer(p, "/providers/whisper-local/language"));
        let server_binary = cfg
            .as_deref()
            .and_then(|p| super::groq::read_json_pointer(p, "/providers/whisper-local/server_binary"))
            .map(PathBuf::from)
            .unwrap_or_else(default_server_binary);

        Self::new(&model_path, language, &server_binary)
    }

    pub fn new(model_path: &Path, language: Option<String>, server_binary: &Path) -> Result<Self> {
        if !model_path.exists() {
            bail!(
                "whisper model not found at {}\n\nDownload one from \
                 https://huggingface.co/ggerganov/whisper.cpp/tree/main and put it there.\n\
                 Recommended for Chinese: ggml-small.bin (466MB).",
                model_path.display(),
            );
        }
        // 不檢查 server_binary 存在性 — 它可能是 "whisper-server" 純 binary 名,
        // 透過 OS PATH 解析,Command::new() 才能驗。錯誤訊息留到 ensure_started()
        // 第一次 spawn 失敗時,給更精準的 hint。

        let server = WhisperServer {
            binary: server_binary.to_path_buf(),
            model_path: model_path.to_path_buf(),
            language,
            state: Mutex::new(None),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(INFERENCE_TIMEOUT_SECS))
                .build()
                .context("build reqwest client for whisper-server")?,
        };
        Ok(Self {
            server: Arc::new(server),
        })
    }

    pub fn model_path(&self) -> &Path {
        &self.server.model_path
    }

    pub fn language(&self) -> Option<&str> {
        self.server.language.as_deref()
    }
}

#[async_trait]
impl TranscriptionProvider for LocalWhisperProvider {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    async fn transcribe(&self, audio: Vec<u8>) -> Result<String> {
        self.server.transcribe(audio).await
    }
}

// ─── 內部:subprocess + HTTP ────────────────────────────────────────────

struct WhisperServer {
    binary: PathBuf,
    model_path: PathBuf,
    /// `Some("zh")` / `Some("auto")` / `None`(送 inference 時走 server 預設,
    /// 通常是 auto detect)。
    language: Option<String>,
    /// 序列化「啟動」這件事:check + spawn + health-check 整段在 tokio mutex
    /// 裡。鎖住期間最多 ~500ms(warm-up),之後每次 transcribe lock 拿到就
    /// 直接看到 Some(alive) 立刻釋放。
    state: Mutex<Option<RunningServer>>,
    http: reqwest::Client,
}

struct RunningServer {
    child: Child,
    port: u16,
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        // 最佳努力 — kill 是非阻塞,送 SIGKILL(Linux)/ TerminateProcess(Win)。
        // 不 wait(可能跑在 tokio thread,blocking wait 會卡 runtime)。OS reap。
        let _ = self.child.kill();
        tracing::debug!(port = self.port, pid = self.child.id(), "whisper-server killed on drop");
    }
}

impl WhisperServer {
    /// 確保子程序在跑;不在就 spawn 然後 health-check 等它 ready。
    /// 回傳目前綁的 port。
    async fn ensure_started(&self) -> Result<u16> {
        let mut state = self.state.lock().await;

        // Already-running fast path
        if let Some(running) = state.as_mut() {
            match running.child.try_wait() {
                Ok(None) => return Ok(running.port), // alive
                Ok(Some(status)) => {
                    tracing::warn!(?status, "whisper-server exited; respawning");
                    *state = None;
                }
                Err(e) => {
                    tracing::warn!(?e, "whisper-server try_wait error; respawning");
                    *state = None;
                }
            }
        }

        let port = pick_free_port().context("pick free localhost port for whisper-server")?;
        let lang = self.language.as_deref().unwrap_or("auto");
        let model_str = self
            .model_path
            .to_str()
            .ok_or_else(|| anyhow!("model path not valid UTF-8: {}", self.model_path.display()))?;

        let mut cmd = Command::new(&self.binary);
        cmd.args([
            "--model",
            model_str,
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--inference-path",
            "/inference",
            "--language",
            lang,
        ]);
        cmd.stdout(Stdio::null()).stderr(Stdio::null());

        let child = cmd.spawn().with_context(|| {
            format!(
                "spawn whisper-server from {} — 確認檔案存在 + 可執行。\n\
                 下載: https://github.com/ggml-org/whisper.cpp/releases\n\
                 解壓後放到 ~/.mori/bin/whisper-server{} 或在 config 寫絕對路徑\n\
                 (~/.mori/config.json → providers.whisper-local.server_binary)。",
                self.binary.display(),
                if cfg!(windows) { ".exe" } else { "" },
            )
        })?;

        tracing::info!(
            binary = %self.binary.display(),
            model = %self.model_path.display(),
            port,
            language = lang,
            "spawned whisper-server",
        );

        let running = RunningServer { child, port };
        *state = Some(running);

        // 釋放 lock 之前先 health-check;成功才放 port 回去。失敗就把 state 清掉,
        // 下次 ensure_started 再試一次。
        let healthcheck = self.wait_for_ready(port).await;
        if let Err(e) = healthcheck {
            *state = None; // RunningServer Drop kills the child
            return Err(e);
        }

        Ok(port)
    }

    async fn wait_for_ready(&self, port: u16) -> Result<()> {
        // whisper-server 載入模型期間 HTTP 連得上但 /inference 還沒 ready;
        // 我們 poll 根 URL,有任何 200/404 都算 "已 listen"。GET / 在
        // whisper.cpp server 是 index.html(load 完返 200)— 200 = 真 ready。
        let deadline = std::time::Instant::now() + Duration::from_secs(HEALTHCHECK_TIMEOUT_SECS);
        let url = format!("http://127.0.0.1:{port}/");
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            match self
                .http
                .get(&url)
                .timeout(Duration::from_millis(500))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    tracing::debug!(port, attempts, "whisper-server ready");
                    return Ok(());
                }
                _ => {
                    if std::time::Instant::now() >= deadline {
                        bail!(
                            "whisper-server did not become ready within {}s — \
                             模型載入過慢或 binary 異常退出。檢查 ~/.mori/models/ 模型檔",
                            HEALTHCHECK_TIMEOUT_SECS,
                        );
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    }

    async fn transcribe(&self, audio: Vec<u8>) -> Result<String> {
        let port = self.ensure_started().await?;
        let url = format!("http://127.0.0.1:{port}/inference");

        tracing::debug!(bytes = audio.len(), port, "whisper-server inference request");

        let part = reqwest::multipart::Part::bytes(audio)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .context("build multipart audio part")?;
        let mut form = reqwest::multipart::Form::new().part("file", part);
        // 加 initial prompt 鎮住 caption hallucination(跟 v1 in-process 路徑同字串)。
        form = form.text(
            "prompt",
            "以下是使用者直接對 AI 助手 Mori 說的話,繁體中文。\
             常見用語:程式、軟體、檔案、影片、電腦、滑鼠、伺服器、資料庫、\
             記住、提醒、行事曆、會議。",
        );
        // response_format 預設 json,顯式宣告省得未來 server 改預設。
        form = form.text("response_format", "json");
        // language 經由啟動 CLI flag 已設,這裡冗餘傳一次(server 接受 per-request override)。
        if let Some(lang) = &self.language {
            form = form.text("language", lang.clone());
        }

        let resp = self
            .http
            .post(&url)
            .multipart(form)
            .send()
            .await
            .context("POST /inference to whisper-server")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("whisper-server returned {}: {}", status, body);
        }

        // whisper.cpp server 回 `{"text": "..."}`(json) 或純文字(text format)。
        // 我們上面要求 json,直接 parse。
        let json: serde_json::Value = resp.json().await.context("parse whisper-server JSON")?;
        let text = json
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("whisper-server JSON has no `text` field: {json}"))?
            .trim()
            .to_string();

        if text.is_empty() {
            tracing::warn!("whisper-server returned empty transcription — mic may be muted");
        }

        Ok(text)
    }
}

// ─── 預設 / helpers ────────────────────────────────────────────────────

/// `~/.mori/models/ggml-small.bin` — 預設模型路徑。
pub fn default_model_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".mori").join("models").join("ggml-small.bin")
}

/// 預設 whisper-server binary 位置:`~/.mori/bin/whisper-server[.exe]`。
///
/// 若不存在,Command::new 會嘗試從 PATH 解(fallback)。user 想把 binary 放
/// 其他位置就在 config `providers.whisper-local.server_binary` 寫絕對路徑。
pub fn default_server_binary() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    let exe_name = if cfg!(windows) {
        "whisper-server.exe"
    } else {
        "whisper-server"
    };
    let in_mori_bin = home.join(".mori").join("bin").join(exe_name);
    if in_mori_bin.exists() {
        in_mori_bin
    } else {
        // 不存在就只給 binary 名,讓 OS PATH 解析(user 把 whisper-server 裝到
        // /usr/local/bin / C:\Program Files\... 也能用)。
        PathBuf::from(exe_name)
    }
}

fn mori_config_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(|h| PathBuf::from(h).join(".mori").join("config.json"))
}

/// 拿一個 OS 認可的閒置 port — bind :0 讓 kernel 配,讀完立刻釋放。
/// 有 TOCTOU 風險(可能 0.1ms 內被別人搶走),實務上幾乎不會踩到。
fn pick_free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").context("bind 127.0.0.1:0")?;
    let port = listener.local_addr().context("read assigned port")?.port();
    drop(listener);
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_model_path_in_mori_dir() {
        let path = default_model_path();
        let s = path.to_string_lossy();
        assert!(
            s.contains(".mori") && s.contains("models"),
            "default 路徑該指向 ~/.mori/models/, 實際:{s}",
        );
        assert!(path.file_name().unwrap().to_string_lossy().ends_with(".bin"));
    }

    #[test]
    fn default_server_binary_platform_correct() {
        let p = default_server_binary();
        let s = p.to_string_lossy();
        if cfg!(windows) {
            assert!(s.ends_with("whisper-server.exe"), "Windows binary 該帶 .exe: {s}");
        } else {
            assert!(s.ends_with("whisper-server"), "Unix binary 不該帶副檔名: {s}");
        }
    }

    #[test]
    fn provider_construction_fails_when_model_missing() {
        let nonexistent = PathBuf::from("/tmp/this-does-not-exist-mori-test.bin");
        let bin = PathBuf::from("whisper-server");
        let result = LocalWhisperProvider::new(&nonexistent, None, &bin);
        let err = match result {
            Ok(_) => panic!("expected Err for nonexistent model file"),
            Err(e) => e,
        };
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("not found") && msg.contains("huggingface"),
            "should include download instructions: {msg}"
        );
    }

    #[test]
    fn pick_free_port_returns_nonzero() {
        let p = pick_free_port().unwrap();
        assert!(p > 1024, "kernel 該配 ephemeral port (>1024), 實際:{p}");
    }
}
