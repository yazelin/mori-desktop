//! mori-mcp — Model Context Protocol client integration.
//!
//! 讓 Mori 能連外部 MCP server(chrome-devtools / Notion / GitHub / Slack /
//! filesystem / playwright 等官方 MCP 實作),拉出 tools 餵給 LLM 用。
//!
//! ## Wave 6 範圍
//!
//! 兩個 sub-stream:
//!
//! - **MCP-1(本 crate,foundation)**:
//!   - [`config`]:`~/.mori/mcp.json` schema(stdio / http)
//!   - [`client::McpClient`]:rmcp client wrapper(connect / list_tools / call_tool / shutdown)
//!   - [`discovery::McpRegistry`]:多 server 管理(從 config build、aggregate tools、dispatch call)
//!
//! - **MCP-2(接續,留待後續 PR)**:
//!   - 把 [`McpTool`] 包成 mori-core `Skill` trait,註冊進 SkillRegistry
//!   - mori-tauri AppState 持有 `Arc<McpRegistry>`,在 startup load config + connect
//!   - system prompt 加 MCP tool 描述
//!   - Deps 頁 Node.js detect(多數 MCP server 走 `npx`)
//!
//! ## Config 範例
//!
//! `~/.mori/mcp.json`:
//!
//! ```json
//! {
//!   "servers": [
//!     {
//!       "name": "github",
//!       "transport": "stdio",
//!       "command": "npx",
//!       "args": ["-y", "@modelcontextprotocol/server-github"],
//!       "env": { "GITHUB_TOKEN": "..." }
//!     },
//!     {
//!       "name": "remote-mcp",
//!       "transport": "http",
//!       "url": "https://mcp.example.com/mcp"
//!     }
//!   ]
//! }
//! ```
//!
//! ## 為什麼 rmcp 而不是自己刻
//!
//! rmcp 是 MCP 官方 Rust SDK,支援:
//! - 兩條主要 transport(child-process stdio、Streamable HTTP)
//! - 完整 protocol model(Tool / Resource / Prompt / Sampling)
//! - 自動 InitializeRequest / capability negotiation
//!
//! 我們的 wrapper 是「only expose 必要 type、隔離 rmcp 版本變動」,讓 mori-core
//! 不直接 import rmcp(MCP-2 接 SkillRegistry 時看得最清楚)。

pub mod client;
pub mod config;
pub mod discovery;

pub use client::{McpClient, McpError, McpTool, McpToolResult};
pub use config::{default_config_path, load_config, ConfigError, McpConfig, McpServerConfig};
pub use discovery::McpRegistry;
