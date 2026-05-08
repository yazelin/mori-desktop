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
/// - `default_transcribe_provider`: "groq"(預設) | "whisper-local"
/// - `providers.groq.{api_key, transcribe_model}` (Groq 路徑)
/// - `providers.whisper-local.{model_path, language}` (本機 whisper 路徑)
///
/// retry_callback 只在 Groq 路徑套用(本機 Whisper 沒 rate-limit)。
pub fn build_transcription_provider(
    retry_cb: Option<groq::RetryCallback>,
) -> Result<Arc<dyn TranscriptionProvider>> {
    let default = read_default_transcribe_provider();

    match default.as_str() {
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
        other => {
            if other != "groq" {
                tracing::warn!(
                    provider = other,
                    "unknown default_transcribe_provider — falling back to 'groq'",
                );
            }
            let key = groq::GroqProvider::discover_api_key().ok_or_else(|| {
                anyhow::anyhow!(
                    "no GROQ_API_KEY configured for STT. Edit ~/.mori/config.json or set \
                     $GROQ_API_KEY (or set default_transcribe_provider to 'whisper-local' \
                     for local STT)"
                )
            })?;
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/groq/transcribe_model"))
                .unwrap_or_else(|| groq::GroqProvider::DEFAULT_TRANSCRIBE_MODEL.to_string());
            let chat_model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/groq/chat_model"))
                .unwrap_or_else(|| groq::GroqProvider::DEFAULT_CHAT_MODEL.to_string());
            tracing::info!(
                provider = "groq",
                model = %model,
                "transcription provider selected",
            );
            // GroqProvider 用一個構造同時帶 chat_model + transcribe_model;
            // 我們這只用 transcribe 路徑,但建構時還是要給 chat_model(沒空值)。
            let p = groq::GroqProvider::new(key, chat_model)
                .with_transcribe_model(model);
            let p = if let Some(cb) = retry_cb {
                p.with_retry_callback(cb)
            } else {
                p
            };
            Ok(Arc::new(p))
        }
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
    let default = read_default_transcribe_provider();
    match default.as_str() {
        "whisper-local" => {
            let model_path = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/whisper-local/model_path"))
                .unwrap_or_else(|| {
                    super::whisper_local::default_model_path()
                        .display()
                        .to_string()
                });
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
                .and_then(|p| groq::read_json_pointer(p, "/providers/groq/transcribe_model"))
                .unwrap_or_else(|| groq::GroqProvider::DEFAULT_TRANSCRIBE_MODEL.to_string());
            TranscribeSnapshot {
                name: "groq".into(),
                model,
                language: None,
            }
        }
    }
}

fn read_default_transcribe_provider() -> String {
    mori_config_path()
        .as_deref()
        .and_then(|p| groq::read_json_pointer(p, "/default_transcribe_provider"))
        .unwrap_or_else(|| "groq".to_string())
}

fn mori_config_path() -> Option<std::path::PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".mori").join("config.json"))
}
