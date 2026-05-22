//! OAuth2 flow — desktop-style installed-app flow,本機 redirect server。
//!
//! 流程:
//! 1. spawn 本機 HTTP listener(`localhost:8765`,等待 GET `/oauth/callback?code=…`)
//! 2. caller 開瀏覽器到 Google consent screen([`build_auth_url`])
//! 3. 使用者同意 → Google redirect 回 `localhost:8765/oauth/callback?code=…`
//! 4. listener 取 `code`,POST 給 Google token endpoint 換 access_token + refresh_token
//! 5. listener 回 user 一頁 "consent done, you can close this window" 後關 socket
//! 6. token 經 [`crate::token::GmailToken::save`] 寫進 `~/.mori/gmail-token.json`
//!
//! ## 為何不用大型 OAuth library
//!
//! `oauth2` crate 雖然 robust,但本流程實際只需要 ~3 個 HTTP 動作(redirect → 解
//! query → exchange / refresh POST),加 listener 整體仍在 ~300 行內。多帶一個大
//! dep 對 Mori 用戶而言只是 build time + binary size。User-owned data 原則也要求
//! 沒有第三方 OAuth relay,直接打 Google 即可。
//!
//! ## 安全注意
//!
//! - Listener 只 bind `127.0.0.1`,不對外開放
//! - `state` 參數應在初次 redirect 時設,callback 比對 — 本實作 [`parse_callback_query`]
//!   會把 state 一併拉出來給 caller 比對
//! - `client_secret` 對 desktop OAuth client 而言不是真 secret(Google 文件明說),
//!   但仍應放使用者本機 `~/.mori/gmail-config.json`,不入 repo
//!
//! ## 未實作 / 暫不做
//!
//! - PKCE(Google desktop 流程支援但非必要;Gm-2 可以加)
//! - 多帳號(token 檔目前單例)
//! - Listener TLS(不必要;`http://localhost` 是 Google 對 desktop client 認可的
//!   redirect URI scheme)

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::token::{home_dir, GmailToken, TokenError};

/// Gm-1 唯一 scope。Gm-2 接 send 時會擴成 `"gmail.readonly gmail.send"`。
pub const GMAIL_READONLY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";

/// Gm-2 升 scope:`gmail.send`(POST /messages/send 需要)。
pub const GMAIL_SEND_SCOPE: &str = "https://www.googleapis.com/auth/gmail.send";

/// Gm-2 預設組合 scope — readonly + send。Mori 對 Gmail 的標準授權集合。
pub const GMAIL_DEFAULT_SCOPES: &[&str] = &[GMAIL_READONLY_SCOPE, GMAIL_SEND_SCOPE];

/// Google OAuth2 endpoints。獨立常數方便測試時改寫(client 層走 mock server)。
pub const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// OAuth 客戶端 config — 由 user 從 Google Cloud Console 拿,寫進
/// `~/.mori/gmail-config.json`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

impl OAuthConfig {
    /// 從 disk 讀 config(`~/.mori/gmail-config.json` 或 caller 給的路徑)。
    pub fn load(path: &Path) -> Result<Self, OAuthError> {
        let raw = std::fs::read_to_string(path).map_err(OAuthError::Io)?;
        let cfg: Self = serde_json::from_str(&raw).map_err(OAuthError::Json)?;
        Ok(cfg)
    }

    /// 預設 config 路徑 — `~/.mori/gmail-config.json`(同 token 路徑同目錄)。
    pub fn default_path() -> Option<PathBuf> {
        home_dir().map(|h| h.join(".mori").join("gmail-config.json"))
    }
}

/// OAuth 層錯誤類型。
#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    /// 本機 listener / TCP IO 失敗。
    #[error("io: {0}")]
    Io(std::io::Error),

    /// Config / response JSON 解析失敗。
    #[error("json: {0}")]
    Json(serde_json::Error),

    /// HTTP request 失敗(token exchange / refresh)。
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    /// Google token endpoint 回 4xx / 5xx。
    #[error("token endpoint error: status={status} body={body}")]
    TokenEndpoint { status: u16, body: String },

    /// Listener 收到 callback 但沒有 `code` 參數(user 拒絕 / Google 出錯)。
    #[error("missing 'code' in callback: {0}")]
    MissingCode(String),

    /// `state` 不匹配 — 可能被 CSRF / 中間人攻擊;直接拒。
    #[error("oauth state mismatch (expected={expected}, got={got})")]
    StateMismatch { expected: String, got: String },

    /// 寫 token 檔失敗。
    #[error("token save: {0}")]
    Token(#[from] TokenError),

    /// Redirect URI 不合法 / 無法解析。
    #[error("invalid redirect uri: {0}")]
    InvalidRedirectUri(String),
}

impl From<std::io::Error> for OAuthError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for OAuthError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// Google token endpoint 的 raw response。
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    /// refresh_token 只在初次 consent 時有;refresh 流程通常拿不到新的。
    refresh_token: Option<String>,
    expires_in: i64,
    scope: String,
    token_type: String,
}

/// 組 Google consent URL。
///
/// `state` 是 caller 提供的 CSRF token,callback 必須回相同 state。Caller 自己生
/// 隨機字串(不在這層引 rand crate)。
///
/// Scopes 由 caller 控制 — Gm-1 用 `[GMAIL_READONLY_SCOPE]`,Gm-2 升級成
/// `[GMAIL_READONLY_SCOPE, GMAIL_SEND_SCOPE]`(也就是 `GMAIL_DEFAULT_SCOPES`)。
/// Google 對 scope 的格式是 single string、space-separated;這層把 slice join 起來。
pub fn build_auth_url(config: &OAuthConfig, scopes: &[&str], state: &str) -> String {
    // 標準 OAuth2 query。`access_type=offline` 才會回 refresh_token;
    // `prompt=consent` 強制每次都跑 consent UI(避免 user 已給過 readonly 但要升 scope 時
    // Google 跳過 consent)。Gm-1 兩者都加,Gm-2 升 scope 也照樣 work。
    let scope_joined = scopes.join(" ");
    let mut url = Url::parse(GOOGLE_AUTH_URL).expect("hard-coded URL must parse");
    url.query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &config.redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &scope_joined)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", state);
    url.into()
}

/// 從 callback request line 抽 query params。
///
/// 純 parse helper,跟 server / network IO 解耦,可獨立測。
///
/// Input 是 Google redirect 過來的 path+query,例如
/// `/oauth/callback?code=4/0AX...&state=abc&scope=...`。
pub fn parse_callback_query(request_line: &str) -> HashMap<String, String> {
    // request line:`GET /oauth/callback?code=…&state=… HTTP/1.1`
    let path_and_query = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("");
    let query = path_and_query.split('?').nth(1).unwrap_or("");

    let mut out = HashMap::new();
    for kv in query.split('&').filter(|s| !s.is_empty()) {
        let mut parts = kv.splitn(2, '=');
        if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
            let key = url::form_urlencoded::parse(k.as_bytes())
                .map(|(a, _)| a.into_owned())
                .next()
                .unwrap_or_else(|| k.to_string());
            let val = url::form_urlencoded::parse(v.as_bytes())
                .map(|(a, _)| a.into_owned())
                .next()
                .unwrap_or_else(|| v.to_string());
            out.insert(key, val);
        }
    }
    out
}

/// Listener bind port,從 `redirect_uri` 拉出來。預設 8765。
fn redirect_port(redirect_uri: &str) -> Result<u16, OAuthError> {
    let url = Url::parse(redirect_uri)
        .map_err(|e| OAuthError::InvalidRedirectUri(format!("{redirect_uri}: {e}")))?;
    url.port_or_known_default()
        .ok_or_else(|| OAuthError::InvalidRedirectUri(format!("{redirect_uri}: no port")))
}

/// 跑完整 OAuth flow:spawn listener,等 user 在瀏覽器同意,exchange token,save token。
///
/// **不會自動開瀏覽器** — caller(Gm-2 Tauri command)用 `webbrowser::open` /
/// shell 開,本層只負責 listener + token exchange,避免拉 `webbrowser` dep 進來。
///
/// Caller 流程示意:
/// ```ignore
/// let state = "random-csrf-token";  // caller 自己生
/// let auth_url = build_auth_url(&config, GMAIL_READONLY_SCOPE, state);
/// // 並行:開瀏覽器 + 等 listener
/// std::thread::spawn(|| open_browser(&auth_url));
/// let token = run_oauth_flow(&config, state, GMAIL_READONLY_SCOPE).await?;
/// token.save(&token_path)?;
/// ```
///
/// Listener 是 **blocking**(`std::net::TcpListener`)— 在 tokio async 環境用
/// `tokio::task::spawn_blocking` 包,不阻塞 runtime。本函式內部已用
/// `spawn_blocking`,caller 直接 `.await` 即可。
pub async fn run_oauth_flow(
    config: &OAuthConfig,
    expected_state: &str,
    scopes: &[&str],
) -> Result<GmailToken, OAuthError> {
    let port = redirect_port(&config.redirect_uri)?;
    let expected_state_owned = expected_state.to_string();

    // 1. 等 listener 收 callback。spawn_blocking 跑 std::net listener。
    let code = tokio::task::spawn_blocking(move || -> Result<String, OAuthError> {
        wait_for_callback(port, &expected_state_owned)
    })
    .await
    .map_err(|e| {
        OAuthError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("listener join: {e}"),
        ))
    })??;

    // 2. exchange code → token
    //    scope 在 exchange_code 內目前用不到(Google 從 authorization code 自身解出
    //    granted scope),參數保留供未來 PKCE / scope-down-grade 場景擴用。
    exchange_code(config, &code, scopes).await
}

/// 阻塞等 listener 接到第一筆 callback,parse code,回應 user 一頁簡單 HTML 後關 socket。
fn wait_for_callback(port: u16, expected_state: &str) -> Result<String, OAuthError> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    // 超時(5 分鐘)避免無限掛著 — user 沒同意 / 關 tab 時 listener 不會永遠等。
    listener
        .set_nonblocking(false)
        .map_err(OAuthError::Io)?;

    // 只接一筆 connection 就結束。若 user 在 5 分鐘內沒同意,呼叫者 future 也會被
    // 上層 task timeout drop 掉(本層不自帶 timeout 邏輯,留給 caller 視 UX 而定)。
    let (mut stream, _peer) = listener.accept()?;

    // 不期待 long-running connection — 設個短 read timeout 防壞 client 卡住。
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(OAuthError::Io)?;

    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf)?;
    let request_text = String::from_utf8_lossy(&buf[..n]);

    // request_text 是整段 HTTP request;第一行就是 request line。
    let request_line = request_text.lines().next().unwrap_or("");
    let params = parse_callback_query(request_line);

    // 比對 state(CSRF 防護)— 即使 user 已同意,state 不對也 reject。
    match params.get("state") {
        Some(got) if got == expected_state => {}
        Some(got) => {
            let _ = stream.write_all(callback_response_body(
                "state mismatch — possible CSRF, refusing.",
            ).as_bytes());
            return Err(OAuthError::StateMismatch {
                expected: expected_state.to_string(),
                got: got.clone(),
            });
        }
        None => {
            let _ = stream.write_all(
                callback_response_body("missing state — refusing.").as_bytes(),
            );
            return Err(OAuthError::MissingCode("no state".into()));
        }
    }

    if let Some(err) = params.get("error") {
        let _ = stream.write_all(
            callback_response_body(&format!("Google returned error: {err}"))
                .as_bytes(),
        );
        return Err(OAuthError::MissingCode(format!("google error: {err}")));
    }

    let code = match params.get("code") {
        Some(c) => c.clone(),
        None => {
            let _ = stream.write_all(
                callback_response_body("no 'code' in callback — refusing.")
                    .as_bytes(),
            );
            return Err(OAuthError::MissingCode(request_line.to_string()));
        }
    };

    // 回 user 一頁簡單成功訊息。即使後面 exchange 失敗,user 端瀏覽器已關心不到。
    let _ = stream.write_all(
        callback_response_body("Mori received your consent. You can close this tab.")
            .as_bytes(),
    );

    Ok(code)
}

fn callback_response_body(message: &str) -> String {
    // 極簡 HTTP/1.1 response;沒裝 hyper / axum,自己寫 headers。
    let body = format!(
        "<!doctype html><html><body style='font-family:sans-serif;padding:2em'>\
         <h2>Mori — Gmail</h2><p>{message}</p></body></html>"
    );
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

/// 用 authorization code 跟 Google 換 token。
async fn exchange_code(
    config: &OAuthConfig,
    code: &str,
    _scopes: &[&str],
) -> Result<GmailToken, OAuthError> {
    exchange_code_at(config, code, GOOGLE_TOKEN_URL).await
}

/// `exchange_code` 但 endpoint 可改寫 — test only。
async fn exchange_code_at(
    config: &OAuthConfig,
    code: &str,
    endpoint: &str,
) -> Result<GmailToken, OAuthError> {
    let client = reqwest::Client::new();
    let form = [
        ("code", code),
        ("client_id", config.client_id.as_str()),
        ("client_secret", config.client_secret.as_str()),
        ("redirect_uri", config.redirect_uri.as_str()),
        ("grant_type", "authorization_code"),
    ];
    let resp = client.post(endpoint).form(&form).send().await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(OAuthError::TokenEndpoint {
            status: status.as_u16(),
            body,
        });
    }
    let raw: TokenResponse = resp.json().await?;
    let refresh = raw.refresh_token.ok_or_else(|| OAuthError::TokenEndpoint {
        status: 200,
        body: "no refresh_token in response — re-consent with prompt=consent".into(),
    })?;
    Ok(GmailToken {
        access_token: raw.access_token,
        refresh_token: refresh,
        expires_at: Utc::now() + chrono::Duration::seconds(raw.expires_in),
        scope: raw.scope,
        token_type: raw.token_type,
    })
}

/// 用 refresh_token 拿新 access_token。Google 通常不會回新的 refresh_token,
/// 我們把舊的 refresh_token 保留下來覆蓋進新 [`GmailToken`]。
pub async fn refresh_token(
    token: &GmailToken,
    config: &OAuthConfig,
) -> Result<GmailToken, OAuthError> {
    refresh_token_at(token, config, GOOGLE_TOKEN_URL).await
}

/// `refresh_token` 但 endpoint 可改寫 — test only。
pub(crate) async fn refresh_token_at(
    token: &GmailToken,
    config: &OAuthConfig,
    endpoint: &str,
) -> Result<GmailToken, OAuthError> {
    let client = reqwest::Client::new();
    let form = [
        ("refresh_token", token.refresh_token.as_str()),
        ("client_id", config.client_id.as_str()),
        ("client_secret", config.client_secret.as_str()),
        ("grant_type", "refresh_token"),
    ];
    let resp = client.post(endpoint).form(&form).send().await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(OAuthError::TokenEndpoint {
            status: status.as_u16(),
            body,
        });
    }
    let raw: TokenResponse = resp.json().await?;
    Ok(GmailToken {
        access_token: raw.access_token,
        // refresh 流程通常不會給新的 refresh_token,沿用舊的。
        refresh_token: raw.refresh_token.unwrap_or_else(|| token.refresh_token.clone()),
        expires_at: Utc::now() + chrono::Duration::seconds(raw.expires_in),
        scope: raw.scope,
        token_type: raw.token_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_config_parses_from_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gmail-config.json");
        let raw = r#"{
            "client_id": "abc.apps.googleusercontent.com",
            "client_secret": "GOCSPX-xyz",
            "redirect_uri": "http://localhost:8765/oauth/callback"
        }"#;
        std::fs::write(&path, raw).unwrap();

        let cfg = OAuthConfig::load(&path).expect("config load");
        assert_eq!(cfg.client_id, "abc.apps.googleusercontent.com");
        assert_eq!(cfg.client_secret, "GOCSPX-xyz");
        assert_eq!(cfg.redirect_uri, "http://localhost:8765/oauth/callback");
    }

    #[test]
    fn oauth_redirect_uri_parses_code() {
        // 真實 Google redirect 過來的 request line(含 url-encoded code)。
        let line = "GET /oauth/callback?state=xyz&code=4%2F0AX4XfWj-secret&scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fgmail.readonly HTTP/1.1";
        let params = parse_callback_query(line);

        assert_eq!(params.get("state").map(String::as_str), Some("xyz"));
        // url-encoded `4/0AX4XfWj-secret`
        assert_eq!(
            params.get("code").map(String::as_str),
            Some("4/0AX4XfWj-secret")
        );
        assert_eq!(
            params.get("scope").map(String::as_str),
            Some("https://www.googleapis.com/auth/gmail.readonly")
        );
    }

    #[test]
    fn build_auth_url_includes_required_params() {
        let cfg = OAuthConfig {
            client_id: "cid".into(),
            client_secret: "secret".into(),
            redirect_uri: "http://localhost:8765/oauth/callback".into(),
        };
        let url = build_auth_url(&cfg, &[GMAIL_READONLY_SCOPE], "state-abc");

        // 不假設 query order,只 assert 必要參數都在。
        assert!(url.starts_with(GOOGLE_AUTH_URL), "url: {url}");
        for needle in [
            "client_id=cid",
            "response_type=code",
            "access_type=offline",
            "prompt=consent",
            "state=state-abc",
        ] {
            assert!(url.contains(needle), "url should contain {needle}: {url}");
        }
        // scope 是 url-encoded 的;assert encoded form。
        assert!(
            url.contains("scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fgmail.readonly"),
            "url should contain encoded scope: {url}"
        );
    }

    #[test]
    fn build_auth_url_joins_multi_scope_with_space() {
        let cfg = OAuthConfig {
            client_id: "cid".into(),
            client_secret: "secret".into(),
            redirect_uri: "http://localhost:8765/oauth/callback".into(),
        };
        let url = build_auth_url(&cfg, GMAIL_DEFAULT_SCOPES, "s");

        // 兩個 scope 用 `+`(url-encoded space)分隔。
        // Order = caller-provided slice order → readonly first, send second。
        assert!(
            url.contains(
                "scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fgmail.readonly+\
                 https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fgmail.send"
            ),
            "url should contain both scopes joined with space: {url}"
        );
    }

    #[test]
    fn redirect_port_extracts_from_uri() {
        assert_eq!(
            redirect_port("http://localhost:8765/oauth/callback").unwrap(),
            8765
        );
        assert_eq!(
            redirect_port("http://127.0.0.1:9090/cb").unwrap(),
            9090
        );
    }

    #[tokio::test]
    async fn refresh_token_hits_endpoint_and_returns_new_token() {
        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock = MockServer::start().await;
        let response_body = serde_json::json!({
            "access_token": "ya29.new_access",
            "expires_in": 3599,
            "scope": "https://www.googleapis.com/auth/gmail.readonly",
            "token_type": "Bearer"
            // 注意:refresh 流程沒回新 refresh_token,沿用舊的。
        });
        Mock::given(method("POST"))
            .and(path("/"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
            .mount(&mock)
            .await;

        let cfg = OAuthConfig {
            client_id: "cid".into(),
            client_secret: "csecret".into(),
            redirect_uri: "http://localhost:8765/oauth/callback".into(),
        };
        let old = GmailToken {
            access_token: "ya29.expired".into(),
            refresh_token: "old_refresh".into(),
            expires_at: Utc::now() - chrono::Duration::seconds(60),
            scope: GMAIL_READONLY_SCOPE.into(),
            token_type: "Bearer".into(),
        };
        let endpoint = format!("{}/", mock.uri());
        let new_token = refresh_token_at(&old, &cfg, &endpoint).await.expect("refresh ok");

        assert_eq!(new_token.access_token, "ya29.new_access");
        // 沿用舊 refresh_token
        assert_eq!(new_token.refresh_token, "old_refresh");
        assert!(!new_token.is_expired(), "newly refreshed token should be fresh");
    }
}
