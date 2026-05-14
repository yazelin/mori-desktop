//! `annuli` HTTP client — 跟 [annuli](https://github.com/yazelin/annuli)(Mori 的
//! 反思引擎)透過 HTTP 對話。
//!
//! ## 設計
//!
//! - 純 async client(reqwest),沒 in-memory state
//! - 認 `~/.mori/config.json` 的 `annuli.endpoint` / `annuli.spirit_name` /
//!   `annuli.basic_auth` / `annuli.soul_token` / `annuli.user_id`
//! - **永遠不把 `soul_token` 印 log** — 寫 request log 時 redact
//!
//! ## 對應 annuli API(來自 Wave 3 routes)
//!
//! | Method | Endpoint                                                    | This module fn |
//! |--------|-------------------------------------------------------------|----------------|
//! | GET    | `/spirits/<x>/soul`                                         | [`AnnuliClient::get_soul`] |
//! | PUT    | `/spirits/<x>/soul` (X-Soul-Token required)                 | [`AnnuliClient::put_soul`] |
//! | POST   | `/spirits/<x>/events`                                       | [`AnnuliClient::append_event`] |
//! | GET    | `/spirits/<x>/events?date=` / `?q=` / `?kind=`              | [`AnnuliClient::list_events_*`] |
//! | POST   | `/spirits/<x>/rings/new`                                    | [`AnnuliClient::trigger_sleep`] |
//! | POST   | `/spirits/<x>/curator/dry-run`                              | [`AnnuliClient::curator_dry_run`] |
//! | POST   | `/spirits/<x>/curator/apply`                                | [`AnnuliClient::curator_apply`] |
//! | POST   | `/spirits/<x>/bootstrap`                                    | [`AnnuliClient::bootstrap`] |
//! | GET    | `/health`                                                   | [`AnnuliClient::health`] |
//!
//! ## 不在本模組
//!
//! - `MemoryStore` trait 對應(那是 `crate::memory::annuli`)
//! - hotkey / UI 整合(在 mori-tauri)

pub mod client;

pub use client::{AnnuliClient, AnnuliClientConfig, AnnuliError, EventRecord, HealthResponse, MemorySection};
