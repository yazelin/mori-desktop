//! Wave 4 step 10:Tauri commands wrap annuli HTTP API,給 AnnuliTab.tsx 用。
//!
//! 所有 command 走 `state.annuli`(若 None → 回 user-friendly 錯誤字串)。
//! Frontend Rust ↔ TS 型別:用簡單 serde struct,不暴露 chrono / Path / 等
//! 不直接序列化得很 React-friendly 的型別。

use std::sync::Arc;

use mori_core::annuli::{AnnuliClient, EventRecord};
use serde::Serialize;

use crate::AppState;

#[derive(Serialize)]
pub struct AnnuliStatus {
    /// `true` 若 config.annuli.enabled + 完整 → 有 client
    pub configured: bool,
    /// `true` 若 client 跑得通 `/health`
    pub reachable: bool,
    pub endpoint: Option<String>,
    pub spirit: Option<String>,
    pub user_id: Option<String>,
    pub soul_token_configured: bool,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct AnnuliMemorySection {
    pub header: String,
    pub index: u32,
    pub body: Option<String>,
}

#[derive(Serialize)]
pub struct AnnuliEvent {
    pub ts: String,
    pub kind: String,
    pub user_id: String,
    pub source: String,
    /// JSON-stringified data(避免 TS 端要 typecheck Value 樹)
    pub data_json: String,
}

fn client(state: &AppState) -> Result<Arc<AnnuliClient>, String> {
    state
        .annuli
        .clone()
        .ok_or_else(|| "annuli 沒接(config.json `annuli.enabled` 沒設或缺欄位)".to_string())
}

fn ev_to_ts(e: &EventRecord) -> AnnuliEvent {
    AnnuliEvent {
        ts: e.ts.to_rfc3339(),
        kind: e.kind.clone(),
        user_id: e.user_id.clone(),
        source: e.source.clone(),
        data_json: serde_json::to_string(&e.data).unwrap_or_else(|_| "{}".to_string()),
    }
}

#[tauri::command]
pub async fn annuli_status(state: tauri::State<'_, Arc<AppState>>) -> Result<AnnuliStatus, String> {
    let Some(client) = state.annuli.clone() else {
        return Ok(AnnuliStatus {
            configured: false,
            reachable: false,
            endpoint: None,
            spirit: None,
            user_id: None,
            soul_token_configured: false,
            error: Some("annuli not configured".to_string()),
        });
    };
    let health = client.health().await;
    match health {
        Ok(h) => Ok(AnnuliStatus {
            configured: true,
            reachable: h.ok,
            endpoint: Some(client.endpoint_for_display()),
            spirit: Some(client.spirit_name().to_string()),
            user_id: Some(client.user_id().to_string()),
            soul_token_configured: h.soul_token_configured,
            error: None,
        }),
        Err(e) => Ok(AnnuliStatus {
            configured: true,
            reachable: false,
            endpoint: Some(client.endpoint_for_display()),
            spirit: Some(client.spirit_name().to_string()),
            user_id: Some(client.user_id().to_string()),
            soul_token_configured: false,
            error: Some(format!("{}", e)),
        }),
    }
}

#[tauri::command]
pub async fn annuli_get_soul(state: tauri::State<'_, Arc<AppState>>) -> Result<String, String> {
    let client = client(&state)?;
    client.get_soul().await.map_err(|e| format!("{}", e))
}

#[tauri::command]
pub async fn annuli_list_memory(
    include_body: bool,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<AnnuliMemorySection>, String> {
    let client = client(&state)?;
    let sections = client.list_memory_sections(include_body).await.map_err(|e| format!("{}", e))?;
    Ok(sections
        .into_iter()
        .map(|s| AnnuliMemorySection {
            header: s.header,
            index: s.index,
            body: s.body,
        })
        .collect())
}

#[tauri::command]
pub async fn annuli_list_events_today(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<AnnuliEvent>, String> {
    let client = client(&state)?;
    let today = chrono::Local::now().date_naive().to_string();
    let events = client.list_events_by_date(&today).await.map_err(|e| format!("{}", e))?;
    Ok(events.iter().map(ev_to_ts).collect())
}

#[tauri::command]
pub async fn annuli_trigger_sleep(state: tauri::State<'_, Arc<AppState>>) -> Result<String, String> {
    let client = client(&state)?;
    client.trigger_sleep().await.map_err(|e| format!("{}", e))
}
