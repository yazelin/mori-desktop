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
use mori_core::memory::MemoryStore;
use mori_core::runtime::{generate_auth_token, RuntimeInfo};
use mori_core::skill::{
    ComposeSkill, EditMemorySkill, ForgetMemorySkill, PolishSkill, RecallMemorySkill,
    RememberSkill, SkillRegistry, SummarizeSkill, TranslateSkill,
};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct SkillServerState {
    pub auth_token: Arc<str>,
    pub memory: Arc<dyn MemoryStore>,
}

pub async fn start(memory: Arc<dyn MemoryStore>) -> Result<RuntimeInfo> {
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
        memory,
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

/// 依當前 Agent profile build 一個臨時 SkillRegistry：
/// - Built-in（純 LLM）skill: translate / polish / summarize / compose
/// - Memory skill: remember / recall_memory / forget_memory / edit_memory
/// - Action skill (Linux): open_url / open_app / send_keys / google_search /
///   ask_chatgpt / ask_gemini / find_youtube
/// - Shell skill: 來自 active agent profile 的 `shell_skills:` 定義
///
/// 注意：set_mode / paste_selection_back 等 stateful skill 不註冊到 HTTP 入口，
/// 它們有 AppHandle / state 依賴，且通常不適合外部觸發。
fn build_dynamic_registry(state: &SkillServerState) -> Result<SkillRegistry> {
    let routing = mori_core::llm::Routing::build_from_config(None)
        .context("build routing for skill_server dynamic registry")?;
    let memory = state.memory.clone();
    let mut registry = SkillRegistry::new();

    // Built-in 純 LLM skill
    registry.register(Arc::new(TranslateSkill::new(routing.skill_provider("translate"))));
    registry.register(Arc::new(PolishSkill::new(routing.skill_provider("polish"))));
    registry.register(Arc::new(SummarizeSkill::new(routing.skill_provider("summarize"))));
    registry.register(Arc::new(ComposeSkill::new(routing.skill_provider("compose"))));

    // Memory skills
    registry.register(Arc::new(RememberSkill::new(memory.clone())));
    registry.register(Arc::new(RecallMemorySkill::new(memory.clone())));
    registry.register(Arc::new(ForgetMemorySkill::new(memory.clone())));
    registry.register(Arc::new(EditMemorySkill::new(memory.clone())));

    // 5G-6: action skills (Linux only)
    #[cfg(target_os = "linux")]
    {
        registry.register(Arc::new(crate::action_skills::OpenUrlSkill));
        registry.register(Arc::new(crate::action_skills::OpenAppSkill));
        registry.register(Arc::new(crate::action_skills::SendKeysSkill));
        registry.register(Arc::new(crate::action_skills::GoogleSearchSkill));
        registry.register(Arc::new(crate::action_skills::AskChatGptSkill));
        registry.register(Arc::new(crate::action_skills::AskGeminiSkill));
        registry.register(Arc::new(crate::action_skills::FindYoutubeSkill));
    }

    // 5H: 當前 agent profile 的 shell skills
    let profile = mori_core::agent_profile::load_active_agent_profile();
    for def in &profile.frontmatter.shell_skills {
        registry.register(Arc::new(crate::shell_skill::ShellSkill::new(def.clone())));
    }

    Ok(registry)
}

async fn list_skills(
    State(state): State<SkillServerState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, String)> {
    check_auth(&headers, &state.auth_token)?;

    let registry = build_dynamic_registry(&state).map_err(|e| {
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

    let registry = build_dynamic_registry(&state).map_err(|e| {
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
    let ctx = MoriContext::default();
    match skill.execute(args, &ctx).await {
        Ok(out) => Ok(out.user_message),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{}: {e:#}", name),
        )),
    }
}
