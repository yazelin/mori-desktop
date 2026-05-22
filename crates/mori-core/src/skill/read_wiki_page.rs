//! ReadWikiPageSkill — LLM 可呼叫的「讀 wiki page」工具(L-mori 記憶之森)。
//!
//! # 為什麼存在
//!
//! Wave 7 「L 記憶之森」(Karpathy LLM Wiki pattern):Mori 在
//! `~/mori-universe/spirits/<name>/wiki/` 維護一份累積的內在百科。System prompt
//! 注入 `wiki/index.md` 讓 LLM 知道**有哪些 page 可拉**;LLM 透過這個 skill 把
//! specific page(`people/yazelin.md`、`projects/mori.md` ...)主動拉進 context。
//!
//! # 設計決策 — 為什麼 inline 讀檔
//!
//! `mori-tauri::wiki_reader` 已經有完整 read logic(graceful skip / path
//! traversal 防護),但 **mori-core 不能 depend mori-tauri**(layer 反過來)。
//! 兩個選擇:
//!
//! 1. 抽 `wiki_reader` 成新 leaf crate(對齊 `mori-file-loader` pattern)
//! 2. **在 skill 內 inline 重寫 ~30 行**(對齊 既有 `slugify` / `remind_me` 內
//!    self-contained helper pattern)
//!
//! 選 (2) — wiki reader logic 不到複雜需新 crate,且兩處邏輯 **必須一致**
//! (path traversal 防護同 rule),重寫成本低 + 程式碼簡單。若 future 加更多
//! wiki 操作(write / list),再抽 crate。
//!
//! # 行為
//!
//! - 吃 `{"page": "people/yazelin.md"}` 參數
//! - resolve 路徑 + 防 path traversal(`..` segment / 絕對路徑 / canonicalize
//!   後跳出 wiki_root → reject)
//! - 讀 .md 內容回傳給 LLM
//! - 失敗(不存在 / traversal / IO):`anyhow!` 往上拋
//!
//! # 平台 / 隱私
//!
//! 讀本機 vault → `ExecutionTarget::Local`。Wiki 內容可能含 user 個資(eg
//! `people/yazelin.md`),但既然要餵回 LLM 做 multi-turn,`Privacy::Cloud`
//! (user 自選 LocalOnly provider 即可)。

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;

use super::{ExecutionTarget, Privacy, Skill, SkillOutput};
use crate::context::Context;

/// LLM 可呼叫的 wiki page reader skill。
pub struct ReadWikiPageSkill {
    vault_root: PathBuf,
    spirit_name: String,
}

impl ReadWikiPageSkill {
    pub fn new(vault_root: PathBuf, spirit_name: String) -> Self {
        Self {
            vault_root,
            spirit_name,
        }
    }
}

#[async_trait]
impl Skill for ReadWikiPageSkill {
    fn name(&self) -> &'static str {
        "read_wiki_page"
    }

    fn description(&self) -> &'static str {
        "讀我的 wiki 內某一 page 進 context。page 是 wiki/ 內的相對路徑(eg \
         'people/yazelin.md'、'concepts/transformer.md')。先看 system prompt \
         開頭的 wiki index 找到 page name 再呼叫,不要亂猜路徑。"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "page": {
                    "type": "string",
                    "description": "wiki/ 內的相對路徑(.md 結尾)。範例:'people/yazelin.md'、'projects/mori.md'、'concepts/karpathy-llm-wiki.md'。"
                }
            },
            "required": ["page"]
        })
    }

    fn target_capability(&self) -> ExecutionTarget {
        ExecutionTarget::Local
    }

    fn privacy(&self) -> Privacy {
        Privacy::Cloud
    }

    async fn execute(&self, args: Value, _ctx: &Context) -> Result<SkillOutput> {
        let page = args
            .get("page")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing page"))?
            .trim()
            .to_string();
        if page.is_empty() {
            return Err(anyhow!("page is empty"));
        }

        tracing::info!(
            spirit = %self.spirit_name,
            page = %page,
            "read_wiki_page skill"
        );

        let content = read_wiki_page_inline(&self.vault_root, &self.spirit_name, &page)
            .map_err(|e| anyhow!("read_wiki_page failed: {e}"))?;

        let chars = content.chars().count();
        let bytes = content.len();

        Ok(SkillOutput {
            user_message: content.clone(),
            data: Some(serde_json::json!({
                "page": page,
                "chars": chars,
                "bytes": bytes,
                "content": content,
            })),
        })
    }
}

// ─── inline reader(對齊 mori-tauri::wiki_reader 的 path traversal 防護) ──

#[derive(Debug, thiserror::Error)]
enum InlineWikiError {
    #[error("path traversal not allowed: {0}")]
    PathTraversal(String),
    #[error("page not found: {0}")]
    NotFound(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// 讀 `<vault_root>/<spirit_name>/wiki/<page_relative>` 全文。
///
/// 安全規則(同 `mori-tauri::wiki_reader::read_wiki_page`):
/// - 空 page / 絕對路徑 / 含 `..` segment / Windows drive prefix → PathTraversal
/// - canonicalize 後檔案必須仍 under wiki_root → 否則 PathTraversal(防 symlink)
/// - 檔案不存在 → NotFound
fn read_wiki_page_inline(
    vault_root: &Path,
    spirit_name: &str,
    page_relative: &str,
) -> Result<String, InlineWikiError> {
    let wiki = vault_root.join(spirit_name).join("wiki");

    if page_relative.is_empty() {
        return Err(InlineWikiError::PathTraversal(page_relative.to_string()));
    }
    let pr_path = PathBuf::from(page_relative);
    if pr_path.is_absolute() {
        return Err(InlineWikiError::PathTraversal(page_relative.to_string()));
    }
    for component in pr_path.components() {
        use std::path::Component;
        match component {
            Component::Normal(_) => {}
            _ => return Err(InlineWikiError::PathTraversal(page_relative.to_string())),
        }
    }

    let target = wiki.join(&pr_path);
    if !target.exists() {
        return Err(InlineWikiError::NotFound(page_relative.to_string()));
    }

    let wiki_canon = wiki.canonicalize().map_err(InlineWikiError::Io)?;
    let target_canon = target.canonicalize().map_err(InlineWikiError::Io)?;
    if !target_canon.starts_with(&wiki_canon) {
        return Err(InlineWikiError::PathTraversal(page_relative.to_string()));
    }

    std::fs::read_to_string(&target_canon).map_err(InlineWikiError::Io)
}

#[cfg(test)]
mod tests {
    //! Skill-level + inline reader unit tests。Wiki reader 主要 path traversal /
    //! NotFound / 成功路徑在 mori-tauri::wiki_reader 也有對應 test,這邊複測
    //! 一次因為 logic 是 duplicated(intentional, see module doc)。

    use super::*;
    use crate::context::Context;
    use std::fs;
    use tempfile::TempDir;

    fn empty_context() -> Context {
        Context::default()
    }

    /// 建一個 fake vault `<tmp>/<spirit>/wiki/`,return (tmpdir, vault_root, wiki_dir)。
    fn make_vault(spirit: &str) -> (TempDir, PathBuf, PathBuf) {
        let dir = TempDir::new().unwrap();
        let vault_root = dir.path().to_path_buf();
        let wiki = vault_root.join(spirit).join("wiki");
        fs::create_dir_all(&wiki).unwrap();
        (dir, vault_root, wiki)
    }

    #[tokio::test]
    async fn read_wiki_page_skill_name_and_description() {
        let skill = ReadWikiPageSkill::new(PathBuf::from("/tmp"), "mori".to_string());
        assert_eq!(skill.name(), "read_wiki_page");
        let desc = skill.description();
        assert!(!desc.is_empty());
        // description 必須提及 wiki / page 才能讓 LLM 知道何時叫
        assert!(desc.contains("wiki"));
        assert!(desc.contains("page"));
    }

    #[tokio::test]
    async fn read_wiki_page_skill_parameters_schema_requires_page() {
        let skill = ReadWikiPageSkill::new(PathBuf::from("/tmp"), "mori".to_string());
        let schema = skill.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["page"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "page"));
    }

    #[tokio::test]
    async fn read_wiki_page_skill_returns_content() {
        let (_tmp, vault_root, wiki) = make_vault("mori");
        fs::create_dir_all(wiki.join("people")).unwrap();
        fs::write(
            wiki.join("people").join("yazelin.md"),
            "# Yazelin\n\nMori 的 主要 user,住台北。\n",
        )
        .unwrap();

        let skill = ReadWikiPageSkill::new(vault_root, "mori".to_string());
        let args = serde_json::json!({ "page": "people/yazelin.md" });
        let out = skill
            .execute(args, &empty_context())
            .await
            .expect("read should succeed");

        assert!(out.user_message.contains("Yazelin"));
        assert!(out.user_message.contains("主要 user"));
        let data = out.data.expect("data present");
        assert_eq!(data["page"], "people/yazelin.md");
        // chars > bytes for CJK content(UTF-8 3 bytes per CJK char)
        assert!(data["chars"].as_u64().unwrap() > 0);
        assert!(data["content"].as_str().unwrap().contains("Yazelin"));
    }

    #[tokio::test]
    async fn read_wiki_page_skill_returns_error_for_missing_page_arg() {
        let skill = ReadWikiPageSkill::new(PathBuf::from("/tmp"), "mori".to_string());
        let args = serde_json::json!({});
        let err = skill
            .execute(args, &empty_context())
            .await
            .expect_err("missing page arg should err");
        assert!(err.to_string().contains("missing page"));
    }

    #[tokio::test]
    async fn read_wiki_page_skill_errors_on_empty_page() {
        let skill = ReadWikiPageSkill::new(PathBuf::from("/tmp"), "mori".to_string());
        let args = serde_json::json!({ "page": "   " });
        let err = skill
            .execute(args, &empty_context())
            .await
            .expect_err("empty page should err");
        assert!(err.to_string().contains("empty"));
    }

    #[tokio::test]
    async fn read_wiki_page_skill_rejects_path_traversal() {
        let (_tmp, vault_root, wiki) = make_vault("mori");
        // 造 secret file 在 vault_root 外
        fs::write(vault_root.parent().unwrap().join("secret.txt"), "TOP SECRET\n")
            .unwrap();
        fs::write(wiki.join("decoy.md"), "decoy\n").unwrap();

        let skill = ReadWikiPageSkill::new(vault_root.clone(), "mori".to_string());

        // `..` traversal
        let args = serde_json::json!({ "page": "../../secret.txt" });
        let err = skill
            .execute(args, &empty_context())
            .await
            .expect_err("traversal should err");
        assert!(
            err.to_string().contains("traversal"),
            "expected traversal err, got: {err}"
        );

        // 絕對路徑
        let args = serde_json::json!({ "page": "/etc/passwd" });
        let err = skill
            .execute(args, &empty_context())
            .await
            .expect_err("absolute path should err");
        assert!(err.to_string().contains("traversal"));
    }

    #[tokio::test]
    async fn read_wiki_page_skill_returns_not_found_for_missing_page() {
        let (_tmp, vault_root, _wiki) = make_vault("mori");
        let skill = ReadWikiPageSkill::new(vault_root, "mori".to_string());
        let args = serde_json::json!({ "page": "people/nonexistent.md" });
        let err = skill
            .execute(args, &empty_context())
            .await
            .expect_err("missing page should err");
        assert!(
            err.to_string().contains("not found") || err.to_string().contains("NotFound"),
            "expected NotFound, got: {err}"
        );
    }

    #[test]
    fn inline_reader_handles_nested_pages() {
        let (_tmp, vault_root, wiki) = make_vault("mori");
        fs::create_dir_all(wiki.join("concepts").join("ml")).unwrap();
        fs::write(
            wiki.join("concepts").join("ml").join("transformer.md"),
            "Attention is all you need.\n",
        )
        .unwrap();

        let got =
            read_wiki_page_inline(&vault_root, "mori", "concepts/ml/transformer.md")
                .expect("should read nested page");
        assert!(got.contains("Attention"));
    }
}
