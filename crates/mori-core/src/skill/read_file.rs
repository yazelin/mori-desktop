//! ReadFileSkill — LLM 可呼叫的「讀檔案」工具,接 `mori-file-loader::read_file_text`。
//!
//! # 為什麼存在
//!
//! `mori-file-loader` 公開 API 已經就緒,Tauri side 也有 `read_file_text_cmd`
//! 給前端 JS 呼叫,但 **LLM 看 system prompt 知道有 `read_file_text` 工具卻
//! 沒有實作路徑** — 走 LLM tool dispatch 的入口是 `SkillRegistry::dispatch`,
//! 必須對應一個 `Skill` impl。這個 skill 就是那個缺口。
//!
//! # 行為
//!
//! - 吃 `{"path": "<absolute or relative path>"}` 參數
//! - 呼叫 `mori_file_loader::read_file_text` 拿純文字內容
//! - 成功:`SkillOutput.user_message` = 檔案內容,`data` 帶 `{ path, chars, bytes }`
//! - 失敗(檔不存在 / 副檔名不支援 / 非 UTF-8 / IO):透過 `anyhow!` 往上拋
//!
//! 副檔名支援由 mori-file-loader 決定:現階段 `.txt` / `.md`;後續加 `.pdf` /
//! `.docx` / `.xlsx` 等 reader 時這邊**不用改**,自動跟著支援。
//!
//! # 平台 / 隱私
//!
//! 讀本地檔案 → `ExecutionTarget::Local`,`Privacy::Cloud`(讀到的內容可能要餵
//! 回 LLM 做 multi-turn,所以不限制 LocalOnly;若使用者擔心,自己選 LocalOnly
//! provider 即可)。

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;

use super::{ExecutionTarget, Privacy, Skill, SkillOutput};
use crate::context::Context;

pub struct ReadFileSkill;

#[async_trait]
impl Skill for ReadFileSkill {
    fn name(&self) -> &'static str {
        "read_file_text"
    }

    fn description(&self) -> &'static str {
        "讀檔案內容回傳純文字。使用者提到「這份檔案 / 這個文件 / 幫我看看 <path>」\
         之類需求且有具體路徑時呼叫。支援 .txt / .md;mori-file-loader 後續加 \
         .pdf / .docx / .xlsx reader 時會自動跟著支援。\
         路徑可絕對可相對(相對於 mori-tauri 的 cwd)。"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "檔案路徑,絕對或相對。副檔名(.txt / .md 等)會用來 dispatch reader。"
                }
            },
            "required": ["path"]
        })
    }

    fn target_capability(&self) -> ExecutionTarget {
        ExecutionTarget::Local
    }

    fn privacy(&self) -> Privacy {
        Privacy::Cloud
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing path"))?
            .trim()
            .to_string();

        if path_str.is_empty() {
            return Err(anyhow!("path is empty"));
        }

        let path = PathBuf::from(&path_str);
        tracing::info!(path = %path.display(), "read_file_text skill");

        // mori-file-loader 是 sync(純 std::fs::read_to_string,讀本地檔案
        // 不適合丟 tokio 線程池;檔案 IO 量級小,直接 inline call。若未來加
        // .pdf / 大檔解析 reader,可改 spawn_blocking。
        let text = mori_file_loader::read_file_text(&path)
            .map_err(|e| anyhow!("read_file_text failed: {e}"))?;

        let chars = text.chars().count();
        let bytes = text.len();

        Ok(SkillOutput {
            user_message: text.clone(),
            data: Some(serde_json::json!({
                "path": path_str,
                "chars": chars,
                "bytes": bytes,
                "text": text,
            })),
        })
    }
}

#[cfg(test)]
mod tests {
    //! Skill-level integration tests — 拼 SkillCall args,確認 dispatch 路徑活著。
    //! mori-file-loader 自身的 reader 行為 unit tests 不重複(那邊的 crate
    //! `tests/integration.rs` 跟內部 mod tests 已覆蓋)。

    use super::*;
    use crate::context::Context;
    use std::fs;
    use tempfile::TempDir;

    fn empty_context() -> Context {
        Context::default()
    }

    #[tokio::test]
    async fn name_and_description_present() {
        let skill = ReadFileSkill;
        assert_eq!(skill.name(), "read_file_text");
        // description 不空、明確指向讀檔案語義
        let desc = skill.description();
        assert!(!desc.is_empty());
        assert!(desc.contains("檔案") || desc.contains("file"));
    }

    #[tokio::test]
    async fn parameters_schema_requires_path() {
        let skill = ReadFileSkill;
        let schema = skill.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["path"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "path"));
    }

    #[tokio::test]
    async fn read_file_skill_returns_text() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("hello.txt");
        fs::write(&p, "Hello, Mori").unwrap();

        let skill = ReadFileSkill;
        let args = serde_json::json!({ "path": p.to_str().unwrap() });
        let ctx = empty_context();
        let out = skill.execute(args, &ctx).await.expect("read should succeed");

        assert_eq!(out.user_message, "Hello, Mori");
        let data = out.data.expect("data present");
        assert_eq!(data["chars"], 11);
        assert_eq!(data["bytes"], 11);
        assert_eq!(data["text"], "Hello, Mori");
    }

    #[tokio::test]
    async fn read_file_skill_supports_md() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("notes.md");
        fs::write(&p, "# Title\n\nbody").unwrap();

        let skill = ReadFileSkill;
        let args = serde_json::json!({ "path": p.to_str().unwrap() });
        let out = skill.execute(args, &empty_context()).await.expect("read md");
        assert!(out.user_message.contains("Title"));
        assert!(out.user_message.contains("body"));
    }

    #[tokio::test]
    async fn read_file_skill_returns_error_for_missing() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("does_not_exist.txt");

        let skill = ReadFileSkill;
        let args = serde_json::json!({ "path": p.to_str().unwrap() });
        let err = skill
            .execute(args, &empty_context())
            .await
            .expect_err("missing file should error");

        let msg = err.to_string();
        // mori-file-loader 回 NotFound,被 anyhow 包成 "read_file_text failed: file not found: ..."
        assert!(
            msg.contains("not found") || msg.contains("NotFound"),
            "expected NotFound message, got: {msg}"
        );
    }

    #[tokio::test]
    async fn read_file_skill_returns_error_for_unsupported_ext() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("data.pdf");
        fs::write(&p, b"%PDF-fake").unwrap();

        let skill = ReadFileSkill;
        let args = serde_json::json!({ "path": p.to_str().unwrap() });
        let err = skill
            .execute(args, &empty_context())
            .await
            .expect_err("pdf not supported yet");
        assert!(err.to_string().contains("unsupported"));
    }

    #[tokio::test]
    async fn read_file_skill_errors_on_missing_path_arg() {
        let skill = ReadFileSkill;
        let args = serde_json::json!({});
        let err = skill
            .execute(args, &empty_context())
            .await
            .expect_err("missing path arg");
        assert!(err.to_string().contains("missing path"));
    }

    #[tokio::test]
    async fn read_file_skill_errors_on_empty_path() {
        let skill = ReadFileSkill;
        let args = serde_json::json!({ "path": "   " });
        let err = skill
            .execute(args, &empty_context())
            .await
            .expect_err("empty path");
        assert!(err.to_string().contains("empty"));
    }
}
