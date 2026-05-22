//! Gmail 系列 LLM-callable skill(Wave 8 Gm-2「跨界之手」)。
//!
//! 三個 skill 共用一份 [`mori_gmail::GmailClient`](Arc<Mutex<>>)。Client 內含 token
//! state(過期會自動 refresh + save 回 disk),`ensure_fresh_token` 需要 `&mut self`,
//! 所以 client 用 `Arc<Mutex<GmailClient>>` 包,所有 skill share 同一份。
//!
//! ## Skill 列表
//!
//! - [`ListGmailSkill`] — `list_gmail(query?, max?)`:列 thread summary。
//! - [`ReadGmailSkill`] — `read_gmail(thread_id)`:展開 thread 全文(messages + bodies)。
//! - [`SendGmailSkill`] — `send_gmail(to, subject, body, reply_to_thread_id?)`:寄信
//!   或回某條 thread。Send 需要 `gmail.send` scope — `execute` 進場前先 check
//!   token 是否含此 scope,沒有就回 error 提示 user 跑 OAuth re-consent。
//!
//! ## 用 Arc<Mutex<>> 而非 Arc 的原因
//!
//! `GmailClient::list_threads / get_thread / send_message` 都吃 `&mut self`
//! (`ensure_fresh_token` 內可能更新 token state)。多個 skill 同進程要 share
//! 一份 client → interior mutability 用 `tokio::sync::Mutex`(async 環境 — 不能用
//! `std::sync::Mutex`,await 跨 lock 會 deadlock)。
//!
//! ## 為什麼分 3 個 skill 而不是 1 個 method-dispatch
//!
//! 對齊既有 `RemindMeSkill` 一個動作一個 skill 的 pattern,讓 LLM tool definition
//! 描述精準(各 skill 自己 schema + description),減少 LLM 看「method 字串」誤
//! invoke 的機率。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use mori_gmail::{GmailClient, GMAIL_SEND_SCOPE};

use super::{ExecutionTarget, Privacy, Skill, SkillOutput};
use crate::context::Context;

/// 共用 client wrapper — 也是 mori-tauri 端 `.manage(SharedGmailClient(...))` 的型別。
/// 讓 Tauri command + 各 Skill 都從同一個 `Arc<Mutex<GmailClient>>` 拿 clone。
#[derive(Clone)]
pub struct SharedGmailClient(pub Arc<Mutex<GmailClient>>);

// ─────────────────────────────────────────────────────────────────────
// List
// ─────────────────────────────────────────────────────────────────────

pub struct ListGmailSkill {
    pub client: Arc<Mutex<GmailClient>>,
}

impl ListGmailSkill {
    pub fn new(client: Arc<Mutex<GmailClient>>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Skill for ListGmailSkill {
    fn name(&self) -> &'static str {
        "list_gmail"
    }

    fn description(&self) -> &'static str {
        "列我最近的 Gmail thread(summary;snippet + history id,沒展開 body)。\
         可選 query 用 Gmail search 語法(`from:alice`、`is:unread`、`subject:meeting`、\
         `after:2026/01/01` 等)。預設 max=10。要看 thread 全文呼叫 `read_gmail(thread_id)`。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Gmail 搜尋語法,例如 `is:unread`、`from:bob@x.com`、`subject:report`。空字串 / 省略 = 全部最近。"
                },
                "max": {
                    "type": "integer",
                    "description": "回傳上限(預設 10)。LLM 用,別超過 50。",
                    "minimum": 1,
                    "maximum": 100
                }
            }
        })
    }

    fn target_capability(&self) -> ExecutionTarget {
        ExecutionTarget::Local
    }

    fn privacy(&self) -> Privacy {
        Privacy::Cloud
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let max = args
            .get("max")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .unwrap_or(10);

        tracing::info!(query = ?query, max, "list_gmail skill");

        let summaries = {
            let mut client = self.client.lock().await;
            client
                .list_threads(query.as_deref(), max)
                .await
                .map_err(|e| anyhow!("list_gmail failed: {e}"))?
        };

        // user_message:列每 thread 一行 — id + snippet。LLM 拿到後通常會挑一條
        // 講給 user 聽 / 進一步 read_gmail。
        let user_message = if summaries.is_empty() {
            "(沒有符合的 Gmail thread)".to_string()
        } else {
            let mut s = String::new();
            for t in &summaries {
                s.push_str(&format!("- {} | {}\n", t.id, t.snippet));
            }
            s
        };

        Ok(SkillOutput {
            user_message,
            data: Some(json!({
                "count": summaries.len(),
                "threads": summaries,
            })),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
// Read
// ─────────────────────────────────────────────────────────────────────

pub struct ReadGmailSkill {
    pub client: Arc<Mutex<GmailClient>>,
}

impl ReadGmailSkill {
    pub fn new(client: Arc<Mutex<GmailClient>>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Skill for ReadGmailSkill {
    fn name(&self) -> &'static str {
        "read_gmail"
    }

    fn description(&self) -> &'static str {
        "讀一條 Gmail thread 的完整訊息(每封 message 的 from / to / subject / date / body)。\
         `thread_id` 通常從 `list_gmail` 結果拿。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": {
                    "type": "string",
                    "description": "Gmail thread id(從 `list_gmail` 結果拿)。"
                }
            },
            "required": ["thread_id"]
        })
    }

    fn target_capability(&self) -> ExecutionTarget {
        ExecutionTarget::Local
    }

    fn privacy(&self) -> Privacy {
        Privacy::Cloud
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let thread_id = args
            .get("thread_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing thread_id"))?
            .trim()
            .to_string();
        if thread_id.is_empty() {
            return Err(anyhow!("thread_id is empty"));
        }

        tracing::info!(thread_id = %thread_id, "read_gmail skill");

        let thread = {
            let mut client = self.client.lock().await;
            client
                .get_thread(&thread_id)
                .await
                .map_err(|e| anyhow!("read_gmail failed: {e}"))?
        };

        // user_message:逐封 dump 給 LLM,LLM 自己決定怎麼摘要 / 答 user。
        let mut s = String::new();
        s.push_str(&format!("Thread {}:\n\n", thread.id));
        for (i, m) in thread.messages.iter().enumerate() {
            s.push_str(&format!(
                "--- Message {} ---\nFrom: {}\nTo: {}\nDate: {}\nSubject: {}\n\n{}\n\n",
                i + 1,
                m.from,
                m.to.join(", "),
                m.date.to_rfc3339(),
                m.subject,
                m.body_text,
            ));
        }

        Ok(SkillOutput {
            user_message: s,
            data: Some(serde_json::to_value(&thread)?),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
// Send
// ─────────────────────────────────────────────────────────────────────

pub struct SendGmailSkill {
    pub client: Arc<Mutex<GmailClient>>,
}

impl SendGmailSkill {
    pub fn new(client: Arc<Mutex<GmailClient>>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Skill for SendGmailSkill {
    fn name(&self) -> &'static str {
        "send_gmail"
    }

    fn description(&self) -> &'static str {
        "代亞澤寄一封 Gmail。to 是 recipient list(可多人),subject / body 必填。\
         可選 `reply_to_thread_id` 把這封接到既有 thread 上(會自動加 In-Reply-To header)。\
         需要 `gmail.send` scope — 沒授權會回 error,user 需要重跑 OAuth flow 升 scope。\
         寫之前**先口頭跟 user 確認內容**(寄出就收不回來)。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "收件人 email list(至少一個)。"
                },
                "subject": {
                    "type": "string",
                    "description": "信件主旨。reply 的話 caller 自己加 `Re: ` prefix。"
                },
                "body": {
                    "type": "string",
                    "description": "信件純文字內容(LLM 寫好的)。"
                },
                "reply_to_thread_id": {
                    "type": "string",
                    "description": "可選 — 若是回覆某條既有 thread,傳那條 thread 的 id(從 `list_gmail` / `read_gmail` 拿)。\
                                    也應從原 message 拿 `message_id` 一起傳 `in_reply_to`(同欄位)— 本層簡化只接 thread_id,\
                                    `In-Reply-To` 用 thread_id 當 placeholder(Gmail thread 機制本身用 threadId 串)。"
                },
                "in_reply_to": {
                    "type": "string",
                    "description": "可選 — 原信的 Message-ID header(讀 thread 後從 message data 拿)。\
                                    若給,會放進 In-Reply-To / References header,對方 mail client 才會顯示為「reply」格式。"
                }
            },
            "required": ["to", "subject", "body"]
        })
    }

    fn target_capability(&self) -> ExecutionTarget {
        ExecutionTarget::Local
    }

    fn privacy(&self) -> Privacy {
        Privacy::Cloud
    }

    /// 寄信是 destructive(寄出去收不回),要 LLM 在 UI 二次確認。
    fn confirm_required(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        // 1. parse args
        let to: Vec<String> = args
            .get("to")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("missing to (must be array)"))?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect();
        if to.is_empty() {
            return Err(anyhow!("to is empty"));
        }
        let subject = args
            .get("subject")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing subject"))?
            .to_string();
        let body = args
            .get("body")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing body"))?
            .to_string();
        let reply_thread_id = args
            .get("reply_to_thread_id")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let in_reply_to = args
            .get("in_reply_to")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        // 2. check send scope before打 API — 沒授權直接回明確 error,user 看了
        //    才知道要重跑 OAuth flow。Google 本身會 401 / 403 但 error message
        //    含 "insufficient_scope" 對 LLM 不夠直觀。
        {
            let client = self.client.lock().await;
            if !client.token_snapshot().has_scope(GMAIL_SEND_SCOPE) {
                return Err(anyhow!(
                    "send_gmail requires `{}` scope but current token doesn't have it. \
                     Please re-run OAuth flow with send scope (Mori 設定頁 → Gmail OAuth → \
                     Re-authorize with send permission).",
                    GMAIL_SEND_SCOPE,
                ));
            }
        }

        tracing::info!(
            to_count = to.len(),
            has_thread = reply_thread_id.is_some(),
            subject_chars = subject.chars().count(),
            body_chars = body.chars().count(),
            "send_gmail skill",
        );

        // 3. send or reply
        let outcome = {
            let mut client = self.client.lock().await;
            match (reply_thread_id.as_deref(), in_reply_to.as_deref()) {
                (Some(tid), Some(irt)) => client
                    .send_reply(tid, &to, &subject, &body, irt)
                    .await
                    .map_err(|e| anyhow!("send_reply failed: {e}"))?,
                (Some(tid), None) => {
                    // 有 thread_id 但沒 in_reply_to — 用 thread_id 當 placeholder。
                    // Gmail thread 串接靠 threadId(server-side),Header 主要給對方
                    // mail client 認 reply 關係,placeholder 仍比沒 header 好。
                    let placeholder = format!("<{tid}@thread.placeholder>");
                    client
                        .send_reply(tid, &to, &subject, &body, &placeholder)
                        .await
                        .map_err(|e| anyhow!("send_reply failed: {e}"))?
                }
                _ => client
                    .send_message(&to, &subject, &body)
                    .await
                    .map_err(|e| anyhow!("send_message failed: {e}"))?,
            }
        };

        let user_message = if reply_thread_id.is_some() {
            format!(
                "寄出回覆了。message id {} / thread {}。",
                outcome.id, outcome.thread_id,
            )
        } else {
            format!(
                "寄出了。message id {} / thread {}。",
                outcome.id, outcome.thread_id,
            )
        };

        Ok(SkillOutput {
            user_message,
            data: Some(json!({
                "id": outcome.id,
                "thread_id": outcome.thread_id,
                "to": to,
                "subject": subject,
            })),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Skill-level tests — 直接組 `Arc<Mutex<GmailClient>>` 內部用 `GmailClient::with_base`
    //! 注入 wiremock,驗 dispatch 路徑活著 + args 驗證 + scope guard。

    use super::*;
    use crate::context::Context as MoriContext;
    use base64::Engine as _;
    use chrono::Utc;
    use mori_gmail::{GmailToken, OAuthConfig, GMAIL_READONLY_SCOPE};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn empty_context() -> MoriContext {
        MoriContext::default()
    }

    fn dummy_oauth() -> OAuthConfig {
        OAuthConfig {
            client_id: "cid".into(),
            client_secret: "csecret".into(),
            redirect_uri: "http://localhost:8765/oauth/callback".into(),
        }
    }

    /// token 帶 readonly + send scope(Gm-2 升級後狀態)。
    fn token_full_scope() -> GmailToken {
        GmailToken {
            access_token: "ya29.full".into(),
            refresh_token: "1//refresh".into(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            scope: format!("{GMAIL_READONLY_SCOPE} {GMAIL_SEND_SCOPE}"),
            token_type: "Bearer".into(),
        }
    }

    /// token 只有 readonly(Gm-1 token 沒升級)。
    fn token_readonly_only() -> GmailToken {
        GmailToken {
            access_token: "ya29.read".into(),
            refresh_token: "1//refresh".into(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            scope: GMAIL_READONLY_SCOPE.into(),
            token_type: "Bearer".into(),
        }
    }

    fn make_client(mock_uri: String, token: GmailToken) -> Arc<Mutex<GmailClient>> {
        let dir = tempfile::tempdir().unwrap();
        // tempdir 在 fn return 前 drop — 但 GmailClient 只在 refresh 時寫 disk,
        // 測試的 token 都不會過期,不會 trigger 寫;tempdir drop 也只刪空目錄,
        // 安全。若日後 refresh 路徑要測,改成把 TempDir 持給 Arc 同生命週期。
        let token_path = dir.path().join("gmail-token.json");
        let token_endpoint = format!("{mock_uri}/token");
        let client = GmailClient::with_base(token, dummy_oauth(), token_path, mock_uri, token_endpoint);
        Arc::new(Mutex::new(client))
    }

    // ── ListGmailSkill ─────────────────────────────────────────────

    #[tokio::test]
    async fn list_gmail_skill_returns_summaries() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/gmail/v1/users/me/threads"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "threads": [
                    {"id": "t1", "snippet": "hi", "historyId": "100"},
                    {"id": "t2", "snippet": "ping", "historyId": "101"}
                ]
            })))
            .mount(&mock)
            .await;

        let client = make_client(mock.uri(), token_full_scope());
        let skill = ListGmailSkill::new(client);
        let out = skill
            .execute(serde_json::json!({"max": 5}), &empty_context())
            .await
            .expect("list ok");
        assert!(out.user_message.contains("t1"));
        assert!(out.user_message.contains("hi"));
        let data = out.data.expect("data present");
        assert_eq!(data["count"], 2);
        assert_eq!(data["threads"][0]["id"], "t1");
    }

    #[tokio::test]
    async fn list_gmail_skill_returns_empty_message_when_no_threads() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/gmail/v1/users/me/threads"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&mock)
            .await;

        let client = make_client(mock.uri(), token_full_scope());
        let skill = ListGmailSkill::new(client);
        let out = skill
            .execute(serde_json::json!({}), &empty_context())
            .await
            .expect("ok");
        assert!(out.user_message.contains("沒有"), "got: {}", out.user_message);
    }

    // ── ReadGmailSkill ─────────────────────────────────────────────

    #[tokio::test]
    async fn read_gmail_skill_dumps_thread_messages() {
        let mock = MockServer::start().await;
        let body_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"hello body");
        Mock::given(method("GET"))
            .and(path("/gmail/v1/users/me/threads/t-xyz"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "t-xyz",
                "messages": [{
                    "id": "m1",
                    "snippet": "snip",
                    "internalDate": "1716000000000",
                    "payload": {
                        "mimeType": "text/plain",
                        "headers": [
                            {"name": "From", "value": "Alice <alice@x.com>"},
                            {"name": "To", "value": "bob@x.com"},
                            {"name": "Subject", "value": "Hi"}
                        ],
                        "body": {"data": body_b64}
                    }
                }]
            })))
            .mount(&mock)
            .await;

        let client = make_client(mock.uri(), token_full_scope());
        let skill = ReadGmailSkill::new(client);
        let out = skill
            .execute(serde_json::json!({"thread_id": "t-xyz"}), &empty_context())
            .await
            .expect("read ok");
        assert!(out.user_message.contains("Alice"));
        assert!(out.user_message.contains("hello body"));
        assert!(out.user_message.contains("Hi"));
        let data = out.data.expect("data");
        assert_eq!(data["id"], "t-xyz");
        assert_eq!(data["messages"][0]["subject"], "Hi");
    }

    #[tokio::test]
    async fn read_gmail_skill_errors_on_missing_thread_id() {
        // 不 hit mock — 應該在 arg validation 階段就 fail
        let mock = MockServer::start().await;
        let client = make_client(mock.uri(), token_full_scope());
        let skill = ReadGmailSkill::new(client);
        let err = skill
            .execute(serde_json::json!({}), &empty_context())
            .await
            .expect_err("missing thread_id");
        assert!(err.to_string().contains("thread_id"));
    }

    // ── SendGmailSkill ─────────────────────────────────────────────

    #[tokio::test]
    async fn send_gmail_skill_sends_simple_message() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/gmail/v1/users/me/messages/send"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "sent-1",
                "threadId": "thread-new"
            })))
            .mount(&mock)
            .await;

        let client = make_client(mock.uri(), token_full_scope());
        let skill = SendGmailSkill::new(client);
        let out = skill
            .execute(
                serde_json::json!({
                    "to": ["bob@x.com"],
                    "subject": "hi",
                    "body": "hello"
                }),
                &empty_context(),
            )
            .await
            .expect("send ok");
        assert!(out.user_message.contains("sent-1"));
        let data = out.data.expect("data");
        assert_eq!(data["id"], "sent-1");
        assert_eq!(data["thread_id"], "thread-new");
    }

    #[tokio::test]
    async fn send_gmail_skill_blocks_when_token_missing_send_scope() {
        // mock 即使有也不該被 hit — scope guard 先 fail
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/gmail/v1/users/me/messages/send"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "should-not-happen",
                "threadId": "x"
            })))
            .mount(&mock)
            .await;

        let client = make_client(mock.uri(), token_readonly_only());
        let skill = SendGmailSkill::new(client);
        let err = skill
            .execute(
                serde_json::json!({
                    "to": ["bob@x.com"],
                    "subject": "hi",
                    "body": "hello"
                }),
                &empty_context(),
            )
            .await
            .expect_err("scope guard should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("scope") && msg.contains("send"),
            "expected scope-related error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn send_gmail_skill_errors_on_missing_required_args() {
        let mock = MockServer::start().await;
        let client = make_client(mock.uri(), token_full_scope());
        let skill = SendGmailSkill::new(client);

        // 缺 to
        let err = skill
            .execute(
                serde_json::json!({"subject": "s", "body": "b"}),
                &empty_context(),
            )
            .await
            .expect_err("missing to");
        assert!(err.to_string().contains("to"));

        // empty to
        let err = skill
            .execute(
                serde_json::json!({"to": [], "subject": "s", "body": "b"}),
                &empty_context(),
            )
            .await
            .expect_err("empty to");
        assert!(err.to_string().contains("to"));
    }

    // ── name / description / schema 完整性 ─────────────────────────

    #[test]
    fn skill_metadata_is_set() {
        // 不需要 client(metadata 不打 IO),但 trait method 仍要 self —
        // 給空 mock client。
        let dummy = Arc::new(Mutex::new(GmailClient::with_base(
            token_full_scope(),
            dummy_oauth(),
            std::path::PathBuf::from("/tmp/x"),
            "http://localhost",
            "http://localhost/token",
        )));
        let list = ListGmailSkill::new(dummy.clone());
        assert_eq!(list.name(), "list_gmail");
        assert!(!list.description().is_empty());

        let read = ReadGmailSkill::new(dummy.clone());
        assert_eq!(read.name(), "read_gmail");
        let schema = read.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "thread_id"));

        let send = SendGmailSkill::new(dummy);
        assert_eq!(send.name(), "send_gmail");
        assert!(send.confirm_required(), "send should be confirm-required");
        let schema = send.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        for needle in ["to", "subject", "body"] {
            assert!(
                required.iter().any(|v| v.as_str() == Some(needle)),
                "send_gmail schema must require {needle}"
            );
        }
    }
}
