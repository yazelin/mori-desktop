//! Phase 3C — Wake event 後的 intent evaluator。
//!
//! Hey Mori 喚醒後 STT 拿到 transcript,**先過一輪 fast LLM 判斷意圖**,再
//! 決定要不要進完整 agent loop。三種 outcome:
//!
//! - [`Intent::AddressMori`] — user 真的在跟 Mori 講話(指令 / 對話)。
//!   → 走 agent pipeline(現有行為)
//! - [`Intent::BackgroundNoise`] — wake-word false positive。user 不是在
//!   叫 Mori,是自言自語 / 跟別人講話被 mic 收到。
//!   → 直接 skip agent dispatch,emit `noise_rejected` event 給 UI 顯示
//! - [`Intent::Unclear`] — 模糊,可能是斷句不完整 / 半截話。
//!   → 走 agent 但加 hint 提示「user 句子不完整,可禮貌反問」
//!
//! ## 為什麼有這個
//!
//! Hey Mori wake threshold 設低(0.35-0.5)抓得到 user 聲線,但代價是
//! false positive 多 — user 自言自語講到「Mori」之類關鍵字就觸發。讓
//! 完整 agent loop(可能 spawn tool / call skill / 跑 1-3 LLM round)在
//! noise 上跑很浪費 + 體感怪(Mori 對自己的影子回話)。
//!
//! Evaluator 是一個 **單一 fast LLM call**(Groq oss-120b ~200ms),擋
//! 掉 noise 提升訊號比。
//!
//! ## 行為
//!
//! 預設 **OFF**(`evaluator.enabled = false`),既有行為不變。User 在
//! Config tab 主動 enable + 設 provider(預設 groq)才會啟動。

use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::llm::{ChatMessage, LlmProvider, ToolDefinition};

/// Wake event STT 後 evaluator 判出的 user 意圖。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    /// User 真的在跟 Mori 講話(指令 / 對話 / 問問題)
    AddressMori,
    /// Wake-word false positive,user 不是在叫 Mori
    BackgroundNoise,
    /// 模糊 / 不完整,可能需要反問
    Unclear,
}

/// Evaluator output。`intent` 必填,`reason` / `confidence` 可選。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationResult {
    pub intent: Intent,
    /// LLM 給的判斷理由(debug / log 用,UI 可選擇顯示)
    #[serde(default)]
    pub reason: String,
    /// 0.0..=1.0 信心值(LLM 自評,別太當真 — 主要看 intent)
    #[serde(default)]
    pub confidence: f32,
}

const EVALUATOR_SYSTEM_PROMPT: &str = r#"你是 Mori(桌面 AI 同伴)的 wake-word evaluator。每個 wake event 觸發後,user 的語音轉成 transcript 給你。你**只**判斷一件事:user 是不是在跟 Mori 講話?

三種輸出:
- `address_mori`:user 在跟 Mori 講話(指令 / 對話 / 問問題 / 閒聊)。明確或合理推斷都算。
- `background_noise`:wake-word false positive — user 在跟別人講話、自言自語、看影片唸出來、隨口提到「Mori」這個詞。**不是**在跟 Mori 互動。
- `unclear`:模糊 — 句子半截 / 內容空泛無上下文 / 像在思考但沒講完。

判準:
- 短指令「打開瀏覽器」「Mori 來幫我」「Hey Mori 查一下」→ address_mori
- 完整對話「Mori 你今天好嗎」「我想問你一件事」→ address_mori
- 自言自語「奇怪我剛剛在做什麼」「啊好煩」→ background_noise(沒提到 Mori 也沒指示)
- 別人對話「他剛剛在玩什麼遊戲」「等下要吃什麼」→ background_noise
- 半截話「然後...」「嗯...那個...」→ unclear
- 看 YouTube 念稿「主持人說很多 AI 助手像 Siri 或 Mori」→ background_noise(只是提及)

回 JSON,format:
{"intent":"address_mori|background_noise|unclear","reason":"<10字內理由>","confidence":0.0-1.0}

不要加 markdown fence,不要加說明,純 JSON。"#;

/// 主入口 — 給 transcript + provider,回 EvaluationResult。
///
/// LLM 失敗(網路 / parse error)→ fallback 到 [`Intent::AddressMori`],
/// 寧可 false positive 跑 agent 也不要 silent drop user 真的指令。
pub async fn evaluate(
    transcript: &str,
    provider: Arc<dyn LlmProvider>,
) -> Result<EvaluationResult> {
    let messages = vec![
        ChatMessage::system(EVALUATOR_SYSTEM_PROMPT.to_string()),
        ChatMessage::user(format!("Transcript:「{transcript}」")),
    ];
    let tools: Vec<ToolDefinition> = Vec::new();
    let chat = provider
        .chat(messages, tools)
        .await
        .context("evaluator LLM call failed")?;
    let raw = chat.content.unwrap_or_default();
    if raw.trim().is_empty() {
        return Err(anyhow!("evaluator returned empty response"));
    }
    match parse_evaluation(&raw) {
        Ok(result) => Ok(result),
        Err(e) => {
            tracing::warn!(
                error = %e,
                raw = %raw,
                "evaluator: parse failed, falling back to AddressMori",
            );
            Ok(EvaluationResult {
                intent: Intent::AddressMori,
                reason: "parse_failed".into(),
                confidence: 0.0,
            })
        }
    }
}

/// 從 LLM raw output parse JSON。寬容處理:
/// - 純 JSON ✓
/// - 包在 markdown fence(```json ... ```)→ 去 fence
/// - 前後有解釋文字 → 找第一個 `{` 跟最後一個 `}` 取中間
fn parse_evaluation(raw: &str) -> Result<EvaluationResult> {
    let trimmed = raw.trim();
    // 去 markdown fence
    let cleaned = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    // 嘗試直接 parse
    if let Ok(result) = serde_json::from_str::<EvaluationResult>(cleaned) {
        return Ok(result);
    }
    // 找第一個 `{` 跟最後一個 `}`(處理 LLM 加廢話的情況)
    let start = cleaned
        .find('{')
        .ok_or_else(|| anyhow!("no '{{' in evaluator output: {cleaned}"))?;
    let end = cleaned
        .rfind('}')
        .ok_or_else(|| anyhow!("no '}}' in evaluator output: {cleaned}"))?;
    if end <= start {
        return Err(anyhow!("malformed braces in evaluator output"));
    }
    let json_slice = &cleaned[start..=end];
    serde_json::from_str::<EvaluationResult>(json_slice)
        .with_context(|| format!("failed to parse evaluator JSON: {json_slice}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pure_json() {
        let raw = r#"{"intent":"address_mori","reason":"明確指令","confidence":0.95}"#;
        let r = parse_evaluation(raw).unwrap();
        assert_eq!(r.intent, Intent::AddressMori);
        assert_eq!(r.reason, "明確指令");
        assert!((r.confidence - 0.95).abs() < 0.001);
    }

    #[test]
    fn parse_with_markdown_fence() {
        let raw = "```json\n{\"intent\":\"background_noise\",\"reason\":\"自言自語\",\"confidence\":0.8}\n```";
        let r = parse_evaluation(raw).unwrap();
        assert_eq!(r.intent, Intent::BackgroundNoise);
    }

    #[test]
    fn parse_with_extra_text() {
        let raw = "Sure thing! Here's my analysis:\n{\"intent\":\"unclear\",\"reason\":\"半截話\",\"confidence\":0.5}\nLet me know if you need more.";
        let r = parse_evaluation(raw).unwrap();
        assert_eq!(r.intent, Intent::Unclear);
    }

    #[test]
    fn parse_minimal_json() {
        let raw = r#"{"intent":"address_mori"}"#;
        let r = parse_evaluation(raw).unwrap();
        assert_eq!(r.intent, Intent::AddressMori);
        assert_eq!(r.reason, "");
        assert_eq!(r.confidence, 0.0);
    }

    #[test]
    fn parse_invalid_intent_fails() {
        let raw = r#"{"intent":"foo","reason":"x","confidence":1.0}"#;
        assert!(parse_evaluation(raw).is_err());
    }
}
