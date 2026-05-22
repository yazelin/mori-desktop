//! `mori-gmail` — Gmail 整合(「跨界之手」之一)。
//!
//! Mori 透過 Google OAuth2 + Gmail REST API 讀使用者信箱。User-owned data 原則:
//! 沒有中央 OAuth relay,token 直接放在使用者本機 `~/.mori/gmail-token.json`,
//! Mori binary 跟 Google 之間是直連。
//!
//! ## Wave 8 sub-streams
//!
//! - **Gm-1**(本 crate ship 範圍):OAuth2 flow + token storage + read-only API
//!   (`gmail.readonly` scope,`threads.list / threads.get / labels.list`)。
//! - **Gm-2**(下一個 stream):LLM Skill wrapper(`ListGmailSkill` /
//!   `ReadGmailSkill` / `SendGmailSkill`)+ mori-tauri Tauri commands + `gmail.send`
//!   scope 升級 + Deps 頁 OAuth setup 引導。
//!
//! ## Config 慣例
//!
//! User 自己在 Google Cloud Console 建 OAuth client(type = Desktop),拿到
//! `client_id` / `client_secret`,寫進 `~/.mori/gmail-config.json`:
//!
//! ```json
//! {
//!   "client_id": "...",
//!   "client_secret": "...",
//!   "redirect_uri": "http://localhost:8765/oauth/callback"
//! }
//! ```
//!
//! ## 典型用法(Gm-2 會包成 Tauri command)
//!
//! ```ignore
//! use mori_gmail::{OAuthConfig, GmailClient, default_token_path, run_oauth_flow,
//!                  GMAIL_READONLY_SCOPE};
//!
//! // 第一次 consent
//! let config = OAuthConfig::load(&OAuthConfig::default_path().unwrap())?;
//! let token = run_oauth_flow(&config, "csrf-state-token", &[GMAIL_READONLY_SCOPE]).await?;
//! token.save(&default_token_path().unwrap())?;
//!
//! // 之後 API 用
//! let mut client = GmailClient::new(default_token_path().unwrap(), config).await?;
//! let threads = client.list_threads(Some("is:unread"), 10).await?;
//! for t in threads {
//!     let full = client.get_thread(&t.id).await?;
//!     // …
//! }
//! ```
//!
//! ## 未實作 / 暫不做(Gm-2)
//!
//! - `messages.send`(需 `gmail.send` scope)
//! - LLM Skill trait wrapper
//! - mori-tauri Tauri command 註冊
//! - 多帳號 / 帳號切換
//! - HTML body 解析(現階段只取 `text/plain` part)
//! - 附件下載

pub mod client;
pub mod oauth;
pub mod token;

pub use client::{
    build_rfc822_message, GmailClient, GmailError, Label, Message, SendOutcome, Thread,
    ThreadSummary, GMAIL_API_BASE,
};
pub use oauth::{
    build_auth_url, refresh_token, run_oauth_flow, OAuthConfig, OAuthError,
    GMAIL_DEFAULT_SCOPES, GMAIL_READONLY_SCOPE, GMAIL_SEND_SCOPE, GOOGLE_AUTH_URL,
    GOOGLE_TOKEN_URL,
};
pub use token::{default_token_path, GmailToken, TokenError};
