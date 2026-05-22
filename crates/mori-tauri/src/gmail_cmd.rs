//! Tauri commands bridging mori-gmail 到 IPC / LLM tool(Wave 8 Gm-2)。
//!
//! 對齊 `reminders_cmd.rs` 的「do_* helper + #[tauri::command] wrapper」風格 —
//! command 內帶 `tauri::State` 很難 unit test,把實質邏輯拆出來。
//!
//! ## 五條 commands
//!
//! - `gmail_oauth_start_cmd(scopes)` — 跑完整 OAuth flow(spawn listener、等 user
//!   在瀏覽器同意、save token)。**不會自動開瀏覽器**,只回 auth URL 給前端去開
//!   (前端用 `open_external_url` Tauri command 自己處理)。回 `()`。
//! - `gmail_oauth_status_cmd()` — 看 ~/.mori/gmail-token.json 是否存在、是否含
//!   send scope。回 `GmailOAuthStatus { authorized, has_send_scope }`。
//! - `gmail_list_threads_cmd(query?, max?)` — 對映 `GmailClient::list_threads`。
//! - `gmail_get_thread_cmd(thread_id)` — 對映 `GmailClient::get_thread`。
//! - `gmail_send_cmd(to, subject, body, reply_to_thread_id?, in_reply_to?)` —
//!   寄信(或回覆既有 thread)。

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use mori_core::skill::SharedGmailClient;
use mori_gmail::{
    default_token_path, GmailClient, GmailToken, OAuthConfig, SendOutcome, Thread,
    ThreadSummary, GMAIL_DEFAULT_SCOPES, GMAIL_SEND_SCOPE,
};

// ─────────────────────────────────────────────────────────────────────
// 啟動時:嘗試 init client(沒 config / 沒 token 就回 None)
// ─────────────────────────────────────────────────────────────────────

/// 嘗試從本機既有 config + token 建一份 GmailClient。
///
/// 流程:
/// 1. `~/.mori/gmail-config.json` 存在 → load OAuthConfig,否則 None
/// 2. `~/.mori/gmail-token.json` 存在 → GmailClient::new 載 token,否則 None
///
/// 失敗一律回 None(`tracing::warn` 記錄原因)— gmail 是 optional feature,
/// 任何啟動失敗都不該擋住 Mori 主功能。Caller 拿到 None 就不註冊 Gmail skill /
/// 不 .manage(SharedGmailClient),LLM 看不到 Gmail 工具。
pub async fn init_gmail_client_optional() -> Option<SharedGmailClient> {
    let config_path = OAuthConfig::default_path()?;
    if !config_path.exists() {
        tracing::info!(
            path = %config_path.display(),
            "gmail-config.json not found — Gmail skills disabled",
        );
        return None;
    }
    let config = match OAuthConfig::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                path = %config_path.display(),
                error = %e,
                "gmail-config.json load failed — Gmail skills disabled",
            );
            return None;
        }
    };
    let token_path = default_token_path()?;
    if !token_path.exists() {
        tracing::info!(
            path = %token_path.display(),
            "gmail-token.json not found (need OAuth consent first) — Gmail skills disabled",
        );
        return None;
    }
    let client = match GmailClient::new(token_path, config).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "GmailClient::new failed — Gmail skills disabled");
            return None;
        }
    };
    Some(SharedGmailClient(Arc::new(Mutex::new(client))))
}

// ─────────────────────────────────────────────────────────────────────
// 內部 helpers(無 Tauri State,可直接 unit test)
// ─────────────────────────────────────────────────────────────────────

/// `gmail_oauth_status_cmd` 的 helper — 給 token_path 自己控制(unit test 用 tempdir)。
pub(crate) fn do_oauth_status(token_path: &PathBuf) -> GmailOAuthStatus {
    match GmailToken::load(token_path) {
        Ok(t) => GmailOAuthStatus {
            authorized: true,
            has_send_scope: t.has_scope(GMAIL_SEND_SCOPE),
            scope: t.scope.clone(),
        },
        Err(_) => GmailOAuthStatus {
            authorized: false,
            has_send_scope: false,
            scope: String::new(),
        },
    }
}

/// `gmail_list_threads_cmd` 的 helper。
pub(crate) async fn do_list_threads(
    client: &Mutex<GmailClient>,
    query: Option<String>,
    max: Option<u32>,
) -> Result<Vec<ThreadSummary>, String> {
    let mut c = client.lock().await;
    c.list_threads(query.as_deref(), max.unwrap_or(10))
        .await
        .map_err(|e| e.to_string())
}

/// `gmail_get_thread_cmd` 的 helper。
pub(crate) async fn do_get_thread(
    client: &Mutex<GmailClient>,
    thread_id: String,
) -> Result<Thread, String> {
    let mut c = client.lock().await;
    c.get_thread(&thread_id).await.map_err(|e| e.to_string())
}

/// `gmail_send_cmd` 的 helper。
pub(crate) async fn do_send(
    client: &Mutex<GmailClient>,
    to: Vec<String>,
    subject: String,
    body: String,
    reply_to_thread_id: Option<String>,
    in_reply_to: Option<String>,
) -> Result<SendOutcome, String> {
    // scope guard — 對齊 SendGmailSkill 的行為:先 check token scope,讓 error
    // 訊息對 user / LLM 友善(Google 自己回 403 但 message 不夠直觀)。
    {
        let c = client.lock().await;
        if !c.token_snapshot().has_scope(GMAIL_SEND_SCOPE) {
            return Err(format!(
                "send_gmail requires `{}` scope; please re-run OAuth flow",
                GMAIL_SEND_SCOPE,
            ));
        }
    }
    let mut c = client.lock().await;
    match (reply_to_thread_id.as_deref(), in_reply_to.as_deref()) {
        (Some(tid), Some(irt)) => c
            .send_reply(tid, &to, &subject, &body, irt)
            .await
            .map_err(|e| e.to_string()),
        (Some(tid), None) => {
            let placeholder = format!("<{tid}@thread.placeholder>");
            c.send_reply(tid, &to, &subject, &body, &placeholder)
                .await
                .map_err(|e| e.to_string())
        }
        _ => c
            .send_message(&to, &subject, &body)
            .await
            .map_err(|e| e.to_string()),
    }
}

// ─────────────────────────────────────────────────────────────────────
// 公開 IPC types
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailOAuthStatus {
    /// `~/.mori/gmail-token.json` 載得起來。
    pub authorized: bool,
    /// 此 token 含 `gmail.send` scope(`send_gmail` 才能用)。
    pub has_send_scope: bool,
    /// Granted scope 全字串(space-separated)— 供 UI 顯示。
    pub scope: String,
}

// ─────────────────────────────────────────────────────────────────────
// Tauri commands
// ─────────────────────────────────────────────────────────────────────

/// 跑完整 OAuth flow(blocking listener + token exchange + save)。
///
/// `scopes` 沒帶就用 `GMAIL_DEFAULT_SCOPES`(readonly + send)。流程:
/// 1. caller(前端)在呼叫此 command **之前** open auth URL — 用 `open_external_url`
///    或 `webbrowser` 開,Mori 本層只負責 listener
/// 2. user 在瀏覽器同意 → Google redirect 回 localhost:8765/oauth/callback
/// 3. listener 收 code → POST 換 token → save 到 `~/.mori/gmail-token.json`
///
/// **CAVEAT**:此 command 跑完後 client / SharedGmailClient state 仍是舊的 —
/// LLM 要看到新 scope **需要重啟 Mori**(simplification:不做 hot-swap)。
#[tauri::command]
pub async fn gmail_oauth_start_cmd(scopes: Option<Vec<String>>) -> Result<String, String> {
    let config_path =
        OAuthConfig::default_path().ok_or("no HOME / USERPROFILE — can't resolve gmail-config.json path")?;
    let config = OAuthConfig::load(&config_path).map_err(|e| {
        format!(
            "load gmail-config.json failed (path={}): {e}",
            config_path.display()
        )
    })?;

    // state 簡單用 timestamp+pid 當 CSRF token(本機 listener、URL 不出本機,
    // 真有人 MITM ~/.mori 也 game over);本層只負責跟 wait_for_callback 比對。
    let state = format!(
        "mori-{}-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
        std::process::id()
    );

    // scopes:caller 沒帶就用 default(readonly + send)。
    let scope_strings: Vec<String> = scopes.unwrap_or_else(|| {
        GMAIL_DEFAULT_SCOPES.iter().map(|s| s.to_string()).collect()
    });
    let scope_refs: Vec<&str> = scope_strings.iter().map(|s| s.as_str()).collect();

    // 印 auth URL 給前端 / log。前端負責開瀏覽器。
    let auth_url = mori_gmail::build_auth_url(&config, &scope_refs, &state);
    tracing::info!(auth_url = %auth_url, "Gmail OAuth started — open this URL in browser");

    let token = mori_gmail::run_oauth_flow(&config, &state, &scope_refs)
        .await
        .map_err(|e| format!("OAuth flow failed: {e}"))?;

    let token_path = default_token_path().ok_or("no HOME — can't resolve token path")?;
    token
        .save(&token_path)
        .map_err(|e| format!("save token failed: {e}"))?;
    tracing::info!(path = %token_path.display(), "Gmail token saved");

    // 回 user-facing 訊息 + auth URL(前端要再開一次 url 給 user)。
    Ok(format!(
        "OAuth 完成,token 已存 {}。\n(初次跑前需要先開瀏覽器跑 consent — auth_url:{})",
        token_path.display(),
        auth_url,
    ))
}

/// 看 token 狀態(authorized? send scope?)。
#[tauri::command]
pub fn gmail_oauth_status_cmd() -> Result<GmailOAuthStatus, String> {
    let path = default_token_path().ok_or("no HOME — can't resolve token path")?;
    Ok(do_oauth_status(&path))
}

/// `list_threads(query?, max?)` — 列最近 thread summary。
#[tauri::command]
pub async fn gmail_list_threads_cmd(
    state: tauri::State<'_, SharedGmailClient>,
    query: Option<String>,
    max: Option<u32>,
) -> Result<Vec<ThreadSummary>, String> {
    do_list_threads(&state.0, query, max).await
}

/// `get_thread(thread_id)` — 展開 thread 全文(messages + bodies)。
#[tauri::command]
pub async fn gmail_get_thread_cmd(
    state: tauri::State<'_, SharedGmailClient>,
    thread_id: String,
) -> Result<Thread, String> {
    do_get_thread(&state.0, thread_id).await
}

/// `send(to, subject, body, reply_to_thread_id?, in_reply_to?)` — 寄信(或回覆既有 thread)。
#[tauri::command]
pub async fn gmail_send_cmd(
    state: tauri::State<'_, SharedGmailClient>,
    to: Vec<String>,
    subject: String,
    body: String,
    reply_to_thread_id: Option<String>,
    in_reply_to: Option<String>,
) -> Result<SendOutcome, String> {
    do_send(&state.0, to, subject, body, reply_to_thread_id, in_reply_to).await
}

// ─────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! 對齊 reminders_cmd::tests 風格 — Tauri State / runtime mock 麻煩,
    //! 直接 unit-test `do_*` helpers。GmailClient 用 `with_base` 注 wiremock。

    use super::*;
    use base64::Engine as _;
    use chrono::Utc;
    use mori_gmail::{GmailToken, GMAIL_READONLY_SCOPE};
    use wiremock::matchers::{method, path as wmpath};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn dummy_oauth() -> OAuthConfig {
        OAuthConfig {
            client_id: "cid".into(),
            client_secret: "csecret".into(),
            redirect_uri: "http://localhost:8765/oauth/callback".into(),
        }
    }

    fn token_with(scope: &str) -> GmailToken {
        GmailToken {
            access_token: "ya29.fake".into(),
            refresh_token: "1//r".into(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            scope: scope.into(),
            token_type: "Bearer".into(),
        }
    }

    fn client_with(mock_uri: String, token: GmailToken) -> Mutex<GmailClient> {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("gmail-token.json");
        let token_endpoint = format!("{mock_uri}/token");
        Mutex::new(GmailClient::with_base(
            token,
            dummy_oauth(),
            token_path,
            mock_uri,
            token_endpoint,
        ))
    }

    #[test]
    fn do_oauth_status_returns_not_authorized_for_missing_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gmail-token.json"); // 不存在
        let status = do_oauth_status(&path);
        assert!(!status.authorized);
        assert!(!status.has_send_scope);
        assert!(status.scope.is_empty());
    }

    #[test]
    fn do_oauth_status_reports_authorized_and_scope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gmail-token.json");
        let token = token_with(&format!("{GMAIL_READONLY_SCOPE} {GMAIL_SEND_SCOPE}"));
        token.save(&path).unwrap();

        let status = do_oauth_status(&path);
        assert!(status.authorized);
        assert!(status.has_send_scope);
        assert!(status.scope.contains("gmail.readonly"));
        assert!(status.scope.contains("gmail.send"));
    }

    #[test]
    fn do_oauth_status_authorized_but_no_send_scope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gmail-token.json");
        let token = token_with(GMAIL_READONLY_SCOPE);
        token.save(&path).unwrap();

        let status = do_oauth_status(&path);
        assert!(status.authorized);
        assert!(!status.has_send_scope);
        assert_eq!(status.scope, GMAIL_READONLY_SCOPE);
    }

    #[tokio::test]
    async fn do_list_threads_returns_summaries() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(wmpath("/gmail/v1/users/me/threads"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "threads": [{"id": "t1", "snippet": "yo", "historyId": "100"}]
            })))
            .mount(&mock)
            .await;
        let c = client_with(mock.uri(), token_with(GMAIL_READONLY_SCOPE));
        let out = do_list_threads(&c, None, Some(5)).await.expect("ok");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "t1");
    }

    #[tokio::test]
    async fn do_get_thread_returns_thread_data() {
        let mock = MockServer::start().await;
        let body_b64 =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"plain body");
        Mock::given(method("GET"))
            .and(wmpath("/gmail/v1/users/me/threads/t-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "t-1",
                "messages": [{
                    "id": "m1",
                    "snippet": "snip",
                    "internalDate": "1716000000000",
                    "payload": {
                        "mimeType": "text/plain",
                        "headers": [
                            {"name": "From", "value": "a@x.com"},
                            {"name": "To", "value": "b@x.com"},
                            {"name": "Subject", "value": "S"}
                        ],
                        "body": {"data": body_b64}
                    }
                }]
            })))
            .mount(&mock)
            .await;
        let c = client_with(mock.uri(), token_with(GMAIL_READONLY_SCOPE));
        let thread = do_get_thread(&c, "t-1".into()).await.expect("ok");
        assert_eq!(thread.id, "t-1");
        assert_eq!(thread.messages.len(), 1);
        assert_eq!(thread.messages[0].body_text, "plain body");
    }

    #[tokio::test]
    async fn do_send_blocks_without_send_scope() {
        let mock = MockServer::start().await;
        // mock 即使 mount 也不該被打到
        Mock::given(method("POST"))
            .and(wmpath("/gmail/v1/users/me/messages/send"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "x",
                "threadId": "x"
            })))
            .mount(&mock)
            .await;

        let c = client_with(mock.uri(), token_with(GMAIL_READONLY_SCOPE));
        let err = do_send(
            &c,
            vec!["a@b.c".into()],
            "s".into(),
            "b".into(),
            None,
            None,
        )
        .await
        .expect_err("should block");
        assert!(err.contains("scope"), "expected scope error, got: {err}");
    }

    #[tokio::test]
    async fn do_send_sends_when_scope_ok() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(wmpath("/gmail/v1/users/me/messages/send"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "sent-1",
                "threadId": "t-fresh"
            })))
            .mount(&mock)
            .await;
        let c = client_with(
            mock.uri(),
            token_with(&format!("{GMAIL_READONLY_SCOPE} {GMAIL_SEND_SCOPE}")),
        );
        let out = do_send(
            &c,
            vec!["a@b.c".into()],
            "s".into(),
            "b".into(),
            None,
            None,
        )
        .await
        .expect("send ok");
        assert_eq!(out.id, "sent-1");
        assert_eq!(out.thread_id, "t-fresh");
    }
}
