//! desktop 端 STT provider:把語音轉錄**委派給 mori-ear**(耳朵器官)。
//!
//! 取代舊的 `whisper_local.rs`(desktop 自己 spawn 私有 whisper-server + 管模型檔)。
//! 現在 mori-ear 是 Mori 宇宙的單一 STT provider;desktop 只當薄 client:
//!
//!   1. 讀 ear 的 descriptor `~/.mori/mori-ear-server.json` → 驗活(loopback + `GET /` 200)。
//!   2. 沒在線 → **lazy-spawn `mori-ear --serve`**(像舊版 lazy-spawn whisper-server),
//!      poll descriptor ready(≤12s)後再用。spawn 出來的 ear 是 detached、**不** kill-on-drop
//!      —— 耳朵要比 desktop 長壽(desktop rebuild / 關掉,耳朵還在聽 + 還能轉錄)。
//!   3. `POST /inference`(multipart `file` + `language` + `backend`)→ 回 `{"text"}`。
//!
//! backend(per-request 傳給 ear,對應 ear 的 `auto`/`local`):
//!   - `auto`(hotkey 語音路徑,[`EarTranscriptionProvider::from_config`]):交給 ear 決定 ——
//!     本機 whisper-server 優先、不可用才 Groq。「just works」。
//!   - `local`(轉檔 UI,[`EarTranscriptionProvider::force_local`]):強制本機、永不上雲
//!     (守住「拿已有音檔產逐字稿」頁的隱私承諾)。
//!
//! reqwest 安全(對齊 ear `local_stt.rs` / 舊 whisper_local):`no_proxy()` + `redirect(none)`
//! + host pin loopback。

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, bail, Context as _, Result};
use async_trait::async_trait;
use tokio::sync::Mutex;

use super::transcribe::TranscriptionProvider;

const DESCRIPTOR_NAME: &str = "mori-ear-server.json";
/// lazy-spawn 後等 descriptor ready 的上限。ear --serve 起得很快(~100ms),給足 buffer。
const READY_TIMEOUT_SECS: u64 = 12;
/// `/inference` 整體 timeout。刻意 > ear 自己的 watchdog(預設 90s),讓 ear 的看門狗先
/// 觸發、回乾淨錯誤,而不是 desktop 這端先 timeout 留個半死連線。
const INFERENCE_TIMEOUT_SECS: u64 = 120;

pub struct EarTranscriptionProvider {
    /// `"auto"` | `"local"` —— per-request 傳給 ear。
    backend: String,
    language: Option<String>,
    http: reqwest::Client,
    /// 序列化 lazy-spawn,避免並發多個 transcribe 同時 spawn 出多個 `mori-ear --serve`。
    spawn_lock: Mutex<()>,
}

impl EarTranscriptionProvider {
    /// 保留 config key `whisper-local` 不變 —— `stt_provider: whisper-local` /
    /// profile frontmatter / config.json 全部沿用,使用者無感切換到 mori-ear。
    pub const NAME: &'static str = "whisper-local";

    pub fn new(backend: &str, language: Option<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(INFERENCE_TIMEOUT_SECS))
            .build()
            .context("build reqwest client for mori-ear")?;
        Ok(Self {
            backend: backend.to_string(),
            language,
            http,
            spawn_lock: Mutex::new(()),
        })
    }

    /// hotkey 語音路徑:`backend=auto`(ear 決定 local-first / Groq fallback)。
    /// language 讀 `~/.mori/config.json` `providers.whisper-local.language`。
    pub fn from_config() -> Result<Self> {
        Self::new("auto", config_language())
    }

    /// 轉檔 UI:`backend=local`(強制本機、永不上雲)。language 優先用呼叫端覆寫,
    /// 沒給就讀 config。
    pub fn force_local(language: Option<String>) -> Result<Self> {
        Self::new("local", language.or_else(config_language))
    }

    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }
}

#[async_trait]
impl TranscriptionProvider for EarTranscriptionProvider {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    async fn transcribe(&self, audio: Vec<u8>) -> Result<String> {
        let (host, port) = self.ensure_ear().await?;
        let url = format!("http://{host}:{port}/inference");

        let part = reqwest::multipart::Part::bytes(audio)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .context("build multipart audio part")?;
        let mut form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("backend", self.backend.clone());
        if let Some(lang) = &self.language {
            form = form.text("language", lang.clone());
        }

        let resp = self
            .http
            .post(&url)
            .multipart(form)
            .send()
            .await
            .context("POST /inference to mori-ear")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("mori-ear /inference returned {status}: {body}");
        }

        let json: serde_json::Value = resp.json().await.context("parse mori-ear JSON")?;
        let text = json
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("mori-ear /inference JSON has no `text` field: {json}"))?
            .trim()
            .to_string();
        if text.is_empty() {
            tracing::warn!("mori-ear returned empty transcription — mic may be muted / silence");
        }
        Ok(text)
    }
}

impl EarTranscriptionProvider {
    /// 確保 ear 服務在線;不在就 lazy-spawn `mori-ear --serve` 等它 ready。回 (host, port)。
    async fn ensure_ear(&self) -> Result<(String, u16)> {
        if let Some(hp) = self.read_alive().await {
            return Ok(hp);
        }
        // 序列化 spawn:拿鎖後再驗一次(別人可能剛 spawn 完)。
        let _g = self.spawn_lock.lock().await;
        if let Some(hp) = self.read_alive().await {
            return Ok(hp);
        }
        spawn_mori_ear_serve()?;
        let deadline = std::time::Instant::now() + Duration::from_secs(READY_TIMEOUT_SECS);
        loop {
            if let Some(hp) = self.read_alive().await {
                return Ok(hp);
            }
            if std::time::Instant::now() >= deadline {
                bail!(
                    "lazy-spawn 了 `mori-ear --serve` 但 {READY_TIMEOUT_SECS}s 內服務沒 ready。\
                     確認 mori-ear 已安裝(`cargo install --path .`,binary 在 ~/.cargo/bin / \
                     ~/.mori/bin / PATH)。"
                );
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// 讀 descriptor + 驗活(loopback host pin + `GET /` 200)。活著回 (host, port)。
    async fn read_alive(&self) -> Option<(String, u16)> {
        let path = descriptor_path()?;
        let text = std::fs::read_to_string(&path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        let host = v.get("host").and_then(|h| h.as_str())?.to_string();
        let port = v.get("port").and_then(|p| p.as_u64())? as u16;
        if !(host == "127.0.0.1" || host == "::1") || port == 0 {
            return None;
        }
        let alive = matches!(
            self.http
                .get(format!("http://{host}:{port}/"))
                .timeout(Duration::from_secs(2))
                .send()
                .await,
            Ok(r) if r.status().is_success()
        );
        alive.then_some((host, port))
    }
}

/// 給轉檔頁 dep-check 用:mori-ear 是否可用(binary 找得到 或 已有 descriptor)+ 解析到的位置。
pub fn ear_availability() -> (bool, String) {
    let bin = resolve_mori_ear_binary();
    let bin_found = bin.is_absolute() && bin.exists();
    let desc_exists = descriptor_path().map(|p| p.exists()).unwrap_or(false);
    let path_str = if bin_found {
        bin.display().to_string()
    } else {
        format!("{} (PATH)", bin.display())
    };
    (bin_found || desc_exists, path_str)
}

fn config_language() -> Option<String> {
    mori_config_path()
        .as_deref()
        .and_then(|p| super::groq::read_json_pointer(p, "/providers/whisper-local/language"))
}

fn descriptor_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".mori").join(DESCRIPTOR_NAME))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

fn mori_config_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".mori").join("config.json"))
}

/// spawn `mori-ear --serve` —— detached(stdio null、**不**留 Child handle → 不 kill-on-drop,
/// 耳朵要比 desktop 長壽)。
fn spawn_mori_ear_serve() -> Result<()> {
    let binary = resolve_mori_ear_binary();
    let mut cmd = Command::new(&binary);
    cmd.arg("--serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::suppress_console_on_windows!(cmd);
    cmd.spawn().with_context(|| {
        format!(
            "spawn `{} --serve` 失敗 —— 確認 mori-ear 已安裝(`cargo install --path .` 放進 \
             ~/.cargo/bin,或丟 ~/.mori/bin,或在 PATH)。",
            binary.display()
        )
    })?;
    tracing::info!(binary = %binary.display(), "lazy-spawned `mori-ear --serve`");
    Ok(())
}

/// 解析 mori-ear binary:`~/.cargo/bin` → `~/.mori/bin` → 裸名(交給 OS PATH 解)。
/// Windows 補 `.exe`。
fn resolve_mori_ear_binary() -> PathBuf {
    let exe = if cfg!(windows) {
        "mori-ear.exe"
    } else {
        "mori-ear"
    };
    if let Some(home) = home_dir() {
        let cargo_bin = home.join(".cargo").join("bin").join(exe);
        if cargo_bin.exists() {
            return cargo_bin;
        }
        let mori_bin = home.join(".mori").join("bin").join(exe);
        if mori_bin.exists() {
            return mori_bin;
        }
    }
    PathBuf::from(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_binary_platform_suffix() {
        let p = resolve_mori_ear_binary();
        let s = p.to_string_lossy();
        if cfg!(windows) {
            assert!(s.ends_with("mori-ear.exe"), "Windows 該帶 .exe: {s}");
        } else {
            assert!(s.ends_with("mori-ear"), "Unix 不帶副檔名: {s}");
        }
    }

    #[test]
    fn descriptor_path_under_dot_mori() {
        if let Some(p) = descriptor_path() {
            let s = p.to_string_lossy();
            assert!(s.contains(".mori") && s.ends_with("mori-ear-server.json"), "{s}");
        }
    }

    #[test]
    fn force_local_uses_local_backend() {
        let p = EarTranscriptionProvider::force_local(Some("zh".into())).unwrap();
        assert_eq!(p.backend, "local");
        assert_eq!(p.language(), Some("zh"));
        assert_eq!(p.name(), "whisper-local");
    }

    #[test]
    fn from_config_uses_auto_backend() {
        let p = EarTranscriptionProvider::from_config().unwrap();
        assert_eq!(p.backend, "auto");
    }
}
