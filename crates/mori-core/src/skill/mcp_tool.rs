//! McpToolSkill — 把 mori-mcp 提供的 [`McpTool`](mori_mcp::McpTool) 包成
//! [`Skill`] trait,讓 LLM 透過 [`SkillRegistry::dispatch`](super::SkillRegistry::dispatch)
//! reach 到外部 MCP server。
//!
//! # Wave 6 MCP-2 範圍
//!
//! MCP-1(`mori-mcp` crate)已經把:
//! - 連線(`McpClient::connect`)
//! - aggregate tool list(`McpRegistry::all_tools`)
//! - dispatch call(`McpRegistry::call`)
//!
//! 三件事都做完了。但 mori-core / mori-tauri 的 [`SkillRegistry`](super::SkillRegistry)
//! 看不見 MCP tools — LLM 拿不到 tool definition,即使知道有也找不到 implementation。
//! 這個 module 是缺口:把每個 [`McpTool`](mori_mcp::McpTool) 包成一個 [`Skill`]
//! 物件,塞進 SkillRegistry,讓 LLM 用 `tool_call` 就能 reach 到。
//!
//! # 名稱規則
//!
//! Skill name format:`mcp_<server>_<tool>`。
//!
//! - **加 `mcp_` prefix** 避免跟既有 skill(`read_file_text` / `remind_me` /
//!   `remember` / `polish` 等)collision
//! - **包含 server name** 避免不同 MCP server 提供同名 tool 時內部衝突
//!   (e.g. GitHub MCP 跟 GitLab MCP 都可能有 `create_issue`)
//!
//! # `&'static str` 對齊
//!
//! [`Skill::name`] / [`Skill::description`] 都要回 `&'static str`,但 MCP tool
//! 名稱跟描述是 runtime 從 server 拉的動態字串。沿用 [`AnthropicPromptSkill`]
//! (super::anthropic_skill::AnthropicPromptSkill)的做法:`Box::leak` 把 `String`
//! 升級成 `&'static str`。每個 MCP server connect 時一次性 leak,量小(一個
//! server 通常 5-30 個 tool),生命週期跟 process 同步,可接受。
//!
//! # Privacy / Target
//!
//! - [`ExecutionTarget::Anywhere`]:MCP tool dispatch 本身要 talk to McpRegistry
//!   (它持有的 child process / HTTP transport 在哪台機就在哪;不綁定特定裝置)。
//!   既有 `ExecutionTarget` enum 只有 `Local` / `Remote(id)` / `Anywhere` 三個
//!   variant — 用 `Anywhere` 對齊「裝有 mori-tauri 的任一裝置都能跑」的語義
//!   (對齊 [`FetchUrlSkill`](super::FetchUrlSkill) 等也吃外部 service 的 skill)。
//! - [`Privacy::Cloud`]:MCP tool 結果預設可餵回 LLM 做 multi-turn;user 想嚴
//!   私自己選 LocalOnly provider 即可
//!
//! # 為什麼 forward `input_schema` 不做 transform
//!
//! MCP server 給的 `input_schema` 已經是 JSON Schema,跟 OpenAI / Groq tool-calling
//! 預期格式一致。直接 forward 給 LLM,參數驗證/補全由 LLM 自己處理(rmcp 在
//! call_tool 也不會強驗 schema — server 端自己判;我們不另加一層)。

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use mori_mcp::{McpRegistry, McpTool};
use serde_json::Value;

use super::{ExecutionTarget, Privacy, Skill, SkillOutput};
use crate::context::Context;

/// `Skill` wrapper for 一個 MCP server 提供的 tool。
///
/// 一個 [`McpToolSkill`] 對應一個 `(server, tool)` pair。多個 tool 的 server
/// 會建出多個 `McpToolSkill`(`McpRegistry::all_tools()` 攤平後 iterate)。
///
/// `Arc<McpRegistry>` 是 share 用 — startup 時建一份 registry,所有 wrapped
/// skill 都拿同一個 Arc(call dispatch 時 route 到正確 server)。
pub struct McpToolSkill {
    registry: Arc<McpRegistry>,
    /// 原 MCP tool descriptor。`server` / `name` 用來 dispatch call,
    /// `input_schema` forward 給 LLM。
    tool: McpTool,
    /// `mcp_<server>_<tool>`,leaked 成 `'static`。
    leaked_name: &'static str,
    /// `[<server>] <description>`,leaked 成 `'static`。
    leaked_description: &'static str,
}

impl McpToolSkill {
    /// Build wrapper。`leak` 兩個動態字串到 `'static`(對齊
    /// [`AnthropicPromptSkill`](super::anthropic_skill::AnthropicPromptSkill) /
    /// `ShellSkill` pattern)。
    pub fn new(registry: Arc<McpRegistry>, tool: McpTool) -> Self {
        let name_string = format!("mcp_{}_{}", tool.server, tool.name);
        let desc_string = if tool.description.is_empty() {
            format!("[{}] MCP tool", tool.server)
        } else {
            format!("[{}] {}", tool.server, tool.description)
        };
        let leaked_name: &'static str = Box::leak(name_string.into_boxed_str());
        let leaked_description: &'static str = Box::leak(desc_string.into_boxed_str());
        Self {
            registry,
            tool,
            leaked_name,
            leaked_description,
        }
    }

    /// Convenience constructor — 包成 `Arc<dyn Skill>` 給 main.rs 註冊。
    pub fn into_arc_skill(self) -> Arc<dyn Skill> {
        Arc::new(self)
    }

    /// 給 introspection / debug 用:回原 MCP tool 的 server name。
    pub fn server_name(&self) -> &str {
        &self.tool.server
    }

    /// 給 introspection / debug 用:回原 MCP tool 內部名稱(沒有 `mcp_<server>_` prefix)。
    pub fn tool_name(&self) -> &str {
        &self.tool.name
    }
}

#[async_trait]
impl Skill for McpToolSkill {
    fn name(&self) -> &'static str {
        self.leaked_name
    }

    fn description(&self) -> &'static str {
        self.leaked_description
    }

    fn parameters_schema(&self) -> Value {
        // 直接 forward MCP server 給的 JSON Schema。LLM 自己解讀;rmcp call_tool
        // 也不強驗 schema(server 端自己判)。
        self.tool.input_schema.clone()
    }

    fn target_capability(&self) -> ExecutionTarget {
        // `Anywhere`:MCP dispatch 透過 McpRegistry 走 child process / HTTP transport,
        // 任一裝有 mori-tauri + McpRegistry 的裝置都能跑(對齊 FetchUrlSkill 等
        // 同樣 fetch 外部 service 的 skill)。
        ExecutionTarget::Anywhere
    }

    fn privacy(&self) -> Privacy {
        // 跟 ReadFileSkill / AnthropicPromptSkill 一致:Cloud(user 想 LocalOnly
        // 自己挑 provider)。
        Privacy::Cloud
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        tracing::info!(
            skill = self.leaked_name,
            server = %self.tool.server,
            tool = %self.tool.name,
            "MCP tool dispatch",
        );

        let result = self
            .registry
            .call(&self.tool.server, &self.tool.name, args)
            .await
            .map_err(|e| anyhow::anyhow!("mcp call failed: {e}"))?;

        // MCP protocol 區分「RPC 成功但 tool 端報錯」(`is_error: true`)跟
        // 「RPC 本身失敗」(McpError::Rpc)。前者我們把 content 當錯誤訊息往
        // 上拋(讓 LLM 看到具體錯而非 generic「mcp_*_* failed」)。
        if result.is_error {
            return Err(anyhow::anyhow!(
                "mcp tool [{}] {} returned error: {}",
                self.tool.server,
                self.tool.name,
                result.content
            ));
        }

        Ok(SkillOutput {
            user_message: result.content.clone(),
            data: Some(serde_json::json!({
                "mcp_server": self.tool.server,
                "mcp_tool": self.tool.name,
                "content": result.content,
            })),
        })
    }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Tests 不需要真 MCP server。McpRegistry 用 `default()`(empty)build,
    //! McpTool 用手寫 DTO struct literal。execute() 走「unknown server」path
    //! 觸發 `McpError::Rpc`,驗證 dispatch wire 通到 registry。
    //!
    //! 真實 server 連線測試在 MCP-1 `discovery.rs` tests 已經有,加 integration
    //! test 要 Node.js / npx + real MCP server,放 follow-up。

    use super::*;
    use mori_mcp::{McpConfig, McpRegistry};
    use std::sync::Arc;

    /// 建 empty registry(default config,0 servers)。
    async fn empty_registry() -> Arc<McpRegistry> {
        Arc::new(McpRegistry::from_config(&McpConfig::default()).await)
    }

    fn fake_tool(server: &str, name: &str, desc: &str) -> McpTool {
        McpTool {
            server: server.to_string(),
            name: name.to_string(),
            description: desc.to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "search query" }
                },
                "required": ["query"]
            }),
        }
    }

    #[tokio::test]
    async fn mcp_tool_skill_name_has_prefix() {
        let registry = empty_registry().await;
        let tool = fake_tool("github", "create_issue", "create a GH issue");
        let skill = McpToolSkill::new(registry, tool);
        // prefix `mcp_` + server + tool — 避免跟既有 read_file_text / remind_me / 等 collision
        assert_eq!(skill.name(), "mcp_github_create_issue");
    }

    #[tokio::test]
    async fn mcp_tool_skill_description_includes_server() {
        let registry = empty_registry().await;
        let tool = fake_tool("notion", "search_pages", "search Notion pages");
        let skill = McpToolSkill::new(registry, tool);
        let desc = skill.description();
        // 訊息一定含 server name(讓 LLM / UI 知道這 tool 從哪個 server 來)
        assert!(
            desc.contains("notion"),
            "description should reference server, got: {desc}"
        );
        assert!(
            desc.contains("search Notion pages"),
            "description should preserve original tool description, got: {desc}"
        );
    }

    #[tokio::test]
    async fn mcp_tool_skill_description_falls_back_when_empty() {
        // 部分 MCP server 給空 description,wrapper 必須仍給 LLM 看得懂的非空文字。
        let registry = empty_registry().await;
        let tool = fake_tool("slack", "list_channels", "");
        let skill = McpToolSkill::new(registry, tool);
        let desc = skill.description();
        assert!(!desc.is_empty());
        assert!(desc.contains("slack"));
    }

    #[tokio::test]
    async fn mcp_tool_skill_forwards_schema() {
        let registry = empty_registry().await;
        let tool = fake_tool("github", "search", "search GH");
        let skill = McpToolSkill::new(registry, tool);
        let schema = skill.parameters_schema();
        // 直接 forward 原 input_schema;LLM tool-calling 拿到原樣
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["query"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "query"));
    }

    #[tokio::test]
    async fn mcp_tool_skill_target_and_privacy() {
        let registry = empty_registry().await;
        let tool = fake_tool("github", "list_issues", "list issues");
        let skill = McpToolSkill::new(registry, tool);
        assert_eq!(skill.target_capability(), ExecutionTarget::Anywhere);
        assert_eq!(skill.privacy(), Privacy::Cloud);
    }

    #[tokio::test]
    async fn mcp_tool_skill_execute_dispatches_to_registry() {
        // empty registry — server 不存在 → registry.call() 回 McpError::Rpc。
        // 驗證 execute() 真的有走 registry dispatch path(而不是直接成功 / 別處返回)。
        let registry = empty_registry().await;
        let tool = fake_tool("nonexistent-server", "any_tool", "");
        let skill = McpToolSkill::new(registry, tool);

        let ctx = Context::default();
        let err = skill
            .execute(serde_json::json!({"query": "x"}), &ctx)
            .await
            .expect_err("dispatching to unknown server should error");

        let msg = err.to_string();
        // mori-mcp 端 unknown server 訊息:"unknown server: nonexistent-server"
        // 我們的 wrapper 把它 wrap 進 "mcp call failed: rpc to MCP server ... unknown server ..."
        assert!(
            msg.contains("mcp call failed"),
            "expected wrapper prefix, got: {msg}"
        );
        assert!(
            msg.contains("unknown server"),
            "expected unknown server in chain, got: {msg}"
        );
    }

    #[tokio::test]
    async fn mcp_tool_skill_server_and_tool_introspection() {
        // server_name() / tool_name() 給 introspection 用(UI、debug log)。
        let registry = empty_registry().await;
        let tool = fake_tool("github", "create_issue", "");
        let skill = McpToolSkill::new(registry, tool);
        assert_eq!(skill.server_name(), "github");
        assert_eq!(skill.tool_name(), "create_issue");
    }

    #[tokio::test]
    async fn mcp_tool_skill_into_arc_skill_works() {
        // 確認 `into_arc_skill()` 真的能塞進 SkillRegistry 一樣的 trait object。
        let registry = empty_registry().await;
        let tool = fake_tool("x", "y", "test");
        let skill = McpToolSkill::new(registry, tool);
        let arc_skill: Arc<dyn Skill> = skill.into_arc_skill();
        assert_eq!(arc_skill.name(), "mcp_x_y");
    }
}
