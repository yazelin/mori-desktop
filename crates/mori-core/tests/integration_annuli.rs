//! Wave 4 step 12 e2e Rust 端 integration test。
//!
//! 需要 annuli HTTP server 在 `http://localhost:5000` 跑、`ANNULI_SOUL_TOKEN`
//! env 設好。沒跑 server 的話這檔 test 全 skip(用 `#[ignore]` annotation,
//! 跑時加 `cargo test --test integration_annuli -- --ignored`)。
//!
//! 跟 mori-desktop bin test 衝突可能:cargo build 整個 workspace 沒事,單獨
//! cargo test --test integration_annuli 也沒事。

use std::env;
use std::time::Duration;

use mori_core::annuli::{AnnuliClient, AnnuliClientConfig};

fn config_from_env() -> Option<AnnuliClientConfig> {
    if env::var("ANNULI_E2E").is_err() {
        return None;
    }
    let endpoint = env::var("ANNULI_E2E_ENDPOINT").unwrap_or_else(|_| "http://localhost:5000".into());
    let spirit = env::var("ANNULI_E2E_SPIRIT").unwrap_or_else(|_| "mori".into());
    let user_id = env::var("ANNULI_E2E_USER").unwrap_or_else(|_| "yazelin".into());
    let soul_token = env::var("ANNULI_SOUL_TOKEN").ok();
    Some(AnnuliClientConfig {
        endpoint,
        spirit_name: spirit,
        user_id,
        soul_token,
        basic_auth: None,
        timeout: Duration::from_secs(15),
    })
}

#[tokio::test]
#[ignore]
async fn e2e_health_reachable() {
    let cfg = config_from_env().expect("set ANNULI_E2E=1");
    let client = AnnuliClient::new(cfg).expect("build client");
    let h = client.health().await.expect("health");
    assert!(h.ok);
}

#[tokio::test]
#[ignore]
async fn e2e_get_soul() {
    let cfg = config_from_env().expect("set ANNULI_E2E=1");
    let client = AnnuliClient::new(cfg).expect("build client");
    let soul = client.get_soul().await.expect("get soul");
    assert!(!soul.is_empty(), "SOUL.md should not be empty");
}

#[tokio::test]
#[ignore]
async fn e2e_append_and_list_event() {
    let cfg = config_from_env().expect("set ANNULI_E2E=1");
    let client = AnnuliClient::new(cfg).expect("build client");
    let unique_marker = format!("e2e-rust-{}", chrono::Utc::now().timestamp_millis());
    let (date, _line_no) = client
        .append_event(
            "chat",
            "rust-integration-test",
            serde_json::json!({ "role": "user", "text": &unique_marker }),
        )
        .await
        .expect("append event");

    // 列今天 events,確認剛寫的那條 unique marker 在裡面
    let events = client.list_events_by_date(&date).await.expect("list events");
    let found = events
        .iter()
        .any(|e| serde_json::to_string(&e.data).unwrap_or_default().contains(&unique_marker));
    assert!(found, "newly-appended event with marker {unique_marker} not found in list");
}

#[tokio::test]
#[ignore]
async fn e2e_put_soul_without_token_returns_err() {
    // 故意把 soul_token 設成 None,試 PUT — client should 在 with_soul_token() 早 reject
    let mut cfg = config_from_env().expect("set ANNULI_E2E=1");
    cfg.soul_token = None;
    let client = AnnuliClient::new(cfg).expect("build client");
    let result = client.put_soul("EVIL").await;
    assert!(result.is_err(), "PUT /soul without token should fail");
}

#[tokio::test]
#[ignore]
async fn e2e_list_memory_sections() {
    let cfg = config_from_env().expect("set ANNULI_E2E=1");
    let client = AnnuliClient::new(cfg).expect("build client");
    // 不需 token,GET /memory 是 read-only
    let sections = client.list_memory_sections(false).await.expect("list memory");
    // 數可能是 0 也可能 >0,只要不 crash
    println!("memory sections: {}", sections.len());
}

#[tokio::test]
#[ignore]
async fn e2e_append_memory_section_with_token() {
    let cfg = config_from_env().expect("set ANNULI_E2E=1");
    if cfg.soul_token.is_none() {
        eprintln!("skipping: ANNULI_SOUL_TOKEN not set");
        return;
    }
    let client = AnnuliClient::new(cfg).expect("build client");
    let header = format!("e2e-rust-section-{}", chrono::Utc::now().timestamp_millis());
    let body = "Wave 4 e2e Rust client 寫 memory section 測試。";
    client.append_memory_section(&header, body).await.expect("append memory section");

    let sections = client.list_memory_sections(true).await.expect("list with body");
    let found = sections.iter().find(|s| s.header == header);
    let found = found.expect("appended section should appear in list");
    assert!(found.body.as_deref().unwrap_or("").contains(body));
}
