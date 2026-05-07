//! Agent 迴圈 — 把 LLM、Skill registry、Memory store 拼成 Mori 的「大腦」。
//!
//! Phase 1D 採用「single round of tool use」模式:
//!
//! ```text
//!   user input
//!      │
//!      ▼
//!   provider.chat(messages, tools)
//!      │
//!      ├─ 有 content,沒 tool_calls → 直接回傳 content
//!      │
//!      ├─ 有 tool_calls(可能也有 content)→
//!      │      execute(tool_calls)
//!      │      回傳 = LLM content(若有)+ skills 的 user_messages(疊在後面)
//!      │
//!      └─ 都沒有 → 回傳空字串(罕見、防禦性 fallback)
//! ```
//!
//! 不做多輪 tool-call 迴圈(LLM 看到 tool 結果後再講話)— 那需要把
//! ChatMessage 擴展成支援 assistant.tool_calls + tool.tool_call_id,留到
//! phase 2+ 真正需要時再做。phase 1D 的 RememberSkill 是「fire and forget」,
//! 單輪就夠了。

use std::sync::Arc;

use anyhow::{Context as _, Result};

use crate::context::Context;
use crate::llm::{ChatMessage, LlmProvider};
use crate::skill::{SkillOutput, SkillRegistry};

/// 一輪 agent 執行的完整結果。
pub struct AgentTurn {
    /// 給 UI / 使用者看的最終回應(自然語言)
    pub response: String,
    /// 這輪有哪些 skill 被呼叫(供 log / debug / UI 顯示)
    pub skill_calls: Vec<SkillCallRecord>,
}

#[derive(Debug, Clone)]
pub struct SkillCallRecord {
    pub name: String,
    pub args: serde_json::Value,
    pub output: SkillOutput,
}

pub struct Agent {
    provider: Arc<dyn LlmProvider>,
    skills: Arc<SkillRegistry>,
}

impl Agent {
    pub fn new(provider: Arc<dyn LlmProvider>, skills: Arc<SkillRegistry>) -> Self {
        Self { provider, skills }
    }

    /// 跑一輪:給定 system prompt + 使用者輸入 → 拿到 Mori 的回覆。
    pub async fn respond(
        &self,
        system_prompt: &str,
        user_input: &str,
        ctx: &Context,
    ) -> Result<AgentTurn> {
        let messages = vec![
            ChatMessage {
                role: "system".into(),
                content: system_prompt.to_string(),
            },
            ChatMessage {
                role: "user".into(),
                content: user_input.to_string(),
            },
        ];
        let tools = self.skills.tool_definitions();
        tracing::debug!(
            tool_count = tools.len(),
            tool_names = ?tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            "agent: chat with tools"
        );

        let chat = self
            .provider
            .chat(messages, tools)
            .await
            .context("provider chat")?;

        let mut skill_calls = Vec::new();
        let mut skill_messages = Vec::new();

        for tc in chat.tool_calls {
            match self.skills.dispatch(&tc.name, tc.arguments.clone(), ctx).await {
                Ok(output) => {
                    tracing::info!(
                        skill = %tc.name,
                        msg = %output.user_message,
                        "skill executed"
                    );
                    skill_messages.push(output.user_message.clone());
                    skill_calls.push(SkillCallRecord {
                        name: tc.name,
                        args: tc.arguments,
                        output,
                    });
                }
                Err(e) => {
                    tracing::error!(skill = %tc.name, ?e, "skill failed");
                    skill_messages.push(format!("(skill {} 出錯:{e})", tc.name));
                }
            }
        }

        let response = match (chat.content, skill_messages.is_empty()) {
            // 既有 LLM 自然語言、也有 skill 結果 → 都顯示
            (Some(content), false) if !content.trim().is_empty() => {
                format!("{}\n\n{}", content.trim(), skill_messages.join("\n"))
            }
            // 只有 LLM 自然語言
            (Some(content), true) if !content.trim().is_empty() => content,
            // 只有 skill 結果
            (_, false) => skill_messages.join("\n"),
            // 空空如也(罕見,防禦性處理)
            _ => String::new(),
        };

        Ok(AgentTurn {
            response,
            skill_calls,
        })
    }
}
