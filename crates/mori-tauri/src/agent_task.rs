//! 排程 agent 任務的執行端：到點由時之鳥叫進來，Mori 用「唯讀」工具跑任務指令，
//! 有值得報告的結果才跳桌面通知。
//!
//! 唯讀是結構上保證的：這裡自己組一份只有 `list_gmail`（只回沒報告過的）和
//! `read_gmail` 的 registry，根本沒有 `send_gmail`，LLM 想寄也寄不出去。

use std::sync::Arc;

use mori_core::agent::Agent;
use mori_core::context::Context;
use mori_core::llm::Routing;
use mori_core::skill::{NewOnlyListGmailSkill, ReadGmailSkill, SharedGmailClient, SkillRegistry};
use mori_time::{Notifier, Reminder};
use tauri::{AppHandle, Manager};

const SYSTEM_PROMPT: &str = "你是 Mori，正在背景執行使用者設定的定時任務，沒有人在等你回話。\
你只有唯讀工具，不能寄信或改任何東西。照任務指令做完後，用繁體中文、一兩句話回報重點。\
如果沒有值得回報的事（例如沒有新信），只回 NOTHING 這個字，不要多說。\
列信箱的結果已含寄件摘要，通常不必逐封 read_gmail；最多讀 2 封。\
有多封新信時，每封各一行（寄件人＋主旨重點），不要只挑一封。";

/// 沒有結果要報告的暗號（LLM 回這個就不通知）。
const NOTHING: &str = "NOTHING";

pub async fn run(app: AppHandle, reminder: Reminder) {
    let Some(prompt) = reminder.agent_prompt.clone() else {
        return;
    };
    let outcome = run_inner(&app, &reminder, &prompt).await;
    let (kind, detail) = match &outcome {
        Ok(Some(text)) => ("notified", text.clone()),
        Ok(None) => ("nothing_to_report", String::new()),
        Err(e) => ("error", format!("{e:#}")),
    };
    mori_core::event_log::append(serde_json::json!({
        "kind": "agent_task_run",
        "reminder_id": reminder.id,
        "result": kind,
        "detail": detail,
    }));
    match outcome {
        Ok(Some(text)) => notify(&reminder.text, &text).await,
        Ok(None) => tracing::info!(reminder_id = reminder.id, "agent task: nothing to report"),
        Err(e) => tracing::warn!(reminder_id = reminder.id, error = %format!("{e:#}"), "agent task failed"),
    }
}

async fn run_inner(app: &AppHandle, reminder: &Reminder, prompt: &str) -> anyhow::Result<Option<String>> {
    let gmail = app
        .try_state::<SharedGmailClient>()
        .ok_or_else(|| anyhow::anyhow!("Gmail 尚未授權（~/.mori/gmail-token.json 不存在）"))?;
    let client = gmail.0.clone();

    let mut registry = SkillRegistry::new();
    let list_skill = Arc::new(NewOnlyListGmailSkill::new(
        client.clone(),
        crate::mori_dir().join(format!("agent-task-{}-seen.json", reminder.id)),
    ));
    registry.register(list_skill.clone());
    registry.register(Arc::new(ReadGmailSkill::new(client)));

    let routing = Routing::build_from_config(None)?;
    // bash-cli-agent 會自己 spawn 帶全套工具的 AI CLI，繞過上面的唯讀 registry，不能用在這裡。
    if routing.agent.name() == "bash-cli-agent" {
        anyhow::bail!("agent provider 是 bash-cli-agent，無法保證唯讀，排程任務不執行");
    }

    let turn = Agent::new(routing.agent.clone(), Arc::new(registry))
        .respond(SYSTEM_PROMPT, &[], prompt, &Context::default())
        .await?;
    list_skill.commit(); // 只有整輪成功才把列到的信記成已看過
    let text = turn.response.trim();
    Ok(if text.is_empty() || text.contains(NOTHING) { None } else { Some(text.to_string()) })
}

async fn notify(title: &str, body: &str) {
    let cfg = crate::notification_config::NotificationConfig::load(&crate::mori_dir().join("config.json"));
    if !cfg.os_notification_enabled {
        return;
    }
    let (title, body) = (title.to_string(), body.to_string());
    // notify-rust 在 Linux 自己開 runtime，必須走 spawn_blocking（同 mori-time on_fire）。
    let res = tokio::task::spawn_blocking(move || Notifier::new("Mori").notify(&title, &body)).await;
    if !matches!(res, Ok(Ok(()))) {
        tracing::warn!(?res, "agent task notification failed");
    }
}
