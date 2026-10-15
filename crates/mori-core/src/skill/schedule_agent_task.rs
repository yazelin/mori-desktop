//! ScheduleAgentTaskSkill — 讓 LLM 建立「定時叫 Mori 自己做一件事」的任務。
//!
//! 跟 [`RemindMeCronSkill`](super::RemindMeCronSkill) 的差別：提醒到點只跳通知；
//! 這個到點會叫 agent 用**唯讀工具**（目前是 Gmail 列信／讀信）跑 `prompt`，
//! 有結果才跳桌面通知。例：每 5 分鐘看信箱、有新信就摘要給我。
//! 執行端在 mori-tauri（`agent_task.rs`），這裡只負責建立任務。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;

use mori_time::ReminderService;

use super::{ExecutionTarget, Privacy, Skill, SkillOutput};
use crate::context::Context;

pub struct ScheduleAgentTaskSkill {
    service: Arc<ReminderService>,
}

impl ScheduleAgentTaskSkill {
    pub fn new(service: Arc<ReminderService>) -> Self {
        Self { service }
    }
}

fn required_str(args: &Value, key: &str) -> Result<String> {
    let v = args
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("missing {key}"))?
        .trim()
        .to_string();
    if v.is_empty() {
        return Err(anyhow!("{key} is empty"));
    }
    Ok(v)
}

#[async_trait]
impl Skill for ScheduleAgentTaskSkill {
    fn name(&self) -> &'static str {
        "schedule_agent_task"
    }

    fn description(&self) -> &'static str {
        "建立定時任務：依 cron 到點，由 Mori 自己用唯讀工具（目前只能列信、讀信，不能寄信）\
         執行 prompt，有值得報告的結果才跳桌面通知。用於「每 5 分鐘看一次信箱，有新信告訴我」\
         這類要主動檢查的事。只要到點說一句話的提醒用 remind_me_cron，不要用這個。\
         cron 是 6-field（秒在前）：每 5 分 = '0 */5 * * * *'、每天 8 點 = '0 0 8 * * *'。"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "任務名稱（短句），例：「看信箱」。"
                },
                "cron": {
                    "type": "string",
                    "description": "6-field cron expression 'sec min hour day month weekday'。每 5 分鐘 = '0 */5 * * * *'。"
                },
                "prompt": {
                    "type": "string",
                    "description": "到點要 Mori 做的事，寫成完整指令。例：「看信箱有沒有新信，有的話用一兩句話摘要寄件人與重點；沒有新信就什麼都不要說。」"
                }
            },
            "required": ["text", "cron", "prompt"]
        })
    }

    fn target_capability(&self) -> ExecutionTarget {
        ExecutionTarget::Local
    }

    fn privacy(&self) -> Privacy {
        Privacy::Cloud
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let text = required_str(&args, "text")?;
        let cron = required_str(&args, "cron")?;
        let prompt = required_str(&args, "prompt")?;

        tracing::info!(text = %text, cron = %cron, "schedule_agent_task skill");

        let r = self
            .service
            .schedule_agent_task(text.clone(), cron.clone(), prompt)
            .await
            .map_err(|e| anyhow!("schedule_agent_task failed: {e}"))?;

        Ok(SkillOutput {
            user_message: format!("好，我會依排程「{cron}」幫你「{}」，有結果再通知你。", r.text),
            data: Some(serde_json::json!({ "id": r.id, "text": r.text, "cron_expr": cron })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mori_time::{NoopEmitter, Notifier};
    use tempfile::TempDir;

    async fn skill_in(dir: &TempDir) -> ScheduleAgentTaskSkill {
        let svc = ReminderService::new(
            &dir.path().join("reminders.db"),
            Notifier::disabled("Mori-Test"),
            Arc::new(NoopEmitter),
        )
        .await
        .expect("new service");
        ScheduleAgentTaskSkill::new(Arc::new(svc))
    }

    #[tokio::test]
    async fn creates_agent_task_and_rejects_bad_input() {
        let dir = TempDir::new().unwrap();
        let skill = skill_in(&dir).await;
        let ctx = Context::default();

        let ok = skill
            .execute(
                serde_json::json!({"text": "看信箱", "cron": "0 */5 * * * *", "prompt": "看有沒有新信"}),
                &ctx,
            )
            .await
            .expect("valid");
        assert!(ok.data.unwrap()["id"].as_i64().unwrap() > 0);

        // 缺 prompt、5-field cron 都要擋
        assert!(skill
            .execute(serde_json::json!({"text": "x", "cron": "0 */5 * * * *"}), &ctx)
            .await
            .unwrap_err()
            .to_string()
            .contains("prompt"));
        assert!(skill
            .execute(serde_json::json!({"text": "x", "cron": "*/5 * * * *", "prompt": "y"}), &ctx)
            .await
            .is_err());
    }
}
