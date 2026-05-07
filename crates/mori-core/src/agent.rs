//! Agent 迴圈 — 把 LLM、Skill registry、Memory store 拼成 Mori 的「大腦」。
//!
//! Phase 1E:多輪 tool calling 迴圈。
//!
//! ```text
//!   user input
//!      │
//!      ▼
//!   loop (max N rounds):
//!     provider.chat(messages, tools)
//!        │
//!        ├─ 沒 tool_calls → 回傳 content 當 final response
//!        │
//!        └─ 有 tool_calls → execute each via SkillRegistry,
//!                          把 assistant message + tool result messages
//!                          append 進 messages,迴圈繼續(LLM 看到結果再答)
//! ```
//!
//! Tool 結果回傳給 LLM 後,LLM 自然語言整合 → 給使用者最終回應。
//! 例如:RecallMemorySkill 把 memory body 餵回 → LLM 用記憶內容答話。

use std::sync::Arc;

use anyhow::{Context as _, Result};

use crate::context::Context;
use crate::llm::{ChatMessage, LlmProvider};
use crate::skill::{SkillOutput, SkillRegistry};

const MAX_ROUNDS: usize = 5;

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

    /// 跑一輪互動。多輪 tool call 迴圈最多 [`MAX_ROUNDS`] 次,超過視為異常。
    pub async fn respond(
        &self,
        system_prompt: &str,
        user_input: &str,
        ctx: &Context,
    ) -> Result<AgentTurn> {
        let mut messages = vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user(user_input),
        ];
        let tools = self.skills.tool_definitions();
        let mut all_skill_calls: Vec<SkillCallRecord> = Vec::new();

        tracing::debug!(
            tool_count = tools.len(),
            tool_names = ?tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            "agent: starting multi-turn loop"
        );

        for round in 0..MAX_ROUNDS {
            let chat = self
                .provider
                .chat(messages.clone(), tools.clone())
                .await
                .with_context(|| format!("provider chat (round {round})"))?;

            // No tool calls → final answer
            if chat.tool_calls.is_empty() {
                let response = chat.content.unwrap_or_default();
                tracing::info!(
                    round,
                    chars = response.chars().count(),
                    "agent: final response"
                );
                return Ok(AgentTurn {
                    response,
                    skill_calls: all_skill_calls,
                });
            }

            tracing::info!(
                round,
                n_tools = chat.tool_calls.len(),
                names = ?chat.tool_calls.iter().map(|tc| tc.name.as_str()).collect::<Vec<_>>(),
                "agent: tool calls received"
            );

            // 把 assistant 的決定 echo 回 messages(OpenAI 協定要求)
            messages.push(ChatMessage::assistant_with_tool_calls(
                chat.content.clone(),
                chat.tool_calls.clone(),
            ));

            // Execute each tool, append tool result message
            for tc in &chat.tool_calls {
                let exec = self
                    .skills
                    .dispatch(&tc.name, tc.arguments.clone(), ctx)
                    .await;
                let result_text = match exec {
                    Ok(output) => {
                        let text = output.user_message.clone();
                        all_skill_calls.push(SkillCallRecord {
                            name: tc.name.clone(),
                            args: tc.arguments.clone(),
                            output,
                        });
                        text
                    }
                    Err(e) => {
                        tracing::error!(skill = %tc.name, ?e, "skill failed");
                        let text = format!("(error: {e})");
                        all_skill_calls.push(SkillCallRecord {
                            name: tc.name.clone(),
                            args: tc.arguments.clone(),
                            output: SkillOutput {
                                user_message: text.clone(),
                                data: None,
                            },
                        });
                        text
                    }
                };
                messages.push(ChatMessage::tool_result(
                    tc.id.clone(),
                    tc.name.clone(),
                    result_text,
                ));
            }
        }

        // Hit MAX_ROUNDS — 防無限迴圈,但也代表 LLM 行為怪異
        anyhow::bail!(
            "agent exceeded max tool-call rounds ({MAX_ROUNDS}) — LLM may be looping"
        )
    }
}
