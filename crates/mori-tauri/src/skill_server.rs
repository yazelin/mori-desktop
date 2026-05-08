//! Phase 5D — Local HTTP server 暴露 Mori 的 skills 給外部呼叫。
//!
//! ## 流程
//! 啟動時 bind 127.0.0.1:0 拿空閒 port → 產 32-char auth token →
//! 寫到 `~/.mori/runtime.json`(原子寫入)→ spawn axum 任務 listen。
//!
//! 對外暴露:
//! - `GET  /skill/list`              列出可用 skill name + description
//! - `POST /skill/<name>` body=JSON  執行 skill,回傳 user_message 純文字
//!
//! 都要帶 `Authorization: Bearer <token>` header(token 從 runtime.json 拿)。
//!
//! ## 為什麼不用 MCP
//! 跟 Mori 在 user prompt 裡解釋過 — Bash CLI proxy(這個 server)token
//! 成本遠低於 MCP(MCP 把所有 schema 預載到每輪 prompt;Bash 只在用到
//! 才執行)。Skills 內容 + 參數 schema 走 `mori skill <name> --help` 查
//! 到後就近執行,LLM 不必背全部。

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
use mori_core::skill::{ComposeSkill, PolishSkill, Skill, SummarizeSkill, TranslateSkill};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct SkillServerState {
    pub auth_token: Arc<str>,
}

/// 啟動 skill HTTP server。
/// 動作:
/// 1. bind 127.0.0.1:0
/// 2. 拿 OS 給的 port
/// 3. 產 token,寫 runtime.json
/// 4. spawn server task
///
/// 失敗會回 Err 但不該卡 Mori 啟動 — 呼叫端記 log 後繼續就好(只是
/// 失去 Bash CLI proxy 能力)。
pub async fn start() -> Result<RuntimeInfo> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind skill server to 127.0.0.1:0")?;
    let addr: SocketAddr = listener.local_addr().context("get bound addr")?;

    let token = generate_auth_token();
    let state = SkillServerState {
        auth_token: token.clone().into(),
    };

    let app = Router::new()
        .route("/skill/list", get(list_skills))
        .route("/skill/:name", post(dispatch_skill))
        .with_state(state);

    let info = RuntimeInfo {
        port: addr.port(),
        auth_token: token,
        pid: std::process::id(),
        started_at_epoch: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    };
    let path = info
        .write_to_default()
        .context("write runtime.json")?;
    tracing::info!(
        path = %path.display(),
        port = info.port,
        "skill HTTP server ready"
    );

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(?e, "skill HTTP server exited");
        }
    });

    Ok(info)
}

fn check_auth(headers: &HeaderMap, expected: &str) -> Result<(), (StatusCode, String)> {
    let header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token = header.strip_prefix("Bearer ").unwrap_or("");
    if token == expected {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            "missing or invalid Authorization: Bearer <token>".to_string(),
        ))
    }
}

async fn list_skills(
    State(state): State<SkillServerState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, String)> {
    check_auth(&headers, &state.auth_token)?;
    // 5D-1 MVP:先曝光 4 個純 LLM-only 的 text skill。memory / paste / mode
    // 那幾個有 Mori 平台 state 依賴(memory store / paste controller /
    // mode controller),5D-2 再 wire 上。
    Ok(Json(json!({
        "skills": [
            {"name": "translate", "description": describe::translate()},
            {"name": "polish",    "description": describe::polish()},
            {"name": "summarize", "description": describe::summarize()},
            {"name": "compose",   "description": describe::compose()},
        ],
    })))
}

async fn dispatch_skill(
    State(state): State<SkillServerState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(args): Json<Value>,
) -> Result<String, (StatusCode, String)> {
    check_auth(&headers, &state.auth_token)?;

    // 每次 dispatch 重新讀 routing,讓 user 改 config 後不必重啟 Mori 也生效。
    let routing = mori_core::llm::Routing::build_from_config(None).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("build routing: {e:#}"),
        )
    })?;
    let provider = routing.skill_provider(&name);

    // Arc clone 在每個 arm 裡 — 雖然 match 一輪只走一條,borrow checker
    // 仍要每條獨立持有所有權。Arc clone 只是 refcount++,實際成本可忽略。
    let skill: Box<dyn Skill> = match name.as_str() {
        "translate" => Box::new(TranslateSkill::new(provider.clone())),
        "polish" => Box::new(PolishSkill::new(provider.clone())),
        "summarize" => Box::new(SummarizeSkill::new(provider.clone())),
        "compose" => Box::new(ComposeSkill::new(provider.clone())),
        // memory / paste / mode skill 5D-2 再加(需要把 Mori state 共享進來)
        _ => {
            return Err((
                StatusCode::NOT_FOUND,
                format!(
                    "unknown or not-yet-exposed skill: {name}\n\
                     available: translate, polish, summarize, compose"
                ),
            ));
        }
    };

    tracing::info!(skill = %name, provider = %provider.name(), "skill dispatch via HTTP");
    let ctx = MoriContext::default();
    match skill.execute(args, &ctx).await {
        Ok(out) => Ok(out.user_message),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{}: {e:#}", name),
        )),
    }
}

/// `mori skill <name> --help` 顯示用的 skill 描述。直接 inline 在這,
/// 跟 mori-cli 那邊重複(不耦合到 mori-cli 跑去 query mori-tauri 才能列出
/// help — CLI 不該依賴 server 才能顯示 help)。
mod describe {
    pub fn translate() -> &'static str {
        "Translate text from one language to another.\n\n\
         Args:\n\
         - source_text (string, required): 要翻譯的原文\n\
         - target_lang (string, optional, default 'zh-TW'): 目標語言代碼"
    }

    pub fn polish() -> &'static str {
        "Polish / rewrite text in a given tone.\n\n\
         Args:\n\
         - source_text (string, required): 要潤飾的原文\n\
         - tone (string, optional): formal | casual | concise | friendly | neutral"
    }

    pub fn summarize() -> &'static str {
        "Summarize text into a chosen format.\n\n\
         Args:\n\
         - source_text (string, required): 要摘要的原文\n\
         - style (string, optional): bullet | paragraph | tldr"
    }

    pub fn compose() -> &'static str {
        "Draft new text from a brief.\n\n\
         Args:\n\
         - kind (string, required): email | message | essay | social_post\n\
         - prompt (string, required): 要寫什麼的指示\n\
         - audience (string, optional): 收件對象 / 場合"
    }
}
