//! PasteSelectionBackSkill — 回填處理結果到使用者原本反白的位置。
//!
//! 流程:
//! 1. 使用者反白文字(Wayland primary selection / X11 PRIMARY)
//! 2. 講話下指令(「翻成英文」「潤一下」「改更短」)
//! 3. LLM 看到 `selected_text` in context,先呼叫 translate / polish /
//!    summarize 拿結果
//! 4. **這個 skill** 把結果寫剪貼簿 + 模擬 Ctrl+V → 取代原反白
//!
//! 關鍵:LLM 要在處理結果出來「之後」才呼叫,且只在使用者意圖是
//! **修改**(verb)而不是**問問題**(question)時呼叫。
//!
//! 使用情境分類:
//!   ✅ 「翻譯這段」→ paste(翻譯結果)
//!   ✅ 「潤一下」 → paste(潤稿結果)
//!   ❌ 「這在講什麼」→ 不 paste,結果在 popover 顯示

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::context::Context;
use crate::paste::{PasteController, PasteResult};
use super::{Skill, SkillOutput};

pub struct PasteSelectionBackSkill {
    controller: Arc<dyn PasteController>,
}

impl PasteSelectionBackSkill {
    pub fn new(controller: Arc<dyn PasteController>) -> Self {
        Self { controller }
    }
}

#[async_trait]
impl Skill for PasteSelectionBackSkill {
    fn name(&self) -> &'static str {
        "paste_selection_back"
    }

    fn description(&self) -> &'static str {
        "Replace the user's currently-selected text with `text`. Call this \
         AFTER you've finished processing the user's selected_text — for \
         example after `translate` / `polish` / `summarize` returned a \
         result and the user clearly wanted the selection MODIFIED \
         (verbs: 翻譯, 潤稿, 摘要, 改寫, 改短, 改成 X 語氣). DON'T call \
         it when the user just asked a question about the selection \
         (e.g. '這在講什麼', 'what does this mean') — for those, just \
         answer in chat. Internally writes `text` to the clipboard then \
         simulates the platform paste shortcut (Ctrl+V on Linux/Windows, \
         Cmd+V on macOS), so the result lands in whatever app the user \
         was selecting in. Mori's window is non-focus-stealing so the \
         original app keeps focus and receives the paste."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "處理過的最終文字 — 會直接取代使用者原本反白的範圍。"
                }
            },
            "required": ["text"]
        })
    }

    fn confirm_required(&self) -> bool {
        // 雖然會改使用者編輯區內容,但因為這是「使用者明確要求改寫」的
        // 直接結果,不該攔下二次確認 — 那會打斷流暢度。如果之後發現
        // 誤觸發太多,再升級成 confirm_required = true。
        false
    }

    fn platform_caveat(&self) -> Option<&'static str> {
        // Windows 沒有 X11 PRIMARY selection — user 必須先 Ctrl+C 才有東西貼。
        // Linux 拖反白即可讀,paste 完全自動。macOS 同 Windows。
        if cfg!(target_os = "linux") {
            None
        } else {
            Some(
                "此平台沒有「滑鼠反白即讀」(X11 PRIMARY selection 是 Linux 特有)。\
                 使用此 skill 前要先 Ctrl+C / Cmd+C 把選取內容放進剪貼簿。",
            )
        }
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing text"))?
            .to_string();

        if text.is_empty() {
            return Ok(SkillOutput {
                user_message: "(沒東西可貼,跳過)".to_string(),
                data: None,
            });
        }

        let result = self.controller.paste_back(&text).await?;

        let preview: String = text.chars().take(40).collect();
        let suffix = if text.chars().count() > 40 { "…" } else { "" };
        let (user_message, pasted) = match result {
            PasteResult::Pasted => (
                format!("已貼回反白範圍:「{preview}{suffix}」"),
                true,
            ),
            PasteResult::ClipboardOnly => (
                format!(
                    "結果已放剪貼簿(「{preview}{suffix}」),但模擬 Ctrl+V 失敗 — \
                     ydotoold 可能沒在跑,請手動 Ctrl+V 貼上。\
                     之後跑 setup-wayland-input.sh + 重開機一次能修。"
                ),
                false,
            ),
        };

        Ok(SkillOutput {
            user_message,
            data: Some(serde_json::json!({
                "pasted_chars": text.chars().count(),
                "pasted": pasted,
            })),
        })
    }
}
