//! [`McpClient`] — 單一 MCP server 的 connection wrapper(rmcp 包裝)。
//!
//! 設計重點:
//! - **transport-agnostic API**:caller 給 `&McpServerConfig`,內部依 variant 用
//!   `TokioChildProcess`(stdio)或 `StreamableHttpClientTransport`(http)
//! - **不 leak rmcp 型別給 caller**:list_tools 回 our own [`McpTool`] DTO,
//!   call_tool 回 [`McpToolResult`]。MCP-2(Skill 整合)只 import 我們的型別,
//!   不依賴 rmcp 內部 type — 升 rmcp 不會 cascade 進 mori-core
//! - **shutdown clean**:`shutdown()` 走 `RunningService::cancel()`,讓 background
//!   task 收尾;rmcp 也會在 drop 時 best-effort cancel,但 explicit shutdown 才有
//!   把握等到 transport 真的關
//!
//! Wave 6 MCP-1 範圍:connect / list / (call 已實作但 MCP-2 才整合進 SkillRegistry)/ shutdown。

use std::sync::Arc;

use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation},
    service::{RoleClient, RunningService},
    transport::{ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess},
};
use tokio::process::Command;

use crate::config::{ConfigError, McpServerConfig};

/// MCP client errors。包 connect / RPC / config 三條。
///
/// 故意把 rmcp 內部 error 字串化(`String`),避免 `rmcp::ServiceError` /
/// `ClientInitializeError` 透到 caller。caller 只看 `Connect` / `Rpc` 大類就夠。
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("connect MCP server {server}: {message}")]
    Connect { server: String, message: String },
    #[error("rpc to MCP server {server}: {message}")]
    Rpc { server: String, message: String },
    #[error("config: {0}")]
    Config(#[from] ConfigError),
}

/// 已 connect 的 MCP server handle。`Arc<RunningService<RoleClient, ()>>` 仰賴
/// rmcp 提供的 `RunningService<RoleClient>` —— 它 wrap 了 transport + background
/// task,deref 到 `Peer<RoleClient>`,所有 RPC method 在這上面 call。
///
/// `()` 是「沒 client-side handler」的占位 — MCP-1 不處理 server 主動 request
/// (sampling / roots),走 default no-op handler。MCP-2 / Wave 6 後續若要支援
/// sampling,可以換成自家的 `ClientHandler` struct,API 保持不變。
pub struct McpClient {
    name: String,
    inner: Arc<RunningService<RoleClient, ClientInfo>>,
}

/// 一個 MCP server 暴露出來的 tool(我們的 DTO,不直接 expose rmcp 內部 `Tool`)。
///
/// `input_schema` 是 JSON Schema 物件,給 LLM tool-calling 用。Wave 6 MCP-2 會
/// 把它包成 `Skill` trait 的 `parameters_schema()` 回傳。
#[derive(Debug, Clone, serde::Serialize)]
pub struct McpTool {
    /// Tool 所屬的 server name(`McpServerConfig::name`)
    pub server: String,
    /// Tool 名稱(MCP server 端定義,例 `"github_create_issue"`)
    pub name: String,
    /// 給 LLM 看的描述(可能空 — MCP server 不一定都填)
    pub description: String,
    /// Tool 參數 JSON Schema(直接給 LLM tool-calling 用)
    pub input_schema: serde_json::Value,
}

/// MCP tool 執行結果 DTO。`content` 是已序列化的文字(可能是 JSON / plain text /
/// embedded text resource);MCP-2 把它直接餵回 LLM 當 tool result。
#[derive(Debug, Clone)]
pub struct McpToolResult {
    /// 給 LLM 看的文字 — MCP server 回的 `content[]` 平鋪成單一 string。
    /// 非 Text 型別(image / audio / embedded resource)用 placeholder 表示
    /// (e.g. `"[image: image/png, 12345 bytes]"`),避免 dump binary 給 LLM。
    pub content: String,
    /// MCP 協議的 `isError` 旗標。`true` 代表 tool 端報錯,但 RPC 本身 OK。
    pub is_error: bool,
}

impl McpClient {
    /// Connect 到一個 MCP server。依 transport variant 選 child process / HTTP。
    ///
    /// **timeout**:目前沒設(rmcp 預設 InitializeRequest 走 channel,不會永等)。
    /// `discovery::McpRegistry` 層會用 `tokio::time::timeout` 包這條 call 做硬上限。
    pub async fn connect(server: &McpServerConfig) -> Result<Self, McpError> {
        let name = server.name().to_string();
        // ClientInfo (= InitializeRequestParams) 是 #[non_exhaustive],只能走
        // 公開 builder。`ClientInfo::new(caps, impl)` 是 rmcp 的標準 ctor。
        let client_info = ClientInfo::new(
            ClientCapabilities::default(),
            Implementation::new("mori-mcp", env!("CARGO_PKG_VERSION")),
        );

        let running = match server {
            McpServerConfig::Stdio {
                command,
                args,
                env,
                ..
            } => {
                let cmd_args = args.clone();
                let cmd_env = env.clone();
                let transport = TokioChildProcess::new(Command::new(command).configure(
                    move |cmd| {
                        cmd.args(&cmd_args);
                        for (k, v) in &cmd_env {
                            cmd.env(k, v);
                        }
                    },
                ))
                .map_err(|e| McpError::Connect {
                    server: name.clone(),
                    message: format!("spawn child: {e}"),
                })?;
                client_info
                    .clone()
                    .serve(transport)
                    .await
                    .map_err(|e| McpError::Connect {
                        server: name.clone(),
                        message: format!("stdio serve: {e}"),
                    })?
            }
            McpServerConfig::Http { url, .. } => {
                let transport = StreamableHttpClientTransport::from_uri(url.as_str());
                client_info
                    .clone()
                    .serve(transport)
                    .await
                    .map_err(|e| McpError::Connect {
                        server: name.clone(),
                        message: format!("http serve: {e}"),
                    })?
            }
        };

        tracing::info!(
            server = %name,
            peer_info = ?running.peer_info().map(|p| (&p.server_info.name, &p.server_info.version)),
            "MCP server connected",
        );

        Ok(Self {
            name,
            inner: Arc::new(running),
        })
    }

    /// Server name(來自 config）。
    pub fn name(&self) -> &str {
        &self.name
    }

    /// List 該 server 提供的所有 tools。內部走 `list_all_tools()`(分頁自動 fold)。
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        let tools = self
            .inner
            .list_all_tools()
            .await
            .map_err(|e| McpError::Rpc {
                server: self.name.clone(),
                message: format!("list_tools: {e}"),
            })?;
        Ok(tools
            .into_iter()
            .map(|t| McpTool {
                server: self.name.clone(),
                name: t.name.to_string(),
                description: t
                    .description
                    .map(|c| c.to_string())
                    .unwrap_or_default(),
                // rmcp Tool.input_schema 是 Arc<JsonObject> → 包成 Value::Object。
                input_schema: serde_json::Value::Object((*t.input_schema).clone()),
            })
            .collect())
    }

    /// Call 該 server 上的 tool。`args` 是 JSON object(對應 `input_schema`)。
    ///
    /// MCP-1 已實作,但 SkillRegistry 整合留 MCP-2。MCP server 回 `isError = true`
    /// 不算 RPC 錯誤(會走 `Ok(McpToolResult { is_error: true, ... })`),
    /// caller 自己判斷;只有 transport / RPC 失敗才回 `Err(McpError::Rpc)`。
    pub async fn call_tool(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<McpToolResult, McpError> {
        let mut params = CallToolRequestParams::new(name.to_string());
        // CallToolRequestParams.arguments 是 `Option<JsonObject>`(= serde_json::Map)。
        // 接受 null / 物件;非物件就空 args。
        if let serde_json::Value::Object(map) = args {
            params = params.with_arguments(map);
        }
        let result = self
            .inner
            .call_tool(params)
            .await
            .map_err(|e| McpError::Rpc {
                server: self.name.clone(),
                message: format!("call_tool {name}: {e}"),
            })?;

        // 把 content[] 平鋪成 string。text → 原文;非 text → placeholder。
        let mut buf = String::new();
        for (i, item) in result.content.iter().enumerate() {
            if i > 0 {
                buf.push('\n');
            }
            if let Some(text) = item.as_text() {
                buf.push_str(&text.text);
            } else if let Some(image) = item.as_image() {
                buf.push_str(&format!(
                    "[image: {}, {} bytes base64]",
                    image.mime_type,
                    image.data.len()
                ));
            } else if let Some(res) = item.as_resource() {
                // EmbeddedResource 內可能是 text 或 blob。對 LLM 而言:
                // - text → 直接餵內容
                // - blob → 只表示「embedded blob」+ uri,不 dump base64
                match &res.resource {
                    rmcp::model::ResourceContents::TextResourceContents { uri, text, .. } => {
                        buf.push_str(&format!("[embedded resource {uri}]\n{text}"));
                    }
                    rmcp::model::ResourceContents::BlobResourceContents { uri, blob, .. } => {
                        buf.push_str(&format!(
                            "[embedded blob resource {uri}: {} bytes base64]",
                            blob.len()
                        ));
                    }
                }
            } else {
                buf.push_str("[unsupported content type]");
            }
        }

        Ok(McpToolResult {
            content: buf,
            is_error: result.is_error.unwrap_or(false),
        })
    }

    /// Clean shutdown — cancel background task 等 transport 真的關。
    ///
    /// 拿 `self` 消費掉,避免 caller shutdown 後又 call list_tools。
    /// Arc 在 `RunningService` 上;如果還有別人持有同一個 Arc(理論上 McpRegistry
    /// 內部不會 share McpClient),拿不出 owned value、走 drop fallback(rmcp 也會
    /// 在 drop 時 cancel,只是 best-effort)。
    pub async fn shutdown(self) -> Result<(), McpError> {
        match Arc::try_unwrap(self.inner) {
            Ok(running) => {
                running.cancel().await.map_err(|e| McpError::Rpc {
                    server: self.name.clone(),
                    message: format!("cancel: {e}"),
                })?;
                Ok(())
            }
            Err(_arc) => {
                // 還有別人 ref — drop 我們這份,讓對方持有就好。
                // 真正關 transport 要等所有 ref drop;不算 error。
                tracing::debug!(
                    server = %self.name,
                    "McpClient shutdown skipped: another Arc holder exists, deferring to drop",
                );
                Ok(())
            }
        }
    }
}
