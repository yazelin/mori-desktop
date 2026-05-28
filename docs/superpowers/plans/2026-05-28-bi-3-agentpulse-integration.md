# BI-3 AgentPulse Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把**現成的** AgentPulse(AI coding CLI session 監控 app)接成 Mori 的第一個 event-stream body part。AgentPulse 長一張「對外的嘴」(body-interface 的 `/manifest` `/health` `/sessions` `/events`SSE),mori-desktop 訂閱它的脈搏,顯示 session 清單 + 最小 cue 提醒。達成 backlog 判準:**跑一個 Codex session,Desktop 收到 waiting_input / done cue(不重複)**。

**Architecture:** AgentPulse 的偵測核心**已完成**(hooks → `SessionManager` → `SessionTransition::{Completed, StartedWaiting}`,見其 `session.rs`/`lib.rs`)。本計畫**不動偵測**,只:(A)在 AgentPulse 既有的 raw-HTTP tokio server(`hook_server.rs`,port 19280–19289,寫在 `~/.agentpulse/port`)加 GET 讀取路由 + SSE 廣播 + 啟動寫 manifest 到 `~/.mori/body-parts/mori.agent-pulse/`;(B)mori-desktop 加一個唯讀「脈搏」tab,用 BI-1 registry 找到 AgentPulse manifest、從 webview 直接 `fetch(/sessions)` + `EventSource(/events)`(mori-desktop CSP=null,localhost 直連可行;AgentPulse 回 `Access-Control-Allow-Origin: *`)。

**Tech Stack:** Rust(AgentPulse:tokio broadcast/watch + 既有 raw-HTTP server,**零新依賴** —— tokio 已有 `sync` feature)、React/TS(mori-desktop:新 tab,EventSource + fetch + i18n)。

**兩個 repo / 兩個 PR:**
- **Part A** → `~/mori-universe/AgentPulse`(repo `yazelin/AgentPulse`),自己的 branch + PR。
- **Part B** → `~/mori-universe/mori-desktop`,自己的 branch + PR。
- A 先合(嘴要先存在),B 才能 e2e;但 B 可在本機跑著 A 的 debug build 時開發。

**Spec sources:**
- `docs/body-interface-backlog.md`(BI-3 row L95 + 判準 L106 + A1/D1)
- `docs/mori-body-interface.md`(§Events L440-465 event envelope、§Control L418-429、§Agent Plus L705-773「Pulse / Cue collector」、§766「通知不要重複」)
- AgentPulse `src-tauri/src/{session.rs,hook_server.rs,lib.rs}` + `CLAUDE.md`(現況真相)

---

## 設計決策(已選預設,review 時可推翻)

| # | 決策 | 選的(預設)| 備選 / 備註 |
|---|---|---|---|
| Q1 | **偵測機制** | **沿用 AgentPulse 既有 hooks**(已多家通吃 Claude/Gemini/Codex/Copilot),完全不動 | 不自己 tail `~/.codex/sessions/`(重造輪子且只有一家)|
| Q2 | **transport** | AgentPulse 既有 raw-HTTP server 加 GET 路由 + SSE。port 沿用 19280–19289 / `~/.agentpulse/port` | 不引入 axum(server 已在跑,加路由最小)|
| Q3 | **mori-desktop 怎麼接** | **webview 直接 EventSource + fetch**(CSP=null,localhost 直連)。AgentPulse 回 CORS `*` | 備選:Rust reqwest streaming 再 re-emit Tauri event(CSP 有限制才需要,本專案不需要)|
| Q4 | **誰發使用者通知** | AgentPulse 維持自己的膠囊 + 音效;**mori-desktop 端只在 tab 內被動顯示**,不發系統通知(避免重複,§766)| 誰主誰從留 BI-4 Cue Center |
| Q5 | **port discovery** | AgentPulse 啟動時把**實際 port 寫進 manifest**(`interfaces[].url`)。mori-desktop 從 BI-1 registry 讀 manifest 取 url | AgentPulse 換 port → 重寫 manifest;mori 重新整理時重讀 |
| Q6 | **manifest id / kind** | `id: mori.agent-pulse`,`kind: local_service`,`capabilities: ["agent.session.observe"]` | 對齊 doc L160「`mori.agent-plus`」精神,改用正名 agent-pulse |
| Q7 | **cue 事件型別** | `StartedWaiting`→`cue.waiting_input`(severity attention);`Completed`→`cue.done`(severity info)| 對齊 doc §452 envelope;dedup 用 `event_id` |
| Q8 | **mori 端要不要動 Rust** | **不用** —— 重用 BI-1 `body_registry_list` command 拿 manifest;tab 純前端 fetch/EventSource | 不加新 Tauri command |

---

## File Structure

**Part A — `~/mori-universe/AgentPulse/src-tauri/src/`**

| 檔案 | 責任 | 動作 |
|---|---|---|
| `mori_bridge.rs` | `MoriEvent` envelope + `from_transition()` + `manifest_json(port)` + `write_manifest(port)` + `sse_frame()` + 測試 | Create |
| `hook_server.rs` | `handle_client` 加 method+path 路由(GET /health·/manifest·/sessions·/events SSE + CORS);`HookServer::start` / `accept_loop` / `handle_client` 多收 `broadcast::Sender<MoriEvent>` + `watch::Receiver<AppState>` | Modify |
| `lib.rs` | 建 broadcast + watch channel;event loop 發 snapshot + cue;啟動寫 manifest;`mod mori_bridge;` | Modify |

**Part B — `~/mori-universe/mori-desktop/`**

| 檔案 | 責任 | 動作 |
|---|---|---|
| `src/tabs/PulseTab.tsx` | 讀 `body_registry_list` 找 agent-pulse → fetch `/sessions` + `EventSource /events` → session 清單 + cue 清單(dedup)| Create |
| `src/shellTabs.ts` | `SHELL_TAB_IDS` 加 `"pulse"` | Modify |
| `src/MainShell.tsx` | import + TABS entry + render switch | Modify |
| `src/icons.tsx` | `IconPulse`(脈搏波形)| Modify |
| `src/i18n/locales/{zh-TW,en}.json` | `sidebar.pulse(_sub)` + `pulse_tab.*` | Modify |
| `docs/body-interface-backlog.md` | BI-3 進度 | Modify |

---

# Part A — AgentPulse(對外的嘴)

> Work in `~/mori-universe/AgentPulse`. Branch off its `main`: `git -C ~/mori-universe/AgentPulse checkout main && git -C ~/mori-universe/AgentPulse pull --ff-only && git -C ~/mori-universe/AgentPulse checkout -b feat/mori-body-bridge`.
> AgentPulse 目前**沒有測試 harness**;新增的純函式放 `#[cfg(test)]` 單元測試(`cargo test` 可跑),路由走 curl 手測。
> 編譯/跑:照其 CLAUDE.md —— debug 用 `./dev.sh`(`cargo build`),互動跑用 `cargo tauri dev`(別用 `cargo build` 直接跑,frontend 不會 embed)。本計畫只動 Rust,`cargo build` / `cargo test` 即可驗證編譯與單元測試。

## Task A1: `mori_bridge.rs` — 事件 envelope + manifest + SSE frame

**Files:** Create `src-tauri/src/mori_bridge.rs`

- [ ] **Step 1: 寫 failing test(放檔案底部)**

```rust
//! Mori body-interface 橋接:把 AgentPulse 既有的 session 狀態 / 轉換,用 Mori 的
//! event envelope + manifest 對外播。偵測邏輯不在這(在 session.rs);這裡只做格式轉換
//! 與 ~/.mori/body-parts 的 manifest 寫入。見 mori-desktop docs/mori-body-interface.md。

use crate::session::{AppState, SessionTransition};
use serde::Serialize;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_transition_maps_to_cue_waiting_input() {
        let ev = MoriEvent::from_transition(
            SessionTransition::StartedWaiting, "codex", "sess_1", "2026-05-28T10:00:00+08:00",
        )
        .expect("waiting → Some");
        assert_eq!(ev.r#type, "cue.waiting_input");
        assert_eq!(ev.severity, "attention");
        assert_eq!(ev.session_id, "sess_1");
        assert_eq!(ev.source, "mori.agent-pulse");
        assert!(ev.summary.contains("codex"));
    }

    #[test]
    fn completed_transition_maps_to_cue_done() {
        let ev = MoriEvent::from_transition(
            SessionTransition::Completed, "claude", "s2", "2026-05-28T10:00:00+08:00",
        )
        .expect("completed → Some");
        assert_eq!(ev.r#type, "cue.done");
        assert_eq!(ev.severity, "info");
    }

    #[test]
    fn none_transition_produces_no_event() {
        assert!(MoriEvent::from_transition(
            SessionTransition::None, "x", "s", "t").is_none());
    }

    #[test]
    fn manifest_is_valid_json_with_id_kind_and_live_port() {
        let m = manifest_json(19283);
        let v: serde_json::Value = serde_json::from_str(&m).expect("manifest valid json");
        assert_eq!(v["id"], "mori.agent-pulse");
        assert_eq!(v["kind"], "local_service");
        // SSE interface 必須帶實際 port,讓 mori-desktop 連得到。
        let ifaces = v["interfaces"].as_array().unwrap();
        let sse = ifaces.iter().find(|i| i["transport"] == "sse").unwrap();
        assert_eq!(sse["url"], "http://127.0.0.1:19283/events");
    }

    #[test]
    fn sse_frame_is_data_line_with_double_newline() {
        let f = sse_frame(r#"{"a":1}"#);
        assert_eq!(f, "data: {\"a\":1}\n\n");
    }
}
```

- [ ] **Step 2: 跑測試確認 fail**

Run: `cargo test -p agent-pulse --lib mori_bridge` (crate 名見 Cargo.toml `[lib] name`;若不同就用實際名;或 `cargo test mori_bridge`)
Expected: 編譯失敗(型別/函式未定義)。

- [ ] **Step 3: 寫實作(test mod 上方)**

```rust
/// Mori event envelope — 對齊 docs/mori-body-interface.md §Events。
#[derive(Debug, Clone, Serialize)]
pub struct MoriEvent {
    pub schema_version: u32,
    pub event_id: String,
    pub source: String,
    pub r#type: String,
    pub time: String,
    pub session_id: String,
    pub severity: String,
    pub summary: String,
    pub payload: serde_json::Value,
}

impl MoriEvent {
    /// 把 AgentPulse 的 transition 轉成 Mori cue 事件。None → 不發。
    pub fn from_transition(
        t: SessionTransition,
        provider: &str,
        session_id: &str,
        now: &str,
    ) -> Option<MoriEvent> {
        let (kind, severity, summary) = match t {
            SessionTransition::StartedWaiting => (
                "cue.waiting_input",
                "attention",
                format!("{provider} is waiting for input."),
            ),
            SessionTransition::Completed => (
                "cue.done",
                "info",
                format!("{provider} session finished."),
            ),
            SessionTransition::None => return None,
        };
        Some(MoriEvent {
            schema_version: 1,
            event_id: format!("evt-{session_id}-{now}"),
            source: "mori.agent-pulse".to_string(),
            r#type: kind.to_string(),
            time: now.to_string(),
            session_id: session_id.to_string(),
            severity: severity.to_string(),
            summary,
            payload: serde_json::json!({ "provider": provider }),
        })
    }
}

/// body part manifest(BI-1 BodyManifest 形狀)。帶啟動時的實際 port。
pub fn manifest_json(port: u16) -> String {
    serde_json::json!({
        "schema_version": 1,
        "id": "mori.agent-pulse",
        "name": "AgentPulse",
        "kind": "local_service",
        "description": "AI coding CLI session pulse — observes Claude / Gemini / Codex / Copilot sessions.",
        "capabilities": ["agent.session.observe"],
        "interfaces": [
            { "name": "control", "transport": "http", "base_url": format!("http://127.0.0.1:{port}") },
            { "name": "events",  "transport": "sse",  "url": format!("http://127.0.0.1:{port}/events") }
        ],
        "permissions": [],
        "data_policy": { "owns_raw_data": true, "default_ingestion": "off" }
    })
    .to_string()
}

/// 啟動時把 manifest 寫到 ~/.mori/body-parts/mori.agent-pulse/manifest.json(覆寫 — 因為 port 會變)。
pub fn write_manifest(port: u16) {
    if let Some(home) = dirs::home_dir() {
        let dir = home.join(".mori").join("body-parts").join("mori.agent-pulse");
        if std::fs::create_dir_all(&dir).is_ok() {
            let _ = std::fs::write(dir.join("manifest.json"), manifest_json(port));
        }
    }
}

/// 一個 SSE data frame。
pub fn sse_frame(json: &str) -> String {
    format!("data: {json}\n\n")
}

/// 把當前 AppState 序列化成 /sessions 的 body(已是 serde Serialize)。
pub fn sessions_json(state: &AppState) -> String {
    serde_json::to_string(state).unwrap_or_else(|_| "{}".to_string())
}
```

> 注意:`MoriEvent.payload` 用 `serde_json::json!`;確認 `serde_json` 在 deps(有)。`SessionTransition` 目前 `#[derive(...)]` 沒有 `Serialize` 也沒關係(這裡只 match 它)。`AppState` 已 `Serialize`。

- [ ] **Step 4: 跑測試確認 pass**

Run: `cargo test mori_bridge`
Expected: 5 test PASS。Run `cargo build`(無回歸)。

- [ ] **Step 5: Commit**

```bash
git -C ~/mori-universe/AgentPulse add src-tauri/src/mori_bridge.rs
git -C ~/mori-universe/AgentPulse commit -m "feat(mori): MoriEvent envelope + manifest + SSE frame helpers

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

## Task A2: `hook_server.rs` — GET 路由 + SSE + CORS

**Files:** Modify `src-tauri/src/hook_server.rs`

> 現況:`handle_client` 只處理 POST `/hook/{provider}`,把事件丟進 `tx`。要加:GET 路由 + 兩個共享 handle(`broadcast::Sender<MoriEvent>` 給 /events、`watch::Receiver<AppState>` 給 /sessions)。

- [ ] **Step 1: imports + start 簽名加參數**

頂部加:
```rust
use crate::mori_bridge::{manifest_json, sessions_json, sse_frame, MoriEvent};
use crate::session::AppState;
use tokio::sync::{broadcast, watch};
```
把 `start` 簽名改成接收 channel(呼叫端 lib.rs 會傳):
```rust
    pub async fn start(
        &mut self,
        cue_tx: broadcast::Sender<MoriEvent>,
        snap_rx: watch::Receiver<AppState>,
    ) -> Result<mpsc::UnboundedReceiver<HookEvent>, ServerError> {
```
在 `tokio::spawn(accept_loop(listener, tx));` 改成把 channel + port 帶進去:
```rust
                    tokio::spawn(accept_loop(listener, tx, cue_tx.clone(), snap_rx.clone(), candidate_port));
```

- [ ] **Step 2: accept_loop / handle_client 傳遞 channel**

```rust
async fn accept_loop(
    listener: TcpListener,
    tx: Arc<mpsc::UnboundedSender<HookEvent>>,
    cue_tx: broadcast::Sender<MoriEvent>,
    snap_rx: watch::Receiver<AppState>,
    port: u16,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let tx = tx.clone();
                let cue_tx = cue_tx.clone();
                let snap_rx = snap_rx.clone();
                tokio::spawn(handle_client(stream, tx, cue_tx, snap_rx, port));
            }
            Err(e) => error!("Accept error: {e}"),
        }
    }
}
```

- [ ] **Step 3: handle_client 加 method+path 路由**

把現有 `handle_client` 改成先抓 method + path,GET 走新分支、POST 維持原行為:
```rust
async fn handle_client(
    mut stream: tokio::net::TcpStream,
    tx: Arc<mpsc::UnboundedSender<HookEvent>>,
    cue_tx: broadcast::Sender<MoriEvent>,
    snap_rx: watch::Receiver<AppState>,
    port: u16,
) {
    let mut buf = vec![0u8; 65536];
    let n = match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        stream.read(&mut buf),
    )
    .await
    {
        Ok(Ok(n)) if n > 0 => n,
        _ => return,
    };
    let data = &buf[..n];
    let (method, path) = parse_request_line(data);

    // ---- Mori body-interface 唯讀讀取口(GET)----
    if method == "GET" {
        match path.as_str() {
            "/health" => {
                let _ = stream.write_all(http_ok("text/plain", "ok").as_bytes()).await;
            }
            "/manifest" => {
                let _ = stream
                    .write_all(http_ok("application/json", &manifest_json(port)).as_bytes())
                    .await;
            }
            "/sessions" => {
                let body = sessions_json(&snap_rx.borrow());
                let _ = stream
                    .write_all(http_ok("application/json", &body).as_bytes())
                    .await;
            }
            "/events" => {
                serve_sse(stream, cue_tx).await;
            }
            _ => {
                let _ = stream
                    .write_all(b"HTTP/1.1 404 Not Found\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await;
            }
        }
        return;
    }

    // ---- 既有 hook 入口(POST /hook/{provider})----
    let provider = parse_provider(data);
    let response = if let Some(body_start) = find_body_start(data) {
        let body = &data[body_start..];
        match serde_json::from_slice::<crate::hook_event::RawHookEvent>(body) {
            Ok(raw) => {
                let mut event = raw.normalize(&provider);
                normalize_event_name(&mut event);
                if event.session_id.is_empty() {
                    event.session_id = format!("{}-default", event.provider);
                }
                let _ = tx.send(event);
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}"
            }
            Err(_) => "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        }
    } else {
        "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    };
    let _ = stream.write_all(response.as_bytes()).await;
}

/// "GET /sessions HTTP/1.1" → ("GET", "/sessions")(去掉 query)。
fn parse_request_line(data: &[u8]) -> (String, String) {
    let line = data
        .split(|&b| b == b'\r' || b == b'\n')
        .next()
        .map(|l| String::from_utf8_lossy(l).to_string())
        .unwrap_or_default();
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let raw_path = parts.next().unwrap_or("");
    let path = raw_path.split('?').next().unwrap_or("").to_string();
    (method, path)
}

fn http_ok(content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        ct = content_type,
        len = body.len(),
    )
}

/// SSE:keep-alive,把 broadcast 的 MoriEvent 一筆筆寫成 `data: {json}\n\n`。
async fn serve_sse(mut stream: tokio::net::TcpStream, cue_tx: broadcast::Sender<MoriEvent>) {
    let mut rx = cue_tx.subscribe();
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nAccess-Control-Allow-Origin: *\r\nConnection: keep-alive\r\n\r\n";
    if stream.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    // 先送一筆 comment 讓連線確立。
    let _ = stream.write_all(b": connected\n\n").await;
    loop {
        match rx.recv().await {
            Ok(ev) => {
                let json = serde_json::to_string(&ev).unwrap_or_default();
                if stream.write_all(sse_frame(&json).as_bytes()).await.is_err() {
                    break; // client 斷線
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}
```

- [ ] **Step 2.5: 確認 imports/編譯**

Run: `cargo build`
Expected: 因為 `start` 簽名變了,lib.rs 還沒改 → 這步**會**編譯失敗在 lib.rs 呼叫處。先確認 hook_server.rs 本身無語法錯(錯誤只剩 lib.rs 的 arity mismatch),Task A3 補上。

- [ ] **Step 3: Commit**

```bash
git -C ~/mori-universe/AgentPulse add src-tauri/src/hook_server.rs
git -C ~/mori-universe/AgentPulse commit -m "feat(mori): hook_server GET routes (/health /manifest /sessions /events SSE) + CORS

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

## Task A3: `lib.rs` — channel 接線 + 發 cue + 寫 manifest

**Files:** Modify `src-tauri/src/lib.rs`

- [ ] **Step 1: module + use**

頂部 `mod` 區加 `mod mori_bridge;`。`use` 區加:
```rust
use mori_bridge::MoriEvent;
use tokio::sync::{broadcast, watch};
```

- [ ] **Step 2: 建 channel + 改 start 呼叫 + event loop 發 snapshot/cue**

在 setup 內、`let port = rt.block_on(async { ... })` 那段改寫。原本:
```rust
            let port = rt.block_on(async {
                let mut server = HookServer::new();
                match server.start().await {
                    Ok(mut rx) => {
                        let port = server.port();
                        let h = handle.clone();
                        tokio::spawn(async move {
                            while let Some(event) = rx.recv().await {
                                let mgr = h.state::<AppSessionManager>();
                                let transition = {
                                    let mut m = mgr.0.lock().unwrap();
                                    m.handle_event(&event)
                                };
                                let _ = h.emit("session-update", ());
                                match transition {
                                    session::SessionTransition::Completed => {
                                        let _ = h.emit("task-completed", event.provider.clone());
                                    }
                                    session::SessionTransition::StartedWaiting => {
                                        let _ = h.emit("task-waiting", event.provider.clone());
                                    }
                                    session::SessionTransition::None => {}
                                }
                            }
                        });
                        port
                    }
                    Err(e) => { log::error!("Failed to start server: {e}"); 0 }
                }
            });
```
改成(加 channel、把 snapshot/cue 播出去、保留既有 emit):
```rust
            let (cue_tx, _cue_rx0) = broadcast::channel::<MoriEvent>(256);
            let (snap_tx, snap_rx) = watch::channel(session::SessionManager::new().get_state());
            let port = rt.block_on(async {
                let mut server = HookServer::new();
                match server.start(cue_tx.clone(), snap_rx.clone()).await {
                    Ok(mut rx) => {
                        let port = server.port();
                        let h = handle.clone();
                        let cue_tx = cue_tx.clone();
                        tokio::spawn(async move {
                            while let Some(event) = rx.recv().await {
                                let mgr = h.state::<AppSessionManager>();
                                let (transition, snapshot) = {
                                    let mut m = mgr.0.lock().unwrap();
                                    let t = m.handle_event(&event);
                                    (t, m.get_state())
                                };
                                let _ = snap_tx.send(snapshot);
                                let _ = h.emit("session-update", ());
                                // Mori cue 廣播(給 SSE 訂閱者,例如 mori-desktop)
                                let now = chrono::Utc::now().to_rfc3339();
                                if let Some(ev) = MoriEvent::from_transition(
                                    transition, &event.provider, &event.session_id, &now,
                                ) {
                                    let _ = cue_tx.send(ev);
                                }
                                // 既有:AgentPulse 自己的膠囊音效(不動)
                                match transition {
                                    session::SessionTransition::Completed => {
                                        let _ = h.emit("task-completed", event.provider.clone());
                                    }
                                    session::SessionTransition::StartedWaiting => {
                                        let _ = h.emit("task-waiting", event.provider.clone());
                                    }
                                    session::SessionTransition::None => {}
                                }
                            }
                        });
                        port
                    }
                    Err(e) => { log::error!("Failed to start server: {e}"); 0 }
                }
            });
```

> `snap_tx` 被 move 進 spawn;`snap_rx` 已在 `server.start(..., snap_rx.clone())` 用過。`cue_tx` clone 給 server + event loop 各一份。`watch::channel` 初值用一個空的 `SessionManager::new().get_state()`。

- [ ] **Step 3.5: 啟動寫 manifest**

在 `app.manage(ServerPort(port));` 之後加(port>0 才寫):
```rust
            if port != 0 {
                mori_bridge::write_manifest(port);
            }
```

- [ ] **Step 4: 編譯 + 單元測試**

Run: `cargo build`
Expected: 乾淨(arity 對上了)。
Run: `cargo test mori_bridge`
Expected: 5 PASS。

- [ ] **Step 5: Commit**

```bash
git -C ~/mori-universe/AgentPulse add src-tauri/src/lib.rs
git -C ~/mori-universe/AgentPulse commit -m "feat(mori): broadcast cues + sessions snapshot + write body manifest on startup

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

## Task A4: 手測(curl)+ 開 PR

- [ ] **Step 1: 跑起來**

```bash
cd ~/mori-universe/AgentPulse && cargo tauri dev   # 或 ./watch.sh
```

- [ ] **Step 2: curl 驗證讀取口**

```bash
PORT=$(cat ~/.agentpulse/port)
curl -s localhost:$PORT/health                       # → ok
curl -s localhost:$PORT/manifest | python3 -m json.tool | head    # id=mori.agent-pulse + 帶 port 的 sse url
curl -s localhost:$PORT/sessions | python3 -m json.tool | head    # AppState 快照
ls ~/.mori/body-parts/mori.agent-pulse/manifest.json # 啟動已寫
```

- [ ] **Step 3: SSE + 真 Codex session**

開一個 terminal 連 SSE:`curl -N localhost:$PORT/events`(應先看到 `: connected`)。另開一個 Codex session 跑一輪 → SSE 應出現 `data: {"type":"cue.waiting_input",...}`(turn 完等輸入)。
(若你還沒在 AgentPulse 開 Codex 的 hook,先在它 UI 把 Codex 打開。)

- [ ] **Step 4: PR + auto-merge**

```bash
git -C ~/mori-universe/AgentPulse push -u origin feat/mori-body-bridge
gh -R yazelin/AgentPulse pr create --title "feat(mori): body-interface bridge (manifest + /sessions + /events SSE)" --body "Adds a read-only Mori body-interface mouth to the existing hook server: GET /health /manifest /sessions and an SSE /events stream that re-broadcasts session transitions as Mori cue events (cue.waiting_input / cue.done). Writes a body manifest to ~/.mori/body-parts/mori.agent-pulse/ on startup. Detection unchanged. Lets mori-desktop (BI-3) subscribe to AgentPulse's pulse.

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
gh -R yazelin/AgentPulse pr merge --auto --merge
```

---

# Part B — mori-desktop(訂閱脈搏)

> Work in `~/mori-universe/mori-desktop`. Branch off latest main: `git checkout main && git pull --ff-only && git checkout -b feat/bi-3-pulse-tab`.
> 純前端(重用 BI-1 `body_registry_list`)。沿用 BI-2 PermissionsTab 的 tab pattern + theme token(`var(--c-*)` / `.mori-pill-badge` —— 別寫死 rgba,見 CLAUDE.md「UI 配色」)。

## Task B1: `PulseTab.tsx`

**Files:** Create `src/tabs/PulseTab.tsx`

- [ ] **Step 1: 寫 component**

```tsx
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

interface BodyInterface { name: string; transport: string; base_url?: string; url?: string; }
interface BodyManifest { id: string; name: string; interfaces?: BodyInterface[]; }
interface DiscoveredBodyPart { source: string; status: string; manifest: BodyManifest | null; }

interface SessionInfo {
  id: string; provider: string; state: string; project_name: string;
  is_active: boolean; formatted_time: string;
}
interface SessionsSnapshot { sessions: SessionInfo[]; active_count: number; }
interface Cue {
  event_id: string; type: string; session_id: string; severity: string;
  summary: string; time: string;
}

export default function PulseTab() {
  const { t } = useTranslation();
  const [base, setBase] = useState<string | null>(null);
  const [sseUrl, setSseUrl] = useState<string | null>(null);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [cues, setCues] = useState<Cue[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const seen = useRef<Set<string>>(new Set());

  // 1) 從 BI-1 registry 找 AgentPulse manifest,取 http base + sse url。
  const discover = async () => {
    try {
      const parts = await invoke<DiscoveredBodyPart[]>("body_registry_list");
      const ap = parts.find((p) => p.manifest?.id === "mori.agent-pulse");
      if (!ap?.manifest?.interfaces) { setBase(null); setSseUrl(null); return; }
      const http = ap.manifest.interfaces.find((i) => i.transport === "http")?.base_url ?? null;
      const sse = ap.manifest.interfaces.find((i) => i.transport === "sse")?.url ?? null;
      setBase(http); setSseUrl(sse); setErr(null);
    } catch (e: any) { setErr(String(e)); }
  };
  useEffect(() => { discover(); }, []);

  // 2) 抓 session 清單(輪詢備援,SSE 只送 cue 不送全清單)。
  const refreshSessions = async () => {
    if (!base) return;
    try {
      const r = await fetch(`${base}/sessions`);
      const snap: SessionsSnapshot = await r.json();
      setSessions(snap.sessions ?? []);
    } catch { /* AgentPulse 沒跑就忽略 */ }
  };
  useEffect(() => {
    if (!base) return;
    refreshSessions();
    const id = setInterval(refreshSessions, 5000);
    return () => clearInterval(id);
  }, [base]);

  // 3) SSE 訂閱 cue(dedup by event_id)。
  useEffect(() => {
    if (!sseUrl) return;
    const es = new EventSource(sseUrl);
    es.onmessage = (e) => {
      try {
        const cue: Cue = JSON.parse(e.data);
        if (seen.current.has(cue.event_id)) return;
        seen.current.add(cue.event_id);
        setCues((prev) => [cue, ...prev].slice(0, 50));
        refreshSessions();
      } catch { /* ignore non-json keepalive */ }
    };
    es.onerror = () => { /* EventSource 會自動重連 */ };
    return () => es.close();
  }, [sseUrl]);

  const notRunning = base === null;

  return (
    <div style={{ padding: 16 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 12 }}>
        <h2 style={{ margin: 0 }}>{t("pulse_tab.title")}</h2>
        <button className="mori-btn small ghost" onClick={() => { discover(); refreshSessions(); }}>
          {t("pulse_tab.refresh")}
        </button>
      </div>
      <p style={{ opacity: 0.7, fontSize: 12 }}>{t("pulse_tab.hint")}</p>
      {err && <div className="mori-tab-error" style={{ fontSize: 12 }}>❌ {err}</div>}
      {notRunning && <div style={{ opacity: 0.6 }}>{t("pulse_tab.not_running")}</div>}

      {!notRunning && (
        <>
          <h3 style={{ marginBottom: 6 }}>{t("pulse_tab.sessions_title")}</h3>
          {sessions.length === 0 && <div style={{ opacity: 0.6 }}>{t("pulse_tab.sessions_empty")}</div>}
          <div style={{ display: "flex", flexDirection: "column", gap: 8, marginBottom: 16 }}>
            {sessions.map((s) => (
              <div key={s.id} style={{ border: "1px solid var(--c-border)", borderRadius: 8, padding: 10 }}>
                <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  <StateBadge state={s.state} />
                  <strong>{s.project_name}</strong>
                  <span style={{ fontSize: 11, opacity: 0.7 }}>{s.provider}</span>
                  <span style={{ fontSize: 11, opacity: 0.5, marginLeft: "auto" }}>{s.formatted_time}</span>
                </div>
              </div>
            ))}
          </div>

          <h3 style={{ marginBottom: 6 }}>{t("pulse_tab.cues_title")}</h3>
          {cues.length === 0 && <div style={{ opacity: 0.6 }}>{t("pulse_tab.cues_empty")}</div>}
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            {cues.map((c) => (
              <div key={c.event_id} style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13 }}>
                <CueBadge type={c.type} />
                <span>{c.summary}</span>
                <span style={{ fontSize: 10, opacity: 0.4, marginLeft: "auto" }}>{c.time}</span>
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

function StateBadge({ state }: { state: string }) {
  const tone =
    state === "working" ? "tone-success"
    : state === "waiting_for_user" ? "tone-warning"
    : "tone-neutral";
  return <span className={`mori-pill-badge ${tone}`}>{state}</span>;
}

function CueBadge({ type }: { type: string }) {
  const { t } = useTranslation();
  if (type === "cue.waiting_input")
    return <span className="mori-pill-badge tone-warning">{t("pulse_tab.cue_waiting")}</span>;
  if (type === "cue.done")
    return <span className="mori-pill-badge tone-success">{t("pulse_tab.cue_done")}</span>;
  return <span className="mori-pill-badge tone-neutral">{type}</span>;
}
```

> `state` 值來自 AgentPulse `SessionState`(serde `snake_case`):`idle / working / waiting_for_user / stale`。

- [ ] **Step 2: build 確認**

Run: `npm run build`
Expected: TS 無錯。

- [ ] **Step 3: Commit**

```bash
git add src/tabs/PulseTab.tsx
git commit -m "feat(bi-3): Pulse tab — subscribe AgentPulse /sessions + /events SSE

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

## Task B2: tab 接線(shellTabs / MainShell / icons / i18n)

**Files:** Modify `src/shellTabs.ts`, `src/MainShell.tsx`, `src/icons.tsx`, `src/i18n/locales/{zh-TW,en}.json`

- [ ] **Step 1: `shellTabs.ts`** — `SHELL_TAB_IDS` 在 `"permissions",` 後加一行 `"pulse",`(若 BI-2 的 permissions 已在;否則加在 `"body",` 後)。

- [ ] **Step 2: `icons.tsx`** — 接在 `IconPermissions`(或 `IconBody`)後加:
```tsx
// 💓 Pulse — 心跳波形(agent session 脈搏)。
export function IconPulse(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M3 12 h4 l2 -6 l3 12 l2 -6 h5" />
    </svg>
  );
}
```

- [ ] **Step 3: `MainShell.tsx`**(四處,照 BI-2 PermissionsTab 同 pattern):
  - icons import 加 `IconPulse`
  - `import PulseTab from "./tabs/PulseTab";`
  - `TABS` 陣列加 `{ id: "pulse", Icon: IconPulse, key: "pulse" },`(放 permissions 後)
  - render switch 加 `{tab === "pulse" && <PulseTab />}`

- [ ] **Step 4: `zh-TW.json`**
  - sidebar 區加:
    ```json
    "pulse": "脈搏",
    "pulse_sub": "AI session 監控(AgentPulse)",
    ```
  - 末尾(`,"<前一個>_tab"` 同逗號前綴風格)加:
    ```json
      ,"pulse_tab": {
        "title": "脈搏",
        "refresh": "重新整理",
        "hint": "訂閱 AgentPulse:看哪個 AI coding session 在跑 / 等你輸入 / 跑完了(唯讀;通知由 AgentPulse 膠囊發)。",
        "not_running": "找不到 AgentPulse(沒啟動,或還沒註冊)。開著 AgentPulse 再重新整理。",
        "sessions_title": "進行中的 session",
        "sessions_empty": "目前沒有 session。",
        "cues_title": "提示(cue)",
        "cues_empty": "還沒有 cue。",
        "cue_waiting": "？ 等輸入",
        "cue_done": "✓ 跑完"
      }
    ```

- [ ] **Step 5: `en.json`** — 對應英文:
    ```json
    "pulse": "Pulse",
    "pulse_sub": "AI session monitor (AgentPulse)",
    ```
    ```json
      ,"pulse_tab": {
        "title": "Pulse",
        "refresh": "Refresh",
        "hint": "Subscribes to AgentPulse: which AI coding session is running / waiting for you / done (read-only; notifications come from the AgentPulse capsule).",
        "not_running": "AgentPulse not found (not running, or not registered yet). Start AgentPulse and refresh.",
        "sessions_title": "Active sessions",
        "sessions_empty": "No sessions right now.",
        "cues_title": "Cues",
        "cues_empty": "No cues yet.",
        "cue_waiting": "？ waiting",
        "cue_done": "✓ done"
      }
    ```

- [ ] **Step 6: build + 既有測試**

Run: `npm run build && npm test`
Expected: build 成功;vitest 全綠(`shellTabs.test.ts` —— `"pulse"` 是合法 visible tab)。

- [ ] **Step 7: Commit**

```bash
git add src/shellTabs.ts src/MainShell.tsx src/icons.tsx src/i18n/locales/zh-TW.json src/i18n/locales/en.json
git commit -m "feat(bi-3): wire Pulse tab into sidebar + i18n

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

## Task B3: 手測(e2e 判準)+ 文件 + PR

- [ ] **Step 1: Manual verify(BI-3 done 判準)**

前置:AgentPulse(Part A 的 build)跑著、且在其 UI 開了 Codex 的 hook。
```bash
npm run tauri dev
```
1. 點側欄「脈搏」tab → 若 AgentPulse 在跑,看到 session 區(可能空)+ 無「找不到」訊息。
2. 開一個 Codex session 跑一輪 → tab 的 cue 區出現「？ 等輸入」;Codex 再跑完 → 「✓ 跑完」。**= 判準「跑一個 Codex session → Desktop 收到 waiting_input / done cue」**。
3. 連按兩輪確認**不重複**(同 event_id 只進一次)。
4. 關掉 AgentPulse → 重新整理 → 顯示「找不到 AgentPulse」不 crash。

- [ ] **Step 2: 文件 — `docs/body-interface-backlog.md`**

把 §4 BI-3 判準那行改 done:
```markdown
- **BI-3 done** ✅(YYYY-MM-DD)= AgentPulse 加 body-interface 對外口(manifest + /sessions + /events SSE,PR yazelin/AgentPulse#NN),mori-desktop 唯讀「脈搏」tab 訂閱;跑一個 Codex session,Desktop 收到 waiting_input / done cue(event_id dedup 不重複)。通知仍由 AgentPulse 膠囊發(§766 不重複)。
```

- [ ] **Step 3: Commit + PR + auto-merge**

```bash
git add docs/body-interface-backlog.md
git commit -m "docs(bi-3): mark AgentPulse integration done"
git push -u origin feat/bi-3-pulse-tab
gh pr create --base main --title "feat(bi-3): AgentPulse integration — Pulse tab" --body "..."
gh pr merge --auto --merge
```

---

## Self-Review

**1. Spec coverage**(backlog BI-3 + 判準 + body-interface §Events/§Agent Plus):
- AgentPulse 提供 manifest + /sessions + /events SSE(§Control/§Events)→ Part A ✓
- 偵測 waiting_input / done → 沿用既有 transition,Q1/A1 mapping ✓
- Desktop 顯示 session list + 最小 cue surface → Part B PulseTab ✓
- 「不重複」→ event_id dedup(B1)+ 通知留給 AgentPulse(Q4)✓
- 「跑一個 Codex session → 收到 cue」→ B3 手測判準 ✓
- 重用 BI-1 registry 發現 manifest → Q8,無新 Tauri command ✓
- BI-2 不直接相關(broker 是 BI-2),BI-3 observe-only 無高風險操作 ✓

**2. Placeholder scan:** 無 TBD。AgentPulse crate 名(`cargo test` target)實作時用實際 `[lib] name`;PR body 的 `...` 執行時填 —— 非設計空白。

**3. Type consistency:**
- AgentPulse `MoriEvent`(serde:`r#type`→ 序列化成 `"type"`)↔ mori TS `Cue.type` ✓;`event_id/session_id/severity/summary/time` 兩邊同名 ✓
- AgentPulse `SessionInfo`(serde camel? 否 —— Rust 預設,欄位 `provider/state/project_name/is_active/formatted_time` snake)↔ TS `SessionInfo` 同名 snake ✓
- `SessionState` serde `snake_case` → `idle/working/waiting_for_user/stale` ↔ TS `StateBadge` 判斷字串 ✓
- cue 型別字串 `cue.waiting_input`/`cue.done` 三處一致(A1 mapping → SSE → B1 CueBadge)✓
- manifest `id: mori.agent-pulse` 兩處一致(A1 寫入 ↔ B1 find)✓
- SSE/http url 由 manifest interfaces 帶 port,B1 依 transport 取 `url`/`base_url` ↔ A1 manifest 欄位名一致 ✓

**4. 範圍紀律:** AgentPulse 偵測**完全不動**;只加唯讀讀取口 + 廣播 + manifest 寫入。mori-desktop **零 Rust**(重用 BI-1),純一個唯讀 tab。**無**系統通知(避免重複)、**無** cue 操作(acknowledge/snooze 留 BI-4)、**無**啟停 AgentPulse(launch 留之後)。✓

---

## 全套驗證(收工前)
- AgentPulse:`cargo build` + `cargo test mori_bridge` + curl 手測(A4)
- mori-desktop:`bash scripts/verify.sh` + e2e 手測(B3)

**Branches:** `yazelin/AgentPulse` → `feat/mori-body-bridge`;`mori-desktop` → `feat/bi-3-pulse-tab`(各自 off 最新 main)。
**Plan saved:** `mori-desktop/docs/superpowers/plans/2026-05-28-bi-3-agentpulse-integration.md`
**Two PRs, both auto-merge.** A 先合,B 再合。
