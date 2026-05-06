//! 語音輸入處理。
//!
//! Phase 1:接 LlmProvider::transcribe(目前只 GroqProvider 實作 Whisper)。
//! Phase 4+:加本地 Whisper 選項(whisper-rs / whisper.cpp 綁定),供無網路時使用。

use anyhow::Result;

use crate::llm::LlmProvider;

/// 把音訊位元組轉成文字。
///
/// 使用提供的 [`LlmProvider`] 的 transcribe 能力。對 Groq 來說會打 Whisper API。
pub async fn transcribe<P: LlmProvider + ?Sized>(provider: &P, audio: Vec<u8>) -> Result<String> {
    provider.transcribe(audio).await
}
