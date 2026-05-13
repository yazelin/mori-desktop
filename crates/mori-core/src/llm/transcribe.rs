//! Speech-to-text 抽象。
//!
//! Phase 5C 把 STT 從 `LlmProvider::transcribe`(原本擠在同一個 trait
//! 裡)拆出來。原因:
//! - 一個「provider」要 implement chat **跟** transcribe 兩件事 是過度
//!   耦合 — Ollama / Claude CLI 沒做 STT 也沒打算做,逼它們有 transcribe
//!   就只能 bail "not supported" 占位。
//! - LocalWhisperProvider 只想做 STT,不會 chat。給它 LlmProvider 的型
//!   別會逼它對 chat() 也回傳一個假 error。
//!
//! 拆出 [`TranscriptionProvider`] 後,GroqProvider 同時實作兩個 trait
//! (有 chat 又有 STT),LocalWhisperProvider 只實作 [`TranscriptionProvider`]。
//! main.rs 的 stage 1 (audio → text) 走這個 trait 的 factory 拿到對應
//! provider,跟 chat 路徑完全解耦,使用者可以「Whisper 走 Groq + chat
//! 走 ollama」或「STT 也離線、整套 100% groq-free」。

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use super::groq;

#[async_trait]
pub trait TranscriptionProvider: Send + Sync {
    /// Provider 識別名(groq / whisper-local / ...)
    fn name(&self) -> &'static str;

    /// 把 WAV-encoded 音訊位元組轉成文字。
    ///
    /// 為什麼是 WAV bytes 不是 raw f32 samples:跟 Groq Whisper API 的
    /// multipart 上傳格式對齊;LocalWhisperProvider 內部會解碼 WAV →
    /// f32 → 重採樣到 16kHz → 餵 whisper-rs。多一層 encode/decode 的
    /// CPU 成本可忽略(< 50ms),換來 trait 介面跟 Groq 共用。
    async fn transcribe(&self, audio: Vec<u8>) -> Result<String>;
}

/// 從 `~/.mori/config.json` 蓋出 transcription provider。
///
/// 配置:
/// - `stt_provider`: "groq"(預設) | "whisper-local"
/// - `providers.groq.{api_key, stt_model}` (Groq 路徑)
/// - `providers.whisper-local.{model_path, language}` (本機 whisper 路徑)
///
/// retry_callback 只在 Groq 路徑套用(本機 Whisper 沒 rate-limit)。
pub fn build_transcription_provider(
    retry_cb: Option<groq::RetryCallback>,
) -> Result<Arc<dyn TranscriptionProvider>> {
    let default = read_stt_provider_config();

    match default.as_str() {
        #[cfg(target_os = "linux")]
        "whisper-local" => {
            let p = super::whisper_local::LocalWhisperProvider::from_config()?;
            tracing::info!(
                provider = "whisper-local",
                model_path = %p.model_path().display(),
                language = ?p.language(),
                "transcription provider selected",
            );
            Ok(Arc::new(p))
        }
        #[cfg(not(target_os = "linux"))]
        "whisper-local" => {
            anyhow::bail!(
                "whisper-local STT not supported on this platform yet (whisper-rs-sys \
                 Windows bindgen issue). Switch stt_provider to 'groq' in ~/.mori/config.json."
            );
        }
        other => {
            if other != "groq" {
                tracing::warn!(
                    provider = other,
                    "unknown stt_provider — falling back to 'groq'",
                );
            }
            let key = groq::GroqProvider::discover_api_key().ok_or_else(|| {
                anyhow::anyhow!(
                    "no GROQ_API_KEY configured for STT. Edit ~/.mori/config.json or set \
                     $GROQ_API_KEY (or set stt_provider to 'whisper-local' \
                     for local STT)"
                )
            })?;
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/groq/stt_model"))
                .unwrap_or_else(|| groq::GroqProvider::DEFAULT_STT_MODEL.to_string());
            let llm_model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/groq/model"))
                .unwrap_or_else(|| groq::GroqProvider::DEFAULT_MODEL.to_string());
            tracing::info!(
                provider = "groq",
                model = %model,
                "transcription provider selected",
            );
            // GroqProvider 構造同時帶 LLM model + stt_model;
            // 我們這只用 STT 路徑,但建構時還是要給 LLM model(沒空值)。
            let p = groq::GroqProvider::new(key, llm_model)
                .with_stt_model(model);
            let p = if let Some(cb) = retry_cb {
                p.with_retry_callback(cb)
            } else {
                p
            };
            Ok(Arc::new(p))
        }
    }
}

/// 跟 [`build_transcription_provider`] 一樣，但**直接指定 provider 名稱**，
/// 不讀 config 的 `stt_provider`。給 voice input profile 覆蓋用：
/// profile frontmatter 設 `stt_provider: whisper-local` 時呼叫這個。
pub fn build_named_transcription_provider(
    name: &str,
    retry_cb: Option<groq::RetryCallback>,
) -> Result<Arc<dyn TranscriptionProvider>> {
    match name {
        #[cfg(target_os = "linux")]
        "whisper-local" => {
            let p = super::whisper_local::LocalWhisperProvider::from_config()?;
            tracing::info!(
                provider = "whisper-local",
                model_path = %p.model_path().display(),
                "transcription provider selected (profile override)",
            );
            Ok(Arc::new(p))
        }
        #[cfg(not(target_os = "linux"))]
        "whisper-local" => {
            anyhow::bail!(
                "whisper-local STT not supported on this platform yet (whisper-rs-sys \
                 Windows bindgen issue). Use 'groq' instead."
            );
        }
        "groq" => {
            let key = groq::GroqProvider::discover_api_key().ok_or_else(|| {
                anyhow::anyhow!(
                    "no GROQ_API_KEY for STT override. Set GROQ_API_KEY or providers.groq.api_key"
                )
            })?;
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/groq/stt_model"))
                .unwrap_or_else(|| groq::GroqProvider::DEFAULT_STT_MODEL.to_string());
            let llm_model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/groq/model"))
                .unwrap_or_else(|| groq::GroqProvider::DEFAULT_MODEL.to_string());
            tracing::info!(
                provider = "groq",
                model = %model,
                "transcription provider selected (profile override)",
            );
            let p = groq::GroqProvider::new(key, llm_model).with_stt_model(model);
            let p = if let Some(cb) = retry_cb {
                p.with_retry_callback(cb)
            } else {
                p
            };
            Ok(Arc::new(p))
        }
        other => anyhow::bail!(
            "unknown STT provider '{}' — supported: groq, whisper-local",
            other
        ),
    }
}

/// 給 IPC `chat_provider_info` / log 用 — 不需要構造 provider 就能知道
/// 目前生效的 transcribe provider 設定。
#[derive(Debug, Clone)]
pub struct TranscribeSnapshot {
    pub name: String,
    /// Groq 是 model id,whisper-local 是 model 檔案路徑(.bin)
    pub model: String,
    /// whisper-local 才有;Groq 為 None
    pub language: Option<String>,
}

pub fn active_transcribe_snapshot() -> TranscribeSnapshot {
    let default = read_stt_provider_config();
    match default.as_str() {
        "whisper-local" => {
            let model_path = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/whisper-local/model_path"))
                .unwrap_or_else(default_whisper_model_path_display);
            let language = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/whisper-local/language"));
            TranscribeSnapshot {
                name: "whisper-local".into(),
                model: model_path,
                language,
            }
        }
        _ => {
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/groq/stt_model"))
                .unwrap_or_else(|| groq::GroqProvider::DEFAULT_STT_MODEL.to_string());
            TranscribeSnapshot {
                name: "groq".into(),
                model,
                language: None,
            }
        }
    }
}

fn read_stt_provider_config() -> String {
    mori_config_path()
        .as_deref()
        .and_then(|p| groq::read_json_pointer(p, "/stt_provider"))
        .unwrap_or_else(|| "groq".to_string())
}

fn mori_config_path() -> Option<std::path::PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".mori").join("config.json"))
}

/// Default `.bin` path for the snapshot UI / config stub. Linux uses the real
/// whisper_local default; non-Linux uses the same `~/.mori/models/ggml-small.bin`
/// shape for UI consistency (whisper-local is gated off at the provider build site
/// — this path is purely informational on non-Linux).
fn default_whisper_model_path_display() -> String {
    #[cfg(target_os = "linux")]
    {
        super::whisper_local::default_model_path()
            .display()
            .to_string()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        std::path::PathBuf::from(home)
            .join(".mori")
            .join("models")
            .join("ggml-small.bin")
            .display()
            .to_string()
    }
}
