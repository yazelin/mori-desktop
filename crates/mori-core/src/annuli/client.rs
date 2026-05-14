//! `AnnuliClient` — reqwest 包裝,對應 Wave 3 annuli HTTP API。

use chrono::{DateTime, FixedOffset};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

/// Annuli client config(來自 `~/.mori/config.json` `annuli` 段)。
#[derive(Debug, Clone)]
pub struct AnnuliClientConfig {
    /// e.g., `"http://localhost:5000"`。**不含 trailing slash**。
    pub endpoint: String,
    /// e.g., `"mori"`。
    pub spirit_name: String,
    /// vault `identity/user_id`,給 events.append / rings/new 用。
    pub user_id: String,
    /// optional `X-Soul-Token`(只有 PUT /soul 需要)。
    pub soul_token: Option<String>,
    /// optional `(user, pass)` for basic auth(`ANNULI_ADMIN_USER` / `ANNULI_ADMIN_PASS`)。
    pub basic_auth: Option<(String, String)>,
    /// request timeout。預設 10s,可調。
    pub timeout: Duration,
}

impl AnnuliClientConfig {
    /// Minimal localhost dev config(無 basic auth、無 soul_token、預設 timeout)。
    pub fn local(endpoint: impl Into<String>, spirit_name: impl Into<String>, user_id: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            spirit_name: spirit_name.into(),
            user_id: user_id.into(),
            soul_token: None,
            basic_auth: None,
            timeout: Duration::from_secs(10),
        }
    }
}

/// AnnuliClient errors(透過 [`AnnuliError`] expose,**不洩漏 token**)。
#[derive(Debug, Error)]
pub enum AnnuliError {
    #[error("annuli HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("annuli {endpoint} returned {status}: {body}")]
    Status { endpoint: String, status: u16, body: String },
    #[error("annuli response parse failed: {0}")]
    Parse(String),
    #[error("config error: {0}")]
    Config(String),
}

/// Health probe response.
#[derive(Debug, Deserialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub soul_token_configured: bool,
    pub vault_dir: String,
}

/// One event entry (mirror of annuli's Event but as plain serde struct).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub ts: DateTime<FixedOffset>,
    pub kind: String,
    pub user_id: String,
    pub source: String,
    pub data: serde_json::Value,
    /// `(date_str, line_no)` tuple,annuli 回傳的 id。
    pub id: Option<(String, u32)>,
}

#[derive(Debug, Deserialize)]
struct EventListResponse {
    events: Vec<EventRecord>,
}

#[derive(Debug, Deserialize)]
struct EventAppendResponse {
    ok: bool,
    event_id: (String, u32),
}

#[derive(Debug, Deserialize)]
struct OkResponse {
    #[allow(dead_code)]
    ok: bool,
    #[serde(default)]
    ring_path: Option<String>,
    #[serde(default)]
    report_path: Option<String>,
    #[serde(default)]
    applied: Option<u32>,
    #[serde(default)]
    vault_path: Option<String>,
    #[serde(default)]
    bytes_written: Option<u32>,
}

/// HTTP client for annuli's Wave 3 API.
pub struct AnnuliClient {
    config: AnnuliClientConfig,
    http: Client,
}

impl AnnuliClient {
    pub fn new(config: AnnuliClientConfig) -> Result<Self, AnnuliError> {
        if config.endpoint.is_empty() {
            return Err(AnnuliError::Config("endpoint 不能空".into()));
        }
        if config.endpoint.ends_with('/') {
            return Err(AnnuliError::Config(
                "endpoint 不該以 `/` 結尾(會跟 path concat 出 //)".into(),
            ));
        }
        if config.spirit_name.is_empty() {
            return Err(AnnuliError::Config("spirit_name 不能空".into()));
        }

        let http = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(AnnuliError::Http)?;
        Ok(Self { config, http })
    }

    /// `<endpoint>/spirits/<spirit>/<rel>`。
    fn url(&self, rel: &str) -> String {
        format!("{}/spirits/{}{}", self.config.endpoint, self.config.spirit_name, rel)
    }

    /// `<endpoint>/<rel>`(沒 spirits prefix,例如 `/health`)。
    fn root_url(&self, rel: &str) -> String {
        format!("{}{}", self.config.endpoint, rel)
    }

    fn auth_request(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some((u, p)) = &self.config.basic_auth {
            req = req.basic_auth(u, Some(p));
        }
        req
    }

    /// Apply token header for PUT /soul (only). Other routes 不該加,避免漏 token。
    fn with_soul_token(&self, req: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder, AnnuliError> {
        match &self.config.soul_token {
            Some(t) if !t.is_empty() => Ok(req.header("X-Soul-Token", t)),
            _ => Err(AnnuliError::Config(
                "PUT /soul 需要 soul_token,~/.mori/config.json 沒設".into(),
            )),
        }
    }

    /// 將 response 轉成 error 若 non-2xx,**不打 log token**。
    async fn check_status(&self, endpoint: &str, resp: reqwest::Response) -> Result<reqwest::Response, AnnuliError> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        // truncate body 避免 log 變超長
        let body_trunc = if body.len() > 500 { format!("{}…(truncated)", &body[..500]) } else { body };
        Err(AnnuliError::Status {
            endpoint: endpoint.to_string(),
            status: status.as_u16(),
            body: body_trunc,
        })
    }

    // === Routes ===

    /// `GET /health` — heartbeat。回 `HealthResponse`。
    pub async fn health(&self) -> Result<HealthResponse, AnnuliError> {
        let url = self.root_url("/health");
        let req = self.auth_request(self.http.get(&url));
        let resp = req.send().await?;
        let resp = self.check_status("/health", resp).await?;
        let h: HealthResponse = resp.json().await.map_err(|e| AnnuliError::Parse(e.to_string()))?;
        Ok(h)
    }

    /// `POST /spirits/<x>/bootstrap` — ensure vault structure. Idempotent.
    pub async fn bootstrap(&self) -> Result<String, AnnuliError> {
        let url = self.url("/bootstrap");
        let req = self.auth_request(self.http.post(&url));
        let resp = req.send().await?;
        let resp = self.check_status("/bootstrap", resp).await?;
        let ok: OkResponse = resp.json().await.map_err(|e| AnnuliError::Parse(e.to_string()))?;
        ok.vault_path.ok_or_else(|| AnnuliError::Parse("missing vault_path".into()))
    }

    /// `GET /spirits/<x>/soul` — read SOUL.md as plain text.
    pub async fn get_soul(&self) -> Result<String, AnnuliError> {
        let url = self.url("/soul");
        let req = self.auth_request(self.http.get(&url));
        let resp = req.send().await?;
        let resp = self.check_status("/soul", resp).await?;
        Ok(resp.text().await?)
    }

    /// `PUT /spirits/<x>/soul` — write SOUL.md. Requires `soul_token` in config (else 配置錯誤).
    pub async fn put_soul(&self, body: &str) -> Result<u32, AnnuliError> {
        let url = self.url("/soul");
        let mut req = self.auth_request(self.http.put(&url).body(body.to_string()));
        req = self.with_soul_token(req)?;
        let resp = req.send().await?;
        let resp = self.check_status("/soul", resp).await?;
        let ok: OkResponse = resp.json().await.map_err(|e| AnnuliError::Parse(e.to_string()))?;
        ok.bytes_written.ok_or_else(|| AnnuliError::Parse("missing bytes_written".into()))
    }

    /// `POST /spirits/<x>/events` — append one event. Returns `(date_str, line_no)`.
    pub async fn append_event(
        &self,
        kind: &str,
        source: &str,
        data: serde_json::Value,
    ) -> Result<(String, u32), AnnuliError> {
        let url = self.url("/events");
        let body = serde_json::json!({
            "user_id": self.config.user_id,
            "kind": kind,
            "source": source,
            "data": data,
        });
        let req = self.auth_request(self.http.post(&url).json(&body));
        let resp = req.send().await?;
        let resp = self.check_status("/events", resp).await?;
        let r: EventAppendResponse = resp.json().await.map_err(|e| AnnuliError::Parse(e.to_string()))?;
        if !r.ok {
            return Err(AnnuliError::Parse("ok=false in events append response".into()));
        }
        Ok(r.event_id)
    }

    /// `GET /spirits/<x>/events?date=YYYY-MM-DD`
    pub async fn list_events_by_date(&self, date: &str) -> Result<Vec<EventRecord>, AnnuliError> {
        let url = format!("{}?date={}", self.url("/events"), urlencode(date));
        self.fetch_events(&url).await
    }

    /// `GET /spirits/<x>/events?q=<query>` (FTS5 trigram,需 ≥3 字)
    pub async fn search_events(&self, query: &str, limit: u32) -> Result<Vec<EventRecord>, AnnuliError> {
        let url = format!("{}?q={}&limit={}", self.url("/events"), urlencode(query), limit);
        self.fetch_events(&url).await
    }

    /// `GET /spirits/<x>/events?kind=<kind>`
    pub async fn list_events_by_kind(&self, kind: &str) -> Result<Vec<EventRecord>, AnnuliError> {
        let url = format!("{}?kind={}", self.url("/events"), urlencode(kind));
        self.fetch_events(&url).await
    }

    async fn fetch_events(&self, url: &str) -> Result<Vec<EventRecord>, AnnuliError> {
        let req = self.auth_request(self.http.get(url));
        let resp = req.send().await?;
        let resp = self.check_status("/events", resp).await?;
        let r: EventListResponse = resp.json().await.map_err(|e| AnnuliError::Parse(e.to_string()))?;
        Ok(r.events)
    }

    /// `POST /spirits/<x>/rings/new` — trigger do_sleep, returns ring path.
    pub async fn trigger_sleep(&self) -> Result<String, AnnuliError> {
        let url = self.url("/rings/new");
        let body = serde_json::json!({ "user_id": self.config.user_id });
        let req = self.auth_request(self.http.post(&url).json(&body));
        let resp = req.send().await?;
        let resp = self.check_status("/rings/new", resp).await?;
        let ok: OkResponse = resp.json().await.map_err(|e| AnnuliError::Parse(e.to_string()))?;
        ok.ring_path.ok_or_else(|| AnnuliError::Parse("missing ring_path".into()))
    }

    /// `POST /spirits/<x>/curator/dry-run` — returns report path.
    pub async fn curator_dry_run(&self) -> Result<String, AnnuliError> {
        let url = self.url("/curator/dry-run");
        let req = self.auth_request(self.http.post(&url));
        let resp = req.send().await?;
        let resp = self.check_status("/curator/dry-run", resp).await?;
        let ok: OkResponse = resp.json().await.map_err(|e| AnnuliError::Parse(e.to_string()))?;
        ok.report_path.ok_or_else(|| AnnuliError::Parse("missing report_path".into()))
    }

    /// `POST /spirits/<x>/curator/apply` — returns applied count.
    pub async fn curator_apply(&self, report_path: &str) -> Result<u32, AnnuliError> {
        let url = self.url("/curator/apply");
        let body = serde_json::json!({ "report_path": report_path });
        let req = self.auth_request(self.http.post(&url).json(&body));
        let resp = req.send().await?;
        let resp = self.check_status("/curator/apply", resp).await?;
        let ok: OkResponse = resp.json().await.map_err(|e| AnnuliError::Parse(e.to_string()))?;
        ok.applied.ok_or_else(|| AnnuliError::Parse("missing applied count".into()))
    }
}

/// Minimal URL-encode for query params (避免 pull urlencoding crate)。
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

// Unit tests for AnnuliClient — config validation + URL construction.
// Integration tests(實際 HTTP)在 tests/integration_annuli.rs(Wave 4 step 11
// 才寫,需要真實 annuli server 起來)。

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn local_config() -> AnnuliClientConfig {
        AnnuliClientConfig::local("http://localhost:5000", "mori", "yazelin")
    }

    #[test]
    fn config_local_helper_sets_defaults() {
        let c = local_config();
        assert_eq!(c.endpoint, "http://localhost:5000");
        assert_eq!(c.spirit_name, "mori");
        assert_eq!(c.user_id, "yazelin");
        assert!(c.soul_token.is_none());
        assert!(c.basic_auth.is_none());
        assert_eq!(c.timeout, Duration::from_secs(10));
    }

    #[test]
    fn new_rejects_empty_endpoint() {
        let mut c = local_config();
        c.endpoint = String::new();
        let r = AnnuliClient::new(c);
        assert!(matches!(r, Err(AnnuliError::Config(_))));
    }

    #[test]
    fn new_rejects_trailing_slash_endpoint() {
        let mut c = local_config();
        c.endpoint = "http://localhost:5000/".into();
        let r = AnnuliClient::new(c);
        assert!(matches!(r, Err(AnnuliError::Config(_))));
    }

    #[test]
    fn new_rejects_empty_spirit_name() {
        let mut c = local_config();
        c.spirit_name = String::new();
        let r = AnnuliClient::new(c);
        assert!(matches!(r, Err(AnnuliError::Config(_))));
    }

    #[test]
    fn new_accepts_valid_minimal_config() {
        let r = AnnuliClient::new(local_config());
        assert!(r.is_ok());
    }

    // === URL construction(通過 internal API 暴露,test inside same module)===

    #[test]
    fn url_construction() {
        let client = AnnuliClient::new(local_config()).unwrap();
        assert_eq!(client.url("/soul"), "http://localhost:5000/spirits/mori/soul");
        assert_eq!(
            client.url("/events?date=2026-05-14"),
            "http://localhost:5000/spirits/mori/events?date=2026-05-14"
        );
        assert_eq!(client.url("/rings/new"), "http://localhost:5000/spirits/mori/rings/new");
        assert_eq!(
            client.url("/curator/dry-run"),
            "http://localhost:5000/spirits/mori/curator/dry-run"
        );
    }

    #[test]
    fn root_url_no_spirit_prefix() {
        let client = AnnuliClient::new(local_config()).unwrap();
        assert_eq!(client.root_url("/health"), "http://localhost:5000/health");
    }

    #[test]
    fn url_with_different_spirit_name() {
        let mut c = local_config();
        c.spirit_name = "scribe".into();
        let client = AnnuliClient::new(c).unwrap();
        assert_eq!(client.url("/soul"), "http://localhost:5000/spirits/scribe/soul");
    }

    // === Soul token enforcement ===

    #[test]
    fn put_soul_without_token_rejected_at_config_layer() {
        // No async runtime needed — with_soul_token 是純邏輯
        let client = AnnuliClient::new(local_config()).unwrap();
        let req = client.http.put("http://localhost:5000/spirits/mori/soul");
        let result = client.with_soul_token(req);
        assert!(matches!(result, Err(AnnuliError::Config(_))));
    }

    #[test]
    fn put_soul_with_token_adds_header() {
        let mut c = local_config();
        c.soul_token = Some("super-secret".into());
        let client = AnnuliClient::new(c).unwrap();
        let req = client.http.put("http://localhost:5000/spirits/mori/soul");
        let result = client.with_soul_token(req);
        assert!(result.is_ok());
        // header 加上去後可以 build 出 request(reqwest 內部驗 header)
        let _ = result.unwrap().build().expect("should build OK");
    }

    // === URL-encode helper ===

    #[test]
    fn urlencode_alphanumeric_passthrough() {
        assert_eq!(urlencode("hello123"), "hello123");
        assert_eq!(urlencode("foo-bar_baz.qux~"), "foo-bar_baz.qux~");
    }

    #[test]
    fn urlencode_chinese() {
        assert_eq!(urlencode("森林"), "%E6%A3%AE%E6%9E%97");
    }

    #[test]
    fn urlencode_special_chars() {
        assert_eq!(urlencode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(urlencode("a/b"), "a%2Fb");
    }
}
