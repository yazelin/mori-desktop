//! LLM-based STT 諧音錯字偵測。對話結束後 spawn 一個 task 跑,結果寫進
//! `correction_inbox.jsonl` 等 user 在 CorrectionsTab 確認。
//!
//! Prompt 對齊 spec §4.2。Provider: groq,model 預設 openai/gpt-oss-120b。

use crate::llm::{ChatMessage, LlmProvider, ToolDefinition};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::sync::Arc;

/// LLM 標出的單筆候選。
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AuditCandidate {
    pub wrong: String,
    pub suggested: String,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    #[serde(default)]
    pub reason: String,
}

fn default_confidence() -> f64 {
    0.7
}

/// 跑 audit。Provider trait 由 caller 給(平常 Tauri 端 inject groq provider)。
///
/// 三條過濾:
/// - LLM 回的 `wrong` 不在 transcript_raw 內 → drop(LLM 幻覺)
/// - wrong / suggested 任一空 → drop
/// - 兩者完全相同 → drop
pub async fn audit(
    provider: Arc<dyn LlmProvider>,
    transcript_raw: &str,
    transcript_cleaned: &str,
    corrections_md: &str,
) -> Result<Vec<AuditCandidate>> {
    let system = build_prompt(transcript_raw, transcript_cleaned, corrections_md);
    let messages = vec![ChatMessage::system(&system)];
    let tools: Vec<ToolDefinition> = Vec::new();
    let response = provider
        .chat(messages, tools)
        .await
        .context("correction audit provider chat")?;
    let content = response.content.unwrap_or_default();
    let candidates = parse_response(&content)?;
    Ok(filter(candidates, transcript_raw))
}

fn build_prompt(transcript_raw: &str, transcript_cleaned: &str, corrections_md: &str) -> String {
    format!(
        r#"你是 STT 諧音錯字偵測器。我會給你三段:
1. transcript_raw — STT 直接輸出(可能有諧音錯字、無標點)
2. transcript_cleaned — LLM 校正後版本(已用 corrections.md 處理過 + LLM 自己加標點 segmentation)
3. corrections.md — 現有校正字典(格式:錯字, 錯字2 -> 正字)

任務:列出 transcript_raw 內**可能是 STT 諧音錯字**的詞 + 建議正字。

判斷依據:
- raw → cleaned 過程被改寫的詞(LLM 校過 = 高機率錯字)
- 跟 corrections.md 內錯字組同音 / 形近(已知類型擴散)
- 語義不合 / 諧音怪 / 不像合理中文用語(新類型)

排除:
- 純標點 / segmentation 差異(LLM 加標點不算錯字)
- 已在 corrections.md 內的 entries(那條已經會用)

回 **純 JSON array**,每筆 {{ wrong, suggested, confidence: 0..1, reason: 一句中文說明 }}。
找不到候選回 []。**不要包 markdown code fence,不要附前後說明,只回 JSON**。

==== transcript_raw ====
{transcript_raw}

==== transcript_cleaned ====
{transcript_cleaned}

==== corrections.md ====
{corrections_md}
"#
    )
}

fn parse_response(content: &str) -> Result<Vec<AuditCandidate>> {
    let trimmed = content.trim();
    // 容忍 LLM 偶爾包 ```json fence
    let json_str = if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            after[..end].trim()
        } else {
            after
        }
    } else if trimmed.starts_with("```") {
        let after = &trimmed[3..];
        if let Some(end) = after.find("```") {
            after[..end].trim()
        } else {
            after
        }
    } else {
        trimmed
    };
    serde_json::from_str(json_str)
        .map_err(|e| anyhow!("audit response JSON parse failed: {e} (content: {content:?})"))
}

fn filter(candidates: Vec<AuditCandidate>, transcript_raw: &str) -> Vec<AuditCandidate> {
    candidates
        .into_iter()
        .filter(|c| {
            !c.wrong.is_empty()
                && !c.suggested.is_empty()
                && c.wrong != c.suggested
                && transcript_raw.contains(&c.wrong)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_json_array() {
        let content = r#"[{"wrong":"英檔","suggested":"音檔","confidence":0.85,"reason":"同音"}]"#;
        let result = parse_response(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].wrong, "英檔");
        assert_eq!(result[0].suggested, "音檔");
        assert!((result[0].confidence - 0.85).abs() < 1e-9);
    }

    #[test]
    fn parse_with_json_fence() {
        let content = r#"```json
[{"wrong":"英檔","suggested":"音檔","confidence":0.85,"reason":"x"}]
```"#;
        let result = parse_response(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].wrong, "英檔");
    }

    #[test]
    fn parse_empty_array_ok() {
        let result = parse_response("[]").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_malformed_returns_err() {
        let result = parse_response("not json");
        assert!(result.is_err());
    }

    #[test]
    fn filter_drops_hallucinated_wrong_not_in_raw() {
        let c1 = AuditCandidate {
            wrong: "英檔".into(),
            suggested: "音檔".into(),
            confidence: 0.9,
            reason: "".into(),
        };
        let c2 = AuditCandidate {
            wrong: "不存在的字".into(),
            suggested: "X".into(),
            confidence: 0.9,
            reason: "".into(),
        };
        let result = filter(vec![c1, c2], "我看英檔的內容");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].wrong, "英檔");
    }

    #[test]
    fn filter_drops_identical_wrong_and_suggested() {
        let c1 = AuditCandidate {
            wrong: "音檔".into(),
            suggested: "音檔".into(),
            confidence: 0.9,
            reason: "".into(),
        };
        let result = filter(vec![c1], "音檔");
        assert!(result.is_empty());
    }

    #[test]
    fn filter_drops_empty_fields() {
        let c1 = AuditCandidate {
            wrong: "".into(),
            suggested: "X".into(),
            confidence: 0.9,
            reason: "".into(),
        };
        let c2 = AuditCandidate {
            wrong: "X".into(),
            suggested: "".into(),
            confidence: 0.9,
            reason: "".into(),
        };
        let result = filter(vec![c1, c2], "X");
        assert!(result.is_empty());
    }
}
