//! Skill = LLM 可呼叫的工具。
//!
//! 設計原則:
//! - 每個 skill 自成一個 module,實作 [`Skill`] trait
//! - schema 從 Rust struct 自動產生(用 schemars)→ 直接餵給 LLM tool calling
//! - destructive 操作要 `confirm_required: true`,執行前 UI / TTS 二次確認
//! - 跨裝置:`target_capability` 表明這個 skill 能在哪跑(Local / Remote / Anywhere)

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context::Context;

/// 這個 skill 可以在哪邊執行?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionTarget {
    /// 只能本機(例如:存取本地檔案、控制本機音量)
    Local,
    /// 必須指定遠端裝置(例如:操作家裡那台桌機的某個服務)
    Remote(u64),
    /// 任一線上裝置都可以(由 LLM 或負載決定)
    Anywhere,
}

/// 隱私等級。LocalOnly 的 skill 強制只用本地 LLM,絕不送雲端。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Privacy {
    /// 可送雲端 LLM
    Cloud,
    /// 必須本地 LLM(例:讀信、操作敏感檔案)
    LocalOnly,
}

/// Skill 執行結果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillOutput {
    /// 給使用者看(自然語言)
    pub user_message: String,
    /// 結構化資料(供下游 skill 串接)
    pub data: Option<Value>,
}

#[async_trait]
pub trait Skill: Send + Sync {
    /// 給 LLM 看的 skill 名稱。應為唯一 snake_case 識別字串。
    fn name(&self) -> &'static str;

    /// 給 LLM 看的描述。盡量寫清楚「什麼時候該叫這個」。
    fn description(&self) -> &'static str;

    /// 參數的 JSON Schema(可從 Rust struct 用 schemars 產生)。
    fn parameters_schema(&self) -> Value;

    /// 是否需要二次確認?(destructive 操作為 true)
    fn confirm_required(&self) -> bool {
        false
    }

    /// 這 skill 能在哪邊執行?
    fn target_capability(&self) -> ExecutionTarget {
        ExecutionTarget::Local
    }

    /// 隱私等級。
    fn privacy(&self) -> Privacy {
        Privacy::Cloud
    }

    /// 執行 skill。
    async fn execute(&self, args: Value, context: &Context) -> Result<SkillOutput>;
}

/// Phase 1 內建 skill:把 LLM 的回應原樣轉給使用者。
///
/// 主要當作端到端 pipeline 的 sanity check 用。
pub struct EchoSkill;

#[async_trait]
impl Skill for EchoSkill {
    fn name(&self) -> &'static str {
        "echo"
    }

    fn description(&self) -> &'static str {
        "Repeat or rephrase the user's input back to them. Useful for confirming what was heard."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The text to echo back."
                }
            },
            "required": ["message"]
        })
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("(empty)")
            .to_string();
        Ok(SkillOutput {
            user_message: message,
            data: None,
        })
    }
}
