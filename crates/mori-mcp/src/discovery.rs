//! [`McpRegistry`] — 對所有 config 內的 MCP server 做 connect / aggregate。
//!
//! 設計原則:
//! - **個別 server fail 不擋整批**:某條 stdio command 找不到 binary / 某條 HTTP
//!   endpoint 連不上,只 log warn skip 它,registry 仍含其他成功的 server
//! - **Aggregate tool list**:`all_tools()` 把所有 connected server 的 tools 攤成
//!   一條 Vec,MCP-2 直接 iterate 註冊進 SkillRegistry
//! - **Call dispatch**:`call(server, tool, args)` 把 LLM 的 tool_call route 到
//!   對應 McpClient(MCP-2 用)
//!
//! 不做(留 MCP-2):
//! - tool name 衝突偵測(同一 server 內 unique;跨 server 由 caller 自己 namespace)
//! - hot-reload(改 mcp.json 後 re-connect)
//! - retry / circuit breaker(rmcp 本身 transport-level reconnect 不在範圍內)

use std::collections::HashMap;
use std::sync::Arc;

use crate::client::{McpClient, McpError, McpTool, McpToolResult};
use crate::config::McpConfig;

/// MCP server registry — 持有所有 connected client。
///
/// 用 `Arc<McpClient>` 因為 call_tool / shutdown 可能跨 task 進入,要 share。
/// 內部用 HashMap 以 server name 索引,O(1) dispatch。
pub struct McpRegistry {
    clients: HashMap<String, Arc<McpClient>>,
}

impl Default for McpRegistry {
    fn default() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }
}

impl McpRegistry {
    /// 依 config 逐個 connect。**個別 server fail 只 log + skip**,不 propagate。
    ///
    /// 回傳的 registry 可能只有 partial servers — caller 用 `connected_servers()`
    /// 或 `all_tools()` 查實際情況。
    ///
    /// 順序:照 `config.servers` 的順序 connect(serial,不平行 — 避免大量 child
    /// process 同時 spawn 造成 resource spike;一般 user 只配 3-5 個 server,
    /// serial 順序也只是 startup 時 ~1-2s 差,可接受)。
    pub async fn from_config(config: &McpConfig) -> Self {
        let mut clients = HashMap::new();
        for server in &config.servers {
            let name = server.name().to_string();
            // 名稱碰撞 → 用後寫的,warn。
            if clients.contains_key(&name) {
                tracing::warn!(server = %name, "duplicate MCP server name in config — replacing");
            }
            match McpClient::connect(server).await {
                Ok(client) => {
                    clients.insert(name.clone(), Arc::new(client));
                    tracing::info!(server = %name, "MCP server registered");
                }
                Err(e) => {
                    tracing::warn!(
                        server = %name,
                        error = %e,
                        "failed to connect MCP server — skipping; other servers continue",
                    );
                }
            }
        }
        tracing::info!(
            connected = clients.len(),
            configured = config.servers.len(),
            "McpRegistry initialized",
        );
        Self { clients }
    }

    /// 已 connect 的 server name list。
    pub fn connected_servers(&self) -> Vec<String> {
        self.clients.keys().cloned().collect()
    }

    /// 列出所有 connected server 的所有 tools(攤平)。
    ///
    /// 每個 server 走 `client.list_tools()` 各自的 RPC;個別 fail log + skip
    /// (同 connect 失敗策略 — 某條 server 之後變不穩,不擋其他 server 的 tools 暴露)。
    pub async fn all_tools(&self) -> Vec<McpTool> {
        let mut out = Vec::new();
        for (name, client) in &self.clients {
            match client.list_tools().await {
                Ok(tools) => {
                    tracing::debug!(server = %name, count = tools.len(), "listed tools");
                    out.extend(tools);
                }
                Err(e) => {
                    tracing::warn!(
                        server = %name,
                        error = %e,
                        "list_tools failed — skipping this server's tools",
                    );
                }
            }
        }
        out
    }

    /// Dispatch tool call。`server` 找不到回 `McpError::Rpc`(reuse 同 error
    /// variant,訊息 prefix `unknown server`)。
    pub async fn call(
        &self,
        server: &str,
        tool: &str,
        args: serde_json::Value,
    ) -> Result<McpToolResult, McpError> {
        let client = self.clients.get(server).ok_or_else(|| McpError::Rpc {
            server: server.to_string(),
            message: format!("unknown server: {server}"),
        })?;
        client.call_tool(tool, args).await
    }

    /// 取單一 client(MCP-2 註冊 Skill 時可能要保留 Arc)。
    pub fn get(&self, server: &str) -> Option<Arc<McpClient>> {
        self.clients.get(server).cloned()
    }

    /// Shutdown 全部 client。逐個 cancel;個別 fail log warn 不 abort。
    ///
    /// 拿 `self`(消費)— 避免 caller 拿後再 call。
    pub async fn shutdown(self) -> Result<(), McpError> {
        for (name, client) in self.clients {
            match Arc::try_unwrap(client) {
                Ok(c) => {
                    if let Err(e) = c.shutdown().await {
                        tracing::warn!(server = %name, error = %e, "MCP client shutdown failed");
                    }
                }
                Err(_) => {
                    tracing::debug!(server = %name, "MCP client still referenced — drop will handle");
                }
            }
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! `McpRegistry` tests 走 **真 child process,但用 stub command** —
    //! stdio MCP server 是 spawn `cmd args...`,測試只需要:
    //! 1. server 個別 fail 不擋整批 → 用「肯定 fail 的 command」(亂打 binary 名)
    //! 2. shutdown clean → registry empty 也能 shutdown
    //! 3. all_tools 在 0 server 時回空 Vec
    //!
    //! 為什麼不寫「能 connect 成功」的 integration test?— 那要真正 spawn
    //! 一個 MCP server child process(`@modelcontextprotocol/server-everything`
    //! 之類),依賴 Node.js + npx,CI / dev box 不見得有。MCP-2 / 後續會在
    //! integration test 層補,本層只 unit 驗證行為。
    use super::*;
    use crate::config::McpServerConfig;
    use std::collections::HashMap;

    #[tokio::test]
    async fn empty_config_returns_empty_registry() {
        let cfg = McpConfig::default();
        let reg = McpRegistry::from_config(&cfg).await;
        assert!(reg.connected_servers().is_empty());
        assert!(reg.all_tools().await.is_empty());
    }

    #[tokio::test]
    async fn mcp_registry_handles_individual_server_failure() {
        // 2 個 server entry,都用「絕對不存在的 binary」走 stdio。
        // 預期 from_config 不 panic、registry 0 connected。
        // 等於驗證「個別 server fail 不擋整批」+「全 fail 也 graceful」。
        let cfg = McpConfig {
            servers: vec![
                McpServerConfig::Stdio {
                    name: "fake-a".into(),
                    command: "/nonexistent/mori-mcp-fake-binary-a".into(),
                    args: vec![],
                    env: HashMap::new(),
                },
                McpServerConfig::Stdio {
                    name: "fake-b".into(),
                    command: "/nonexistent/mori-mcp-fake-binary-b".into(),
                    args: vec!["--never-runs".into()],
                    env: HashMap::new(),
                },
            ],
        };
        let reg = McpRegistry::from_config(&cfg).await;
        // 都 fail 但 from_config 仍 return,且沒 connected server。
        assert!(
            reg.connected_servers().is_empty(),
            "expected 0 connected, got {:?}",
            reg.connected_servers(),
        );
        // all_tools 也走 graceful path
        assert!(reg.all_tools().await.is_empty());
    }

    #[tokio::test]
    async fn mcp_registry_all_tools_aggregates_empty_when_no_servers() {
        // single-failure 也走 aggregate path — all_tools 在 0 connected
        // server 時直接回空 Vec,不 panic。
        let cfg = McpConfig {
            servers: vec![McpServerConfig::Stdio {
                name: "fail-only".into(),
                command: "/nonexistent/mori-mcp-aggregate-test".into(),
                args: vec![],
                env: HashMap::new(),
            }],
        };
        let reg = McpRegistry::from_config(&cfg).await;
        let tools = reg.all_tools().await;
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn mcp_registry_call_unknown_server_returns_rpc_error() {
        // dispatch 一個不存在的 server,期望 Rpc error(訊息含 unknown server)。
        let reg = McpRegistry::from_config(&McpConfig::default()).await;
        let err = reg
            .call("does-not-exist", "any-tool", serde_json::json!({}))
            .await
            .expect_err("unknown server should error");
        match err {
            McpError::Rpc { server, message } => {
                assert_eq!(server, "does-not-exist");
                assert!(
                    message.contains("unknown server"),
                    "expected 'unknown server' in message, got: {message}",
                );
            }
            other => panic!("expected Rpc, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mcp_registry_shutdown_clean_on_empty() {
        // empty registry → shutdown 不 panic,回 Ok。基線健全性。
        let reg = McpRegistry::from_config(&McpConfig::default()).await;
        reg.shutdown().await.expect("shutdown ok on empty");
    }

    #[tokio::test]
    async fn mcp_registry_shutdown_clean_after_all_failed() {
        // 連 fail 的 server registry 也能 clean shutdown(內部 0 client,
        // 等同 empty case;確認 from_config 失敗 path 沒留 dangling state)。
        let cfg = McpConfig {
            servers: vec![McpServerConfig::Stdio {
                name: "shutdown-test".into(),
                command: "/nonexistent/mori-mcp-shutdown-test".into(),
                args: vec![],
                env: HashMap::new(),
            }],
        };
        let reg = McpRegistry::from_config(&cfg).await;
        reg.shutdown().await.expect("shutdown ok after fail");
    }
}
