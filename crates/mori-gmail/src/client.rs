//! Gmail REST API client(read-only)。
//!
//! 對應的 Google API doc:
//! - List threads:`GET /gmail/v1/users/{userId}/threads`
//! - Get thread:`GET /gmail/v1/users/{userId}/threads/{id}`
//! - List labels:`GET /gmail/v1/users/{userId}/labels`
//!
//! `userId` 一律用 `"me"`(token 對應的 user)。
//!
//! ## Token 新鮮度
//!
//! 每個 mut method 進場先 [`ensure_fresh_token`] — 若 token 過期就用 refresh_token
//! 換新並 save 回 disk。LLM 在 Gm-2 透過 Skill 呼叫時不需要關心 refresh 細節。
//!
//! ## Body 解碼
//!
//! Gmail message body 是 **base64url(no padding)**。我們對每個 message 找:
//! 1. 若 mime type = `text/plain`,直接 decode 該 part
//! 2. 否則找 `multipart/alternative` → 內含 `text/plain` part
//! 3. 都拿不到 → `body_text = ""`(寧可空字串也不讓 caller 收到 partial parse 噪音)
//!
//! HTML-only / attachment 等狀況 Gm-1 暫不處理(text/plain 對 LLM context 足夠)。
//!
//! ## Endpoint base
//!
//! Production base 為 `https://gmail.googleapis.com`。測試時透過
//! [`GmailClient::with_base`] 注入 mock server URL。

use std::path::PathBuf;

use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::oauth::{refresh_token_at, OAuthConfig, OAuthError, GOOGLE_TOKEN_URL};
use crate::token::{GmailToken, TokenError};

/// Production Gmail API base。測試走 [`GmailClient::with_base`] 改寫。
pub const GMAIL_API_BASE: &str = "https://gmail.googleapis.com";

/// Client 層錯誤類型。
#[derive(Debug, thiserror::Error)]
pub enum GmailError {
    #[error("token: {0}")]
    Token(#[from] TokenError),

    #[error("oauth: {0}")]
    OAuth(#[from] OAuthError),

    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    /// Gmail API 回 4xx / 5xx。`message` 是 Google JSON error body 或 raw text。
    #[error("api: status={status} message={message}")]
    Api { status: u16, message: String },

    /// Body base64url 解碼失敗(罕見;Google 自己出問題)。
    #[error("base64 decode: {0}")]
    Base64(#[from] base64::DecodeError),

    /// 解析回應 JSON 結構時出問題(欄位缺失 / 型別不對)。
    #[error("response shape: {0}")]
    Shape(String),
}

/// `threads.list` 回的 thread summary(沒展開 messages)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadSummary {
    pub id: String,
    #[serde(default)]
    pub snippet: String,
    #[serde(default, rename = "historyId")]
    pub history_id: String,
}

/// `threads.get` 回的完整 thread(含 messages + bodies)。
#[derive(Debug, Clone, Serialize)]
pub struct Thread {
    pub id: String,
    pub messages: Vec<Message>,
}

/// 單封 Gmail message — 解析過的 header + base64url-decoded text body。
#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub id: String,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    pub date: DateTime<Utc>,
    pub body_text: String,
    pub snippet: String,
}

/// Gmail label(`labels.list` 結果)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Label {
    pub id: String,
    pub name: String,
    #[serde(default, rename = "type")]
    pub label_type: String,
}

/// 主 client。對外 read-only(send 留 Gm-2)。
pub struct GmailClient {
    token: GmailToken,
    oauth_config: OAuthConfig,
    token_path: PathBuf,
    http: reqwest::Client,
    /// Gmail REST endpoint base — 預設 [`GMAIL_API_BASE`],測試改寫。
    api_base: String,
    /// Token refresh endpoint — 預設 [`GOOGLE_TOKEN_URL`],測試改寫。
    token_endpoint: String,
}

impl GmailClient {
    /// 從 disk 載 token + config,組 client。Token 過期會在第一次 API call 自動 refresh。
    pub async fn new(
        token_path: PathBuf,
        oauth_config: OAuthConfig,
    ) -> Result<Self, GmailError> {
        let token = GmailToken::load(&token_path)?;
        Ok(Self {
            token,
            oauth_config,
            token_path,
            http: reqwest::Client::new(),
            api_base: GMAIL_API_BASE.to_string(),
            token_endpoint: GOOGLE_TOKEN_URL.to_string(),
        })
    }

    /// Test-only:用記憶體裡的 token + custom endpoints。
    #[doc(hidden)]
    pub fn with_base(
        token: GmailToken,
        oauth_config: OAuthConfig,
        token_path: PathBuf,
        api_base: impl Into<String>,
        token_endpoint: impl Into<String>,
    ) -> Self {
        Self {
            token,
            oauth_config,
            token_path,
            http: reqwest::Client::new(),
            api_base: api_base.into(),
            token_endpoint: token_endpoint.into(),
        }
    }

    /// 取目前 token(僅供測試 / debugging)。
    #[doc(hidden)]
    pub fn token_snapshot(&self) -> &GmailToken {
        &self.token
    }

    /// 每次 API call 前呼叫:過期 → refresh → save → swap 進 `self.token`。
    async fn ensure_fresh_token(&mut self) -> Result<(), GmailError> {
        if !self.token.is_expired() {
            return Ok(());
        }
        tracing::debug!("gmail token expired, refreshing");
        let new_token =
            refresh_token_at(&self.token, &self.oauth_config, &self.token_endpoint).await?;
        new_token.save(&self.token_path)?;
        self.token = new_token;
        Ok(())
    }

    /// `GET /gmail/v1/users/me/threads`(可選 query)。
    pub async fn list_threads(
        &mut self,
        query: Option<&str>,
        max_results: u32,
    ) -> Result<Vec<ThreadSummary>, GmailError> {
        self.ensure_fresh_token().await?;
        let url = format!("{}/gmail/v1/users/me/threads", self.api_base);

        let mut req = self
            .http
            .get(&url)
            .bearer_auth(&self.token.access_token)
            .query(&[("maxResults", max_results.to_string())]);
        if let Some(q) = query {
            req = req.query(&[("q", q)]);
        }

        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(GmailError::Api {
                status: status.as_u16(),
                message: body,
            });
        }

        // Gmail 回 `{ "threads": [...], "resultSizeEstimate": N, "nextPageToken": "..." }`
        // 沒有 thread 時 `threads` 鍵可能不存在,給 default empty vec。
        #[derive(Deserialize)]
        struct ListResp {
            #[serde(default)]
            threads: Vec<ThreadSummary>,
        }
        let parsed: ListResp = resp.json().await?;
        Ok(parsed.threads)
    }

    /// `GET /gmail/v1/users/me/threads/{id}` — 完整 thread,含 messages + decoded text。
    pub async fn get_thread(&mut self, thread_id: &str) -> Result<Thread, GmailError> {
        self.ensure_fresh_token().await?;
        let url = format!("{}/gmail/v1/users/me/threads/{}", self.api_base, thread_id);

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.token.access_token)
            // `format=full` 拿到 payload + bodies。raw / metadata / minimal 都拿不到 body。
            .query(&[("format", "full")])
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(GmailError::Api {
                status: status.as_u16(),
                message: body,
            });
        }
        let raw: RawThread = resp.json().await?;
        parse_thread(raw)
    }

    /// `GET /gmail/v1/users/me/labels`。
    pub async fn list_labels(&mut self) -> Result<Vec<Label>, GmailError> {
        self.ensure_fresh_token().await?;
        let url = format!("{}/gmail/v1/users/me/labels", self.api_base);

        let resp = self.http.get(&url).bearer_auth(&self.token.access_token).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(GmailError::Api {
                status: status.as_u16(),
                message: body,
            });
        }
        #[derive(Deserialize)]
        struct ListResp {
            #[serde(default)]
            labels: Vec<Label>,
        }
        let parsed: ListResp = resp.json().await?;
        Ok(parsed.labels)
    }
}

// =================== 解析 Gmail JSON ===================
//
// Gmail message payload 是個樹:
//   payload: { mimeType, headers, body, parts? }
// `parts` 是遞迴(`multipart/*` 才有);每個 part 又有 mimeType / body。
// `body.data` 是 base64url(no padding)的內容字串。

#[derive(Debug, Deserialize)]
struct RawThread {
    id: String,
    #[serde(default)]
    messages: Vec<RawMessage>,
}

#[derive(Debug, Deserialize)]
struct RawMessage {
    id: String,
    #[serde(default)]
    snippet: String,
    /// `internalDate` 是 epoch milliseconds 字串(Google API quirk)。
    #[serde(default, rename = "internalDate")]
    internal_date: String,
    payload: Option<RawPayload>,
}

#[derive(Debug, Deserialize)]
struct RawPayload {
    #[serde(default, rename = "mimeType")]
    mime_type: String,
    #[serde(default)]
    headers: Vec<RawHeader>,
    #[serde(default)]
    body: RawBody,
    #[serde(default)]
    parts: Vec<RawPayload>,
}

#[derive(Debug, Deserialize)]
struct RawHeader {
    name: String,
    value: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawBody {
    #[serde(default)]
    data: String,
}

fn parse_thread(raw: RawThread) -> Result<Thread, GmailError> {
    let messages = raw
        .messages
        .into_iter()
        .map(parse_message)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Thread { id: raw.id, messages })
}

fn parse_message(raw: RawMessage) -> Result<Message, GmailError> {
    let snippet = raw.snippet;
    let id = raw.id;

    // internalDate(ms epoch string) → DateTime<Utc>。Gmail 一定會給,但壞訊息
    // 我們不 hard fail,回 epoch 0 — caller 通常會排序、看不到也沒事。
    let date = raw
        .internal_date
        .parse::<i64>()
        .ok()
        .and_then(|ms| chrono::DateTime::<Utc>::from_timestamp_millis(ms))
        .unwrap_or(DateTime::<Utc>::from_timestamp(0, 0).unwrap());

    let mut from = String::new();
    let mut to_list: Vec<String> = Vec::new();
    let mut subject = String::new();
    let mut body_text = String::new();

    if let Some(payload) = raw.payload {
        for h in &payload.headers {
            match h.name.to_ascii_lowercase().as_str() {
                "from" => from = h.value.clone(),
                "to" => {
                    to_list = h
                        .value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                "subject" => subject = h.value.clone(),
                _ => {}
            }
        }
        body_text = extract_plain_text(&payload)?;
    }

    Ok(Message {
        id,
        from,
        to: to_list,
        subject,
        date,
        body_text,
        snippet,
    })
}

/// DFS 找第一個 `text/plain` part,base64url decode 它的 `body.data`。
fn extract_plain_text(payload: &RawPayload) -> Result<String, GmailError> {
    // 1. 自己就是 text/plain
    if payload.mime_type.eq_ignore_ascii_case("text/plain") && !payload.body.data.is_empty() {
        return decode_base64url(&payload.body.data);
    }
    // 2. 遞迴 parts
    for part in &payload.parts {
        let inner = extract_plain_text(part)?;
        if !inner.is_empty() {
            return Ok(inner);
        }
    }
    // 3. 都沒有 → 空字串(不 fail caller)
    Ok(String::new())
}

fn decode_base64url(s: &str) -> Result<String, GmailError> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s.trim())?;
    // body 內容若不是 UTF-8(Google 偶爾會在 text/plain 摻 latin-1 / quoted-printable
    // 殘渣),不 hard fail,用 lossy decode — LLM 拿到「字」比拿到 error 重要。
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn dummy_config() -> OAuthConfig {
        OAuthConfig {
            client_id: "cid".into(),
            client_secret: "csecret".into(),
            redirect_uri: "http://localhost:8765/oauth/callback".into(),
        }
    }

    fn fresh_token() -> GmailToken {
        GmailToken {
            access_token: "ya29.fresh".into(),
            refresh_token: "1//refresh".into(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            scope: crate::oauth::GMAIL_READONLY_SCOPE.into(),
            token_type: "Bearer".into(),
        }
    }

    fn expired_token() -> GmailToken {
        GmailToken {
            access_token: "ya29.stale".into(),
            refresh_token: "1//refresh".into(),
            expires_at: Utc::now() - chrono::Duration::seconds(60),
            scope: crate::oauth::GMAIL_READONLY_SCOPE.into(),
            token_type: "Bearer".into(),
        }
    }

    #[tokio::test]
    async fn list_threads_returns_summaries() {
        let mock = MockServer::start().await;

        let body = serde_json::json!({
            "threads": [
                {"id": "t1", "snippet": "hi from alice", "historyId": "100"},
                {"id": "t2", "snippet": "ping", "historyId": "101"}
            ],
            "resultSizeEstimate": 2
        });
        Mock::given(method("GET"))
            .and(path("/gmail/v1/users/me/threads"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&mock)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("gmail-token.json");
        let mut client = GmailClient::with_base(
            fresh_token(),
            dummy_config(),
            token_path,
            mock.uri(),
            // token endpoint 本測試不會打到,給空字串也行;塞 mock 比較不會誤觸網。
            format!("{}/token", mock.uri()),
        );

        let threads = client.list_threads(None, 10).await.expect("list ok");
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].id, "t1");
        assert_eq!(threads[0].snippet, "hi from alice");
        assert_eq!(threads[0].history_id, "100");
        assert_eq!(threads[1].id, "t2");
    }

    #[tokio::test]
    async fn list_threads_returns_api_error_for_4xx() {
        let mock = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/gmail/v1/users/me/threads"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string(
                    r#"{"error":{"code":401,"message":"Invalid Credentials"}}"#,
                ),
            )
            .mount(&mock)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("gmail-token.json");
        let mut client = GmailClient::with_base(
            fresh_token(),
            dummy_config(),
            token_path,
            mock.uri(),
            format!("{}/token", mock.uri()),
        );

        let err = client.list_threads(None, 10).await.expect_err("must 401");
        match err {
            GmailError::Api { status, message } => {
                assert_eq!(status, 401);
                assert!(message.contains("Invalid Credentials"), "msg={message}");
            }
            other => panic!("expected Api 401, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_thread_decodes_base64_body() {
        let mock = MockServer::start().await;

        // base64url(no padding)of "Hello from Mori\nThis is a test body."
        // 用 URL_SAFE_NO_PAD encode 出來。
        let plain = "Hello from Mori\nThis is a test body.";
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(plain.as_bytes());

        let body = serde_json::json!({
            "id": "thread-123",
            "messages": [
                {
                    "id": "msg-1",
                    "snippet": "Hello from Mori",
                    "internalDate": "1716000000000",
                    "payload": {
                        "mimeType": "text/plain",
                        "headers": [
                            {"name": "From", "value": "Alice <alice@example.com>"},
                            {"name": "To", "value": "bob@example.com, carol@example.com"},
                            {"name": "Subject", "value": "Test"}
                        ],
                        "body": {"data": encoded}
                    }
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/gmail/v1/users/me/threads/thread-123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&mock)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("gmail-token.json");
        let mut client = GmailClient::with_base(
            fresh_token(),
            dummy_config(),
            token_path,
            mock.uri(),
            format!("{}/token", mock.uri()),
        );

        let thread = client.get_thread("thread-123").await.expect("ok");
        assert_eq!(thread.id, "thread-123");
        assert_eq!(thread.messages.len(), 1);
        let msg = &thread.messages[0];
        assert_eq!(msg.id, "msg-1");
        assert_eq!(msg.from, "Alice <alice@example.com>");
        assert_eq!(msg.to, vec!["bob@example.com", "carol@example.com"]);
        assert_eq!(msg.subject, "Test");
        assert_eq!(msg.body_text, plain);
        assert_eq!(msg.snippet, "Hello from Mori");
    }

    #[tokio::test]
    async fn get_thread_extracts_plain_from_multipart_alternative() {
        let mock = MockServer::start().await;
        let plain = "plain text inside multipart";
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(plain.as_bytes());

        // multipart/alternative → text/plain + text/html;只取 text/plain。
        let body = serde_json::json!({
            "id": "tx",
            "messages": [
                {
                    "id": "m1",
                    "snippet": "snip",
                    "internalDate": "1716000000000",
                    "payload": {
                        "mimeType": "multipart/alternative",
                        "headers": [{"name": "Subject", "value": "S"}],
                        "parts": [
                            {
                                "mimeType": "text/plain",
                                "body": {"data": encoded}
                            },
                            {
                                "mimeType": "text/html",
                                "body": {"data": "PHA+aHRtbDwvcD4"}  // <p>html</p>
                            }
                        ]
                    }
                }
            ]
        });
        Mock::given(method("GET"))
            .and(path("/gmail/v1/users/me/threads/tx"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&mock)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let mut client = GmailClient::with_base(
            fresh_token(),
            dummy_config(),
            dir.path().join("gmail-token.json"),
            mock.uri(),
            format!("{}/token", mock.uri()),
        );
        let thread = client.get_thread("tx").await.expect("ok");
        assert_eq!(thread.messages[0].body_text, plain);
    }

    #[tokio::test]
    async fn ensure_fresh_token_refreshes_when_expired() {
        let mock = MockServer::start().await;

        // 1. Token endpoint 回新 access_token
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.refreshed",
                "expires_in": 3599,
                "scope": crate::oauth::GMAIL_READONLY_SCOPE,
                "token_type": "Bearer"
            })))
            .mount(&mock)
            .await;

        // 2. labels.list 回空 list
        Mock::given(method("GET"))
            .and(path("/gmail/v1/users/me/labels"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"labels": []})),
            )
            .mount(&mock)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("gmail-token.json");
        let mut client = GmailClient::with_base(
            expired_token(),  // 故意過期
            dummy_config(),
            token_path.clone(),
            mock.uri(),
            format!("{}/token", mock.uri()),
        );

        // call API → 應該觸發 refresh
        let labels = client.list_labels().await.expect("ok");
        assert!(labels.is_empty());

        // token 已更新進 client
        assert_eq!(client.token_snapshot().access_token, "ya29.refreshed");
        assert!(!client.token_snapshot().is_expired());

        // 也已 save 到 disk
        let saved = GmailToken::load(&token_path).expect("token saved to disk");
        assert_eq!(saved.access_token, "ya29.refreshed");
        // refresh response 沒給新 refresh_token → 沿用舊的
        assert_eq!(saved.refresh_token, "1//refresh");
    }

    #[test]
    fn extract_plain_text_returns_empty_when_no_plain_part() {
        // Only text/html → 沒有 text/plain → 空字串(不 fail)
        let payload = RawPayload {
            mime_type: "text/html".into(),
            headers: vec![],
            body: RawBody { data: "PGgxPmhpPC9oMT4".into() },  // <h1>hi</h1>
            parts: vec![],
        };
        let got = extract_plain_text(&payload).expect("ok");
        assert!(got.is_empty(), "html-only should yield empty plain text, got: {got}");
    }

    #[test]
    fn decode_base64url_handles_unpadded() {
        // base64url 沒 padding 也要能 decode。
        // "Hi" -> "SGk"(沒 = padding)
        let got = decode_base64url("SGk").expect("ok");
        assert_eq!(got, "Hi");
    }
}
