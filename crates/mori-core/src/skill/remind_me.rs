//! RemindMeSkill — LLM 可呼叫的「設提醒」工具,接 `mori_time::ReminderService`。
//!
//! # 為什麼存在
//!
//! 「時之鳥」整套 reminder runtime(`mori-time` crate)已經有 [`ReminderService`]
//! 把 K1-K4(schema / scheduler / notifier / parser)整合好,Tauri 端 main.rs 也
//! 在啟動時建一個 `Arc<ReminderService>` 註冊進 AppState。但 LLM 看 system prompt
//! 知道有 `remind_me` 工具卻沒有實作路徑 — 走 LLM tool dispatch 的入口是
//! [`SkillRegistry::dispatch`](super::SkillRegistry::dispatch),必須對應一個
//! [`Skill`] impl。這個 skill 就是那個缺口。
//!
//! # 行為
//!
//! - 吃 `{"text": "<提醒內容>", "when": "<NL 時間>"}` 參數
//! - 呼叫 `ReminderService::remind_me(text, when)` 解析時間 + 建 reminder + 排程
//! - 成功:`SkillOutput.user_message` = 「好,我會在 <時間> 提醒你 <text>」,
//!   `data` 帶 `{ id, text, due_at }`
//! - 失敗(時間 unrecognized / 過去時間 / store 寫入失敗等)→ `anyhow!` 往上拋,
//!   LLM 會把 error message 講給 user 聽,user 改說法重試。
//!
//! # 平台 / 隱私
//!
//! Reminder 寫本機 SQLite + 走本機 desktop 通知 → `ExecutionTarget::Local`。
//! 但 reminder text 可能含 user 私訊內容,並不限制 LocalOnly(這是 LLM tool
//! call 的 user_message;user 自己選 cloud LLM 時 text 本來就會送上去)。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;

use mori_time::ReminderService;

use super::{ExecutionTarget, Privacy, Skill, SkillOutput};
use crate::context::Context;

pub struct RemindMeSkill {
    service: Arc<ReminderService>,
}

impl RemindMeSkill {
    pub fn new(service: Arc<ReminderService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl Skill for RemindMeSkill {
    fn name(&self) -> &'static str {
        "remind_me"
    }

    fn description(&self) -> &'static str {
        "設一個提醒。給 text(要提醒的內容)+ when(中/英文自然語言時間,例如「30 分鐘後」\
         「明天 9 點」「6pm」「tomorrow 9am」)。到時間會跳桌面通知。解析失敗 / 過去時間\
         會回 error,user 需改說法重試。"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "要提醒的內容(短句)。例:「打電話給媽」、「喝水」、「會議開始」。"
                },
                "when": {
                    "type": "string",
                    "description": "中/英文自然語言時間。中文支援「30 分鐘後」「明天 9 點」「下午 3 點」「下週一」;英文支援「30 minutes」「tomorrow 9am」「6pm」「next mon」。"
                }
            },
            "required": ["text", "when"]
        })
    }

    fn target_capability(&self) -> ExecutionTarget {
        ExecutionTarget::Local
    }

    fn privacy(&self) -> Privacy {
        Privacy::Cloud
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing text"))?
            .trim()
            .to_string();
        if text.is_empty() {
            return Err(anyhow!("text is empty"));
        }

        let when = args
            .get("when")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing when"))?
            .trim()
            .to_string();
        if when.is_empty() {
            return Err(anyhow!("when is empty"));
        }

        tracing::info!(text = %text, when = %when, "remind_me skill");

        let reminder = self
            .service
            .remind_me(text.clone(), when.clone())
            .await
            .map_err(|e| anyhow!("remind_me failed: {e}"))?;

        // user_message 用 due_at 的本地時間表示比 UTC 友善
        let due_local = reminder.due_at.with_timezone(&chrono::Local);
        let user_message = format!(
            "好,我會在 {} 提醒你「{}」。",
            due_local.format("%Y-%m-%d %H:%M"),
            reminder.text,
        );

        Ok(SkillOutput {
            user_message,
            data: Some(serde_json::json!({
                "id": reminder.id,
                "text": reminder.text,
                "due_at": reminder.due_at.to_rfc3339(),
            })),
        })
    }
}

#[cfg(test)]
mod tests {
    //! 走真的 ReminderService(`tempfile` DB + Notifier::new),驗證 skill dispatch
    //! 路徑活著 + arg 驗證。Tauri runtime 不 mock。

    use super::*;
    use crate::context::Context;
    use mori_time::{NoopEmitter, Notifier};
    use tempfile::TempDir;

    fn empty_context() -> Context {
        Context::default()
    }

    async fn service_in(dir: &TempDir) -> Arc<ReminderService> {
        let db = dir.path().join("reminders.db");
        Arc::new(
            ReminderService::new(&db, Notifier::disabled("Mori-Test"), Arc::new(NoopEmitter))
                .await
                .expect("new service"),
        )
    }

    #[tokio::test]
    async fn remind_me_skill_returns_ok_for_valid_input() {
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        let skill = RemindMeSkill::new(svc);
        let args = serde_json::json!({
            "text": "喝水",
            "when": "30 minutes",
        });
        let out = skill
            .execute(args, &empty_context())
            .await
            .expect("remind_me should succeed");
        assert!(out.user_message.contains("喝水"));
        let data = out.data.expect("data present");
        assert!(data["id"].as_i64().unwrap() > 0);
        assert_eq!(data["text"], "喝水");
        assert!(data["due_at"].is_string());
    }

    #[tokio::test]
    async fn remind_me_skill_returns_error_for_missing_args() {
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        let skill = RemindMeSkill::new(svc);

        // 缺 text
        let err = skill
            .execute(serde_json::json!({"when": "30 minutes"}), &empty_context())
            .await
            .expect_err("missing text");
        assert!(err.to_string().contains("text"));

        // 缺 when
        let err2 = skill
            .execute(serde_json::json!({"text": "喝水"}), &empty_context())
            .await
            .expect_err("missing when");
        assert!(err2.to_string().contains("when"));
    }

    #[tokio::test]
    async fn remind_me_skill_returns_error_for_unparseable_time() {
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        let skill = RemindMeSkill::new(svc);
        let args = serde_json::json!({
            "text": "X",
            "when": "xyzzy qwerty foobar",
        });
        let err = skill
            .execute(args, &empty_context())
            .await
            .expect_err("unparseable time should fail");
        // remind_me failed: parse time: can't parse time expression: ...
        let msg = err.to_string();
        assert!(
            msg.contains("parse") || msg.contains("can't"),
            "expected parse error, got: {msg}",
        );
    }

    #[tokio::test]
    async fn remind_me_skill_name_and_description_present() {
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        let skill = RemindMeSkill::new(svc);
        assert_eq!(skill.name(), "remind_me");
        let desc = skill.description();
        assert!(!desc.is_empty());
        assert!(desc.contains("提醒") || desc.contains("remind"));
        let schema = skill.parameters_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "text"));
        assert!(required.iter().any(|v| v == "when"));
    }
}
