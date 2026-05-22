//! RemindMeCronSkill — LLM 可呼叫的「週期性提醒」工具,接 `mori_time::ReminderService`。
//!
//! # 為什麼存在
//!
//! [`RemindMeSkill`](super::RemindMeSkill) 只設一次性 reminder(NL → due_at)。
//! 但 user 常需要「每天 8 點」「每週一」「每月 1 號」這種 cron-style 週期提醒。
//! mori-time `ReminderService::remind_me_cron` 已有實作,缺一個 LLM 入口。
//!
//! # 設計選擇:LLM 直接給 6-field cron string
//!
//! 兩條路:
//! 1. **Rust NL → cron parser**:寫表驅動規則,確定 / 覆蓋有限。
//! 2. **LLM 自己 generate cron**:Mori 本來就有 LLM 主腦,讓它 NL → cron 是最自然的。
//!    System prompt 用 few-shot 範例教格式,skill 端只 validate。**選 2**。
//!
//! 風險:LLM 偶爾 generate 錯 cron(週日 = 0 還是 7、second 欄寫不寫等)。Skill 端
//! 接 `ReminderService::remind_me_cron`,scheduler 拒接 invalid cron 會回 clear
//! error,LLM 看 error message 自己修正後重試。
//!
//! # 行為
//!
//! - 吃 `{"text": "<提醒內容>", "cron": "<6-field cron>"}` 參數
//! - 呼叫 `ReminderService::remind_me_cron(text, cron)` 建 cron reminder + 排程
//! - 成功:`user_message = 「好,我會依排程 <cron> 提醒你 <text>」`
//! - 失敗:`anyhow!` 上拋(LLM 收到 error 再試)
//!
//! # Cron 格式
//!
//! 6-field(秒在前):`sec min hour day month weekday`
//! - 每天 8 點:`0 0 8 * * *`
//! - 每週一 9 點:`0 0 9 * * 1`(週日 = 0 = 7)
//! - 每月 1 號 0 點:`0 0 0 1 * *`
//! - 每 30 分:`0 */30 * * * *`

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;

use mori_time::ReminderService;

use super::{ExecutionTarget, Privacy, Skill, SkillOutput};
use crate::context::Context;

pub struct RemindMeCronSkill {
    service: Arc<ReminderService>,
}

impl RemindMeCronSkill {
    pub fn new(service: Arc<ReminderService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl Skill for RemindMeCronSkill {
    fn name(&self) -> &'static str {
        "remind_me_cron"
    }

    fn description(&self) -> &'static str {
        "設一個週期性提醒(cron schedule)。給 text(提醒內容)+ cron(6-field cron expression \
         以秒為首欄,格式:'sec min hour day month weekday')。範例:'0 0 8 * * *' = 每天 \
         早上 8 點、'0 0 9 * * 1' = 每週一 9 點、'0 30 14 * * 0,6' = 每週末 14:30、'0 0 0 1 * *' \
         = 每月 1 號 0 點。weekday 用 0-6(週日 = 0)或 1-7(週日 = 7)。一次性提醒用 \
         `remind_me` 不要用這個。"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "要提醒的內容(短句)。例:「喝水」、「站起來走走」、「會議檢查」。"
                },
                "cron": {
                    "type": "string",
                    "description": "6-field cron expression,格式 'sec min hour day month weekday'。範例:每天 8 點 = '0 0 8 * * *',每週一 9 點 = '0 0 9 * * 1',每月 1 號 = '0 0 0 1 * *',每 30 分 = '0 */30 * * * *'。"
                }
            },
            "required": ["text", "cron"]
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

        let cron = args
            .get("cron")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing cron"))?
            .trim()
            .to_string();
        if cron.is_empty() {
            return Err(anyhow!("cron is empty"));
        }

        tracing::info!(text = %text, cron = %cron, "remind_me_cron skill");

        let reminder = self
            .service
            .remind_me_cron(text.clone(), cron.clone())
            .await
            .map_err(|e| anyhow!("remind_me_cron failed: {e}"))?;

        let user_message = format!(
            "好,我會依排程「{}」提醒你「{}」。",
            cron, reminder.text,
        );

        Ok(SkillOutput {
            user_message,
            data: Some(serde_json::json!({
                "id": reminder.id,
                "text": reminder.text,
                "cron_expr": cron,
            })),
        })
    }
}

#[cfg(test)]
mod tests {
    //! 走真的 ReminderService(`tempfile` DB + Notifier::disabled),驗證 skill dispatch
    //! 路徑活著 + arg / cron 驗證。

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
            // 2026-05-23:popup PR 合進 main 後 ReminderService::new 變 3-param
            // (加 EventEmitter trait);test 用 Notifier::disabled 避免發真實 OS 通知 +
            // NoopEmitter 避免 in-app popup emit。
            ReminderService::new(&db, Notifier::disabled("Mori-Test"), Arc::new(NoopEmitter))
                .await
                .expect("new service"),
        )
    }

    #[tokio::test]
    async fn remind_me_cron_skill_returns_ok_for_valid_cron() {
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        let skill = RemindMeCronSkill::new(svc);
        // 每天 8 點
        let args = serde_json::json!({
            "text": "喝水",
            "cron": "0 0 8 * * *",
        });
        let out = skill
            .execute(args, &empty_context())
            .await
            .expect("valid cron should succeed");
        assert!(out.user_message.contains("喝水"));
        assert!(out.user_message.contains("0 0 8"));
        let data = out.data.expect("data present");
        assert!(data["id"].as_i64().unwrap() > 0);
        assert_eq!(data["text"], "喝水");
        assert_eq!(data["cron_expr"], "0 0 8 * * *");
    }

    #[tokio::test]
    async fn remind_me_cron_skill_returns_error_for_invalid_cron() {
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        let skill = RemindMeCronSkill::new(svc);
        // 5-field 不是 6-field,scheduler 應該拒接
        let args = serde_json::json!({
            "text": "X",
            "cron": "0 8 * * *",
        });
        let err = skill
            .execute(args, &empty_context())
            .await
            .expect_err("invalid cron should fail");
        // remind_me_cron failed: scheduler: ... cron expr '0 8 * * *': ...
        let msg = err.to_string();
        assert!(
            msg.contains("cron") || msg.contains("scheduler"),
            "expected cron / scheduler error, got: {msg}",
        );
    }

    #[tokio::test]
    async fn remind_me_cron_skill_returns_error_for_missing_args() {
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        let skill = RemindMeCronSkill::new(svc);

        // 缺 text
        let err = skill
            .execute(serde_json::json!({"cron": "0 0 8 * * *"}), &empty_context())
            .await
            .expect_err("missing text");
        assert!(err.to_string().contains("text"));

        // 缺 cron
        let err2 = skill
            .execute(serde_json::json!({"text": "X"}), &empty_context())
            .await
            .expect_err("missing cron");
        assert!(err2.to_string().contains("cron"));
    }

    #[tokio::test]
    async fn remind_me_cron_skill_name_and_description_present() {
        let dir = TempDir::new().unwrap();
        let svc = service_in(&dir).await;
        let skill = RemindMeCronSkill::new(svc);
        assert_eq!(skill.name(), "remind_me_cron");
        let desc = skill.description();
        assert!(!desc.is_empty());
        assert!(desc.contains("cron") || desc.contains("週期"));
        let schema = skill.parameters_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "text"));
        assert!(required.iter().any(|v| v == "cron"));
    }
}
