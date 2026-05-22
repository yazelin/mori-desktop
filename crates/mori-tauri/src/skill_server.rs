//! Phase 5D + 5I — Local HTTP server 暴露當前 Agent profile 所有可用 skill。
//!
//! ## 5I 動態化（架構升級）
//!
//! 原本 skill_server 寫死 8 個 skill（translate / polish / summarize / compose +
//! 4 個 memory），claude-bash / gemini-bash / codex-bash 透過 mori CLI 看到的
//! 永遠是這 8 個。5G/5H 新增的 action_skills（open_url / send_keys / 等）和
//! shell_skills（per-profile CLI 包裝）都看不到。
//!
//! 5I 改成「每次 request 即時讀當前 Agent profile，build SkillRegistry」：
//! - GET /skill/list      → 列出當前 profile 所有 skill（含動態的）+ JSON schema
//! - POST /skill/<name>   → 即時從 registry dispatch（找不到才 404）
//!
//! 對 LLM（claude/gemini/codex）視角，工具集會隨使用者按 Ctrl+Alt+N 切 profile
//! 改變，但 mori CLI 介面不變。
//!
//! ## 流程
//! 啟動時 bind 127.0.0.1:0 拿空閒 port → 產 32-char auth token →
//! 寫到 `~/.mori/runtime.json` → spawn axum 任務 listen。
//!
//! 對外暴露:
//! - `GET  /skill/list`              列出當前 profile 的 skill name + description + schema
//! - `POST /skill/<name>` body=JSON  執行 skill,回傳 user_message 純文字
//!
//! 都要帶 `Authorization: Bearer <token>` header(token 從 runtime.json 拿)。

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use mori_core::context::Context as MoriContext;
use mori_core::runtime::{generate_auth_token, RuntimeInfo};
use mori_core::skill::{
    ComposeSkill, EditMemorySkill, ForgetMemorySkill, ListGmailSkill, PolishSkill, ReadFileSkill,
    ReadGmailSkill, ReadWikiPageSkill, RecallMemorySkill, RememberSkill, RemindMeCronSkill, RemindMeSkill,
    SendGmailSkill, SharedGmailClient, SkillRegistry, SummarizeSkill, TranslateSkill,
};
use mori_time::ReminderService;
use serde_json::{json, Value};
use tauri::Manager;

#[derive(Clone)]
pub struct SkillServerState {
    pub auth_token: Arc<str>,
    /// C:不直接持 Arc<dyn MemoryStore>(會被 hot-reload swap 後變 stale),
    /// 持 Arc<AppState>,每次 handler 透過 `app.memory_handle()` 拿當下 snapshot。
    pub app: Arc<crate::AppState>,
    /// Tauri AppHandle — 從 Manager 拿 stateful service(ReminderService /
    /// SharedGmailClient / McpRegistry)。沒這個就只能掛純 functional skill,
    /// 時之鳥 / Gmail / MCP tool 全進不來。
    pub app_handle: tauri::AppHandle,
}

pub async fn start(
    app: Arc<crate::AppState>,
    app_handle: tauri::AppHandle,
) -> Result<RuntimeInfo> {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .context("bind 127.0.0.1:random")?;
    let local = listener.local_addr().context("get local addr")?;
    let port = local.port();
    let token = generate_auth_token();
    let started_at_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let info = RuntimeInfo {
        port,
        auth_token: token.clone(),
        pid: std::process::id(),
        started_at_epoch,
    };
    info.write_to_default()
        .context("write ~/.mori/runtime.json")?;

    let state = SkillServerState {
        auth_token: Arc::from(token.as_str()),
        app,
        app_handle,
    };
    let app = Router::new()
        .route("/skill/list", get(list_skills))
        .route("/skill/:name", post(dispatch_skill))
        .with_state(state);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(?e, "skill HTTP server crashed");
        }
    });

    let path_display = mori_core::runtime::RuntimeInfo::default_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(unknown)".to_string());
    tracing::info!(
        path = %path_display,
        port,
        "skill HTTP server ready"
    );
    Ok(info)
}

fn check_auth(headers: &HeaderMap, expected: &str) -> Result<(), (StatusCode, String)> {
    let got = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    match got {
        Some(token) if token == expected => Ok(()),
        Some(_) => Err((StatusCode::UNAUTHORIZED, "invalid token".into())),
        None => Err((StatusCode::UNAUTHORIZED, "missing Authorization header".into())),
    }
}

// ─── 5I: 動態 registry builder ──────────────────────────────────────────

/// 依當前 Agent profile build 一個臨時 SkillRegistry。內容必須對齊 main.rs
/// `skills_list` 那塊 — 否則 claude-bash / gemini-bash 等走 HTTP 拿 skill list
/// 的 provider 會看不到新 skill(LLM hallucinate「沒掛」)。
///
/// 包含:
/// - Built-in 純 LLM skill: translate / polish / summarize / compose / fetch_url
/// - 萬卷之口: read_file_text
/// - 時之鳥 K5: remind_me(需 ReminderService state)
/// - Gmail(跨界之手): list_gmail / read_gmail / send_gmail(需 SharedGmailClient state)
/// - 記憶之森: read_wiki_page(需 vault_root + spirit_name)
/// - Memory: remember / recall_memory / forget_memory / edit_memory
/// - Action skill: open_url / open_app / send_keys / google_search /
///   ask_chatgpt / ask_gemini / find_youtube
/// - Shell skill: active agent profile 的 `shell_skills:` 定義
/// - Stream I Anthropic SKILL.md: prompt + 可選 scripts
/// - Wave 6 MCP: 已連接 MCP server 的 tool
///
/// **故意排除**:set_mode / paste_selection_back 是 stateful + AppHandle 強耦合,
/// 不適合外部觸發。
async fn build_dynamic_registry(state: &SkillServerState) -> Result<SkillRegistry> {
    let routing = mori_core::llm::Routing::build_from_config(None)
        .context("build routing for skill_server dynamic registry")?;
    let memory = state.app.memory_handle();
    let app = &state.app_handle;
    let mut registry = SkillRegistry::new();

    // Built-in 純 LLM skill
    registry.register(Arc::new(TranslateSkill::new(routing.skill_provider("translate"))));
    registry.register(Arc::new(PolishSkill::new(routing.skill_provider("polish"))));
    registry.register(Arc::new(SummarizeSkill::new(routing.skill_provider("summarize"))));
    registry.register(Arc::new(ComposeSkill::new(routing.skill_provider("compose"))));
    registry.register(Arc::new(mori_core::skill::FetchUrlSkill::new()));

    // Stream E 萬卷之口
    registry.register(Arc::new(ReadFileSkill));

    // §9 P1 時之鳥 K5 — ReminderService 透過 Tauri Manager 拿。沒撈到就跳過
    // (LLM 拿不到 remind_me skill,但其他不會炸)。
    if let Some(svc) = app.try_state::<Arc<ReminderService>>() {
        registry.register(Arc::new(RemindMeSkill::new(svc.inner().clone())));
        // 2026-05-22:remind_me_cron 同 ReminderService 共用 — bash-cli-agent 也要看得到
        registry.register(Arc::new(RemindMeCronSkill::new(svc.inner().clone())));
    }

    // Wave 8 Gm-2 Gmail — optional state,沒設不註冊
    if let Some(shared) = app.try_state::<SharedGmailClient>() {
        let client = shared.0.clone();
        registry.register(Arc::new(ListGmailSkill::new(client.clone())));
        registry.register(Arc::new(ReadGmailSkill::new(client.clone())));
        registry.register(Arc::new(SendGmailSkill::new(client)));
    }

    // Wave 7 L-mori 記憶之森
    if let Some(vault_root) = crate::default_vault_root() {
        let cfg = crate::annuli_config::AnnuliConfig::load(&crate::mori_dir().join("config.json"));
        let spirit = if cfg.spirit_name.is_empty() {
            "mori".to_string()
        } else {
            cfg.spirit_name
        };
        registry.register(Arc::new(ReadWikiPageSkill::new(vault_root, spirit)));
    }

    // Memory skills
    registry.register(Arc::new(RememberSkill::new(memory.clone())));
    registry.register(Arc::new(RecallMemorySkill::new(memory.clone())));
    registry.register(Arc::new(ForgetMemorySkill::new(memory.clone())));
    registry.register(Arc::new(EditMemorySkill::new(memory.clone())));

    // 5G-6: action skills
    registry.register(Arc::new(crate::action_skills::OpenUrlSkill));
    registry.register(Arc::new(crate::action_skills::OpenAppSkill));
    registry.register(Arc::new(crate::action_skills::SendKeysSkill));
    registry.register(Arc::new(crate::action_skills::GoogleSearchSkill));
    registry.register(Arc::new(crate::action_skills::AskChatGptSkill));
    registry.register(Arc::new(crate::action_skills::AskGeminiSkill));
    registry.register(Arc::new(crate::action_skills::FindYoutubeSkill));

    // 5H: 當前 agent profile 的 shell skills
    let profile = mori_core::agent_profile::load_active_agent_profile();
    for def in &profile.frontmatter.shell_skills {
        registry.register(Arc::new(crate::shell_skill::ShellSkill::new(def.clone())));
    }

    // Stream I / Wave 6 DF-2: Anthropic SKILL.md(prompt + optional scripts)
    let anthropic_dir = mori_core::skill::anthropic_skill::default_skills_dir();
    for discovered in mori_core::skill::discover_anthropic_skills(&anthropic_dir) {
        let mori_core::skill::DiscoveredSkill {
            skill,
            scripts_dir,
        } = discovered;
        if let Some(sd) = scripts_dir {
            registry.register(Arc::new(mori_core::skill::AnthropicScriptSkill::new(
                skill.clone(),
                sd,
            )));
        }
        registry.register(Arc::new(mori_core::skill::AnthropicPromptSkill::new(skill)));
    }

    // Wave 6 MCP-2: 已連接 MCP server 提供的 tool。沒掛 MCP 就跳過。
    // 注意 all_tools() 是 async,所以 build_dynamic_registry 整個是 async。
    if let Some(mcp_reg) = app.try_state::<Arc<mori_mcp::McpRegistry>>() {
        let mcp_arc = mcp_reg.inner().clone();
        for tool in mcp_arc.all_tools().await {
            registry.register(Arc::new(mori_core::skill::McpToolSkill::new(
                mcp_arc.clone(),
                tool,
            )));
        }
    }

    Ok(registry)
}

async fn list_skills(
    State(state): State<SkillServerState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, String)> {
    check_auth(&headers, &state.auth_token)?;

    let registry = build_dynamic_registry(&state).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("build registry: {e:#}"),
        )
    })?;

    // tool_definitions() 已經是 OpenAI tool 格式：name + description + parameters schema
    let skills: Vec<Value> = registry
        .tool_definitions()
        .into_iter()
        .map(|td| {
            json!({
                "name": td.name,
                "description": td.description,
                "parameters": td.parameters,
            })
        })
        .collect();

    tracing::debug!(skill_count = skills.len(), "skill_server list_skills (dynamic)");
    Ok(Json(json!({ "skills": skills })))
}

async fn dispatch_skill(
    State(state): State<SkillServerState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(args): Json<Value>,
) -> Result<String, (StatusCode, String)> {
    check_auth(&headers, &state.auth_token)?;

    let registry = build_dynamic_registry(&state).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("build registry: {e:#}"),
        )
    })?;

    let skill = registry.get(&name).ok_or_else(|| {
        let available = registry.names().join(", ");
        (
            StatusCode::NOT_FOUND,
            format!("unknown skill: {name}\navailable: {available}"),
        )
    })?;

    tracing::info!(skill = %name, "skill dispatch via HTTP (dynamic registry)");
    // 為了診斷可見性,記下 args 摘要(避免巨大 payload 進 log;只取 first ~500 chars
    // 的 JSON serialization),skill 結果 ok/err + 文字長度都進 event_log。
    // 這樣 `mori skill call open_url` / shell_skill 出問題時,JSONL 一行就能定位:
    // 「skill 跑了沒、輸入是什麼、回什麼、ok 或 error」。
    let started_at = std::time::Instant::now();
    let args_preview = {
        let s = args.to_string();
        if s.len() > 500 { format!("{}…(truncated)", &s[..500]) } else { s }
    };
    let ctx = MoriContext::default();
    let result = skill.execute(args, &ctx).await;
    let latency_ms = started_at.elapsed().as_millis() as u64;
    match result {
        Ok(out) => {
            let preview = if out.user_message.chars().count() > 200 {
                format!("{}…", out.user_message.chars().take(200).collect::<String>())
            } else {
                out.user_message.clone()
            };
            mori_core::event_log::append(json!({
                "kind": "skill_dispatch",
                "skill": name,
                "args_preview": args_preview,
                "latency_ms": latency_ms,
                "ok": true,
                "user_message_chars": out.user_message.chars().count(),
                "user_message_preview": preview,
            }));
            Ok(out.user_message)
        }
        Err(e) => {
            let err_str = format!("{e:#}");
            mori_core::event_log::append(json!({
                "kind": "skill_dispatch",
                "skill": name,
                "args_preview": args_preview,
                "latency_ms": latency_ms,
                "ok": false,
                "error": err_str.clone(),
            }));
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("{}: {err_str}", name),
            ))
        }
    }
}
