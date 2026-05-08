//! TranslateSkill — 翻譯文字。LLM 內部再呼叫一次 chat 做實際翻譯。

use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::context::Context;
use crate::llm::{ChatMessage, LlmProvider};
use super::{Skill, SkillOutput};

pub struct TranslateSkill {
    provider: Arc<dyn LlmProvider>,
}

impl TranslateSkill {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Skill for TranslateSkill {
    fn name(&self) -> &'static str {
        "translate"
    }

    fn description(&self) -> &'static str {
        "Translate text from one language to another. Use when the user \
         explicitly asks to translate something (e.g. '幫我翻成英文', \
         'translate this to Japanese')."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "source_text": { "type": "string", "description": "要翻譯的原文" },
                "target_lang": {
                    "type": "string",
                    "description": "目標語言。常用:zh-TW(繁中)、zh-CN(簡中)、en、ja、ko。\
                                   **省略時預設 zh-TW**(我們主要使用者是繁中講者)。"
                }
            },
            // `target_lang` 故意不放 required — Groq 的 tool validator 會
            // 擋掉「LLM 漏帶 required」的整個 call(HTTP 400 tool_use_failed),
            // 即使使用者講「翻譯成中文」LLM 仍偶爾會省略此欄。允許省略 +
            // 預設 zh-TW 比 hard fail 友善很多。
            "required": ["source_text"]
        })
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let source_text = args
            .get("source_text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing source_text"))?
            .trim()
            .to_string();
        // target_lang 缺省 → 預設 zh-TW(對齊 schema description 的承諾)
        let target_lang = args
            .get("target_lang")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "zh-TW".to_string());

        let messages = vec![
            ChatMessage::system(
                "You are a precise translator. Translate the user's text into the requested \
                 target language.\n\n\
                 Rules:\n\
                 - Output ONLY the translated text, nothing else.\n\
                 - No preamble like \"Here is the translation:\".\n\
                 - No explanation of word choices.\n\
                 - No quotation marks unless they were in the original.\n\
                 - Preserve formatting (line breaks, lists) of the original.\n\
                 - If the source is already in the target language, output it unchanged.\n\
                 - For target_lang `zh-TW`, use Taiwan Mandarin (繁體中文 + 台灣慣用詞,\
                   例如「軟體」「滑鼠」「影片」,不是「軟件」「鼠標」「視頻」)。"
            ),
            ChatMessage::user(format!(
                "Target language: {target_lang}\n\nText to translate:\n{source_text}"
            )),
        ];

        let resp = self
            .provider
            .chat(messages, vec![])
            .await
            .context("translate: provider chat")?;
        let translated = resp
            .content
            .ok_or_else(|| anyhow!("LLM returned no content"))?
            .trim()
            .to_string();

        Ok(SkillOutput {
            user_message: translated.clone(),
            data: Some(serde_json::json!({
                "target_lang": target_lang,
                "translated": translated,
            })),
        })
    }
}
