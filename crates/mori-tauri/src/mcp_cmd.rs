//! Wave 6 MCP-2:Tauri commands 給前端 list / call MCP tools。
//!
//! 主要消費者是 LLM 走 `SkillRegistry::dispatch` 那條路徑(`McpToolSkill`
//! 已經包好,LLM tool_call 即可 reach);這邊提供的是 **前端調試 / UI 入口**:
//!
//! - `mcp_list_tools_cmd`:前端 SkillsTab 顯示「目前連到哪些 MCP server / 各自
//!   提供哪些 tool」,方便 user 知道有什麼能用
//! - `mcp_call_tool_cmd`:前端可手動 dispatch 單一 tool(debug / 試試看接的對不對),
//!   不經 LLM
//!
//! McpRegistry 從 Tauri Manager 拿(main 啟動已 `.manage(mcp_registry)`)。
//! 啟動失敗(config 讀不到 / server 全部 connect 不上)時 registry 是 empty,
//! 兩個 command 都還是能跑(回空 list / "unknown server" error),不擋 UI。

use std::sync::Arc;

use mori_mcp::{McpRegistry, McpTool};

#[tauri::command]
pub async fn mcp_list_tools_cmd(
    registry: tauri::State<'_, Arc<McpRegistry>>,
) -> Result<Vec<McpTool>, String> {
    // McpRegistry::all_tools() 是 async(走每個 client.list_tools RPC),
    // tauri command 已經是 async context,直接 await。
    Ok(registry.all_tools().await)
}

#[tauri::command]
pub async fn mcp_call_tool_cmd(
    registry: tauri::State<'_, Arc<McpRegistry>>,
    server: String,
    tool: String,
    args: serde_json::Value,
) -> Result<String, String> {
    // 失敗時把整段 mcp error chain 轉 string 回前端;is_error: true(MCP tool
    // 端報錯但 RPC OK)的 content 也直接回給 user 看(對齊 LLM dispatch path:
    // McpToolSkill::execute 那邊則是把 is_error 包成 anyhow err 往 LLM 拋)。
    let result = registry
        .call(&server, &tool, args)
        .await
        .map_err(|e| e.to_string())?;
    if result.is_error {
        return Err(format!("mcp tool returned error: {}", result.content));
    }
    Ok(result.content)
}
