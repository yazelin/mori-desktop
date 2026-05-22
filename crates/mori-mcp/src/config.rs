//! `~/.mori/mcp.json` 設定 — 描述要 connect 的 MCP server list。
//!
//! 兩種 transport:
//! - **stdio**:spawn child process(e.g. `npx -y @modelcontextprotocol/server-github`)
//! - **http**:Streamable HTTP / SSE(remote MCP endpoint)
//!
//! 範例:
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
//!       "url": "https://mcp.example.com"
//!     }
//!   ]
//! }
//! ```
//!
//! 預設 config path 走 `~/.mori/mcp.json`(Windows fallback `%USERPROFILE%`)。
//! 對齊既有 mori_dir pattern(`mori-tauri/src/annuli_supervisor.rs::annuli_root_dir`)。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// MCP config root — 載入自 `~/.mori/mcp.json`。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct McpConfig {
    /// 要 connect 的 MCP server。空 list 等同於沒開 MCP,registry 不會嘗試連任何 server。
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

/// 單一 MCP server 的設定。`transport` 欄位 tag 區分 stdio / http 兩種。
///
/// 兩 variant 都有 `name` — 用來在 `McpRegistry` 內 identify、在 LLM tool dispatch 時
/// 用 `<server>:<tool>` 命名空間(MCP-2 會用到;MCP-1 只 list)。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "transport")]
pub enum McpServerConfig {
    /// stdio MCP server:spawn 子進程,server 用 stdin/stdout 講 JSON-RPC。
    /// 主要用於官方 `@modelcontextprotocol/server-*` 系列(走 `npx -y`)。
    #[serde(rename = "stdio")]
    Stdio {
        name: String,
        /// 執行檔(`"npx"` / `"node"` / 任何 binary)
        command: String,
        /// 啟動參數(e.g. `["-y", "@modelcontextprotocol/server-github"]`)
        #[serde(default)]
        args: Vec<String>,
        /// 環境變數(主要給 token / API key,e.g. `GITHUB_TOKEN`)
        #[serde(default)]
        env: HashMap<String, String>,
    },
    /// HTTP / Streamable HTTP MCP server。`url` 是完整 endpoint(含 path),
    /// 不含 trailing slash 一致比較好。
    #[serde(rename = "http")]
    Http { name: String, url: String },
}

impl McpServerConfig {
    /// 拿這個 server 的 name(兩 variant 都有)。
    pub fn name(&self) -> &str {
        match self {
            McpServerConfig::Stdio { name, .. } => name,
            McpServerConfig::Http { name, .. } => name,
        }
    }
}

/// Config 解析 / 載入錯誤。
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("read config: {0}")]
    Read(#[from] std::io::Error),
    #[error("parse mcp.json: {0}")]
    Parse(#[from] serde_json::Error),
}

/// 從 path 載入 MCP config。**檔案不存在會回 IO error**(NotFound)——
/// caller 通常會在呼叫前自己 `default_config_path().exists()` 判斷,
/// 沒檔案視同空 config（不開任何 MCP server）。
pub fn load_config(path: &Path) -> Result<McpConfig, ConfigError> {
    let raw = std::fs::read_to_string(path)?;
    let cfg: McpConfig = serde_json::from_str(&raw)?;
    Ok(cfg)
}

/// 預設 config path:`$HOME/.mori/mcp.json`（Windows 走 `%USERPROFILE%`）。
///
/// 兩個 env 都讀不到回 `None`(極少見;CI / headless env 可能會碰到)。caller
/// 拿到 None 應該當作「沒開 MCP」處理,不要 panic。
pub fn default_config_path() -> Option<PathBuf> {
    let home_var = if cfg!(target_os = "windows") {
        "USERPROFILE"
    } else {
        "HOME"
    };
    let home = std::env::var(home_var).ok()?;
    Some(PathBuf::from(home).join(".mori").join("mcp.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stdio_server_config() {
        let json = r#"{
            "servers": [
                {
                    "name": "github",
                    "transport": "stdio",
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-github"],
                    "env": { "GITHUB_TOKEN": "ghp_xxx" }
                }
            ]
        }"#;
        let cfg: McpConfig = serde_json::from_str(json).expect("parse ok");
        assert_eq!(cfg.servers.len(), 1);
        match &cfg.servers[0] {
            McpServerConfig::Stdio {
                name,
                command,
                args,
                env,
            } => {
                assert_eq!(name, "github");
                assert_eq!(command, "npx");
                assert_eq!(args, &vec!["-y".to_string(), "@modelcontextprotocol/server-github".to_string()]);
                assert_eq!(env.get("GITHUB_TOKEN").map(String::as_str), Some("ghp_xxx"));
            }
            other => panic!("expected Stdio, got {other:?}"),
        }
    }

    #[test]
    fn parse_http_server_config() {
        let json = r#"{
            "servers": [
                { "name": "remote", "transport": "http", "url": "https://mcp.example.com/mcp" }
            ]
        }"#;
        let cfg: McpConfig = serde_json::from_str(json).expect("parse ok");
        assert_eq!(cfg.servers.len(), 1);
        match &cfg.servers[0] {
            McpServerConfig::Http { name, url } => {
                assert_eq!(name, "remote");
                assert_eq!(url, "https://mcp.example.com/mcp");
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn parse_mixed_config() {
        // stdio + http 同時 coexist;同時測 stdio 的 `args` / `env` 可省略(default 空)。
        let json = r#"{
            "servers": [
                { "name": "playwright", "transport": "stdio", "command": "npx", "args": ["-y", "@modelcontextprotocol/server-playwright"] },
                { "name": "remote", "transport": "http", "url": "https://x.example/mcp" }
            ]
        }"#;
        let cfg: McpConfig = serde_json::from_str(json).expect("parse ok");
        assert_eq!(cfg.servers.len(), 2);
        assert_eq!(cfg.servers[0].name(), "playwright");
        assert_eq!(cfg.servers[1].name(), "remote");
        // stdio entry 沒給 env → 預設空 map
        match &cfg.servers[0] {
            McpServerConfig::Stdio { env, .. } => assert!(env.is_empty()),
            other => panic!("expected Stdio, got {other:?}"),
        }
    }

    #[test]
    fn default_config_path_uses_home() {
        // unit test 不 cross-mutate 全域 env(其他 test 可能在 parallel 跑),
        // 改用直接 assert path shape:HOME / USERPROFILE 在我們的 CI / dev env
        // 預設都有設,拿不到的話跳過(極少見場景)。
        let Some(path) = default_config_path() else {
            // headless env 沒 HOME / USERPROFILE — 視同 None 正常路徑,skip。
            return;
        };
        assert!(path.ends_with(".mori/mcp.json") || path.ends_with(".mori\\mcp.json"));
        assert!(path.is_absolute(), "expected absolute path, got {path:?}");
    }

    #[test]
    fn load_config_returns_error_for_invalid_json() {
        // 在 tempdir 寫一個壞 JSON,確認 load_config 回 Parse error 而非 panic。
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        std::fs::write(&path, "{ not valid json").unwrap();
        let err = load_config(&path).expect_err("invalid json should fail");
        assert!(
            matches!(err, ConfigError::Parse(_)),
            "expected Parse error, got {err:?}"
        );
    }

    #[test]
    fn load_config_round_trip() {
        // 寫一份完整 config → load → 內容一致。
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        let content = r#"{
            "servers": [
                { "name": "fs", "transport": "stdio", "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"] }
            ]
        }"#;
        std::fs::write(&path, content).unwrap();
        let cfg = load_config(&path).expect("load ok");
        assert_eq!(cfg.servers.len(), 1);
        assert_eq!(cfg.servers[0].name(), "fs");
    }

    #[test]
    fn empty_servers_default() {
        // 缺 servers 欄位 → default 空 list,不 error。讓 user 寫 `{}` 也合法。
        let cfg: McpConfig = serde_json::from_str("{}").expect("parse ok");
        assert!(cfg.servers.is_empty());
    }
}
