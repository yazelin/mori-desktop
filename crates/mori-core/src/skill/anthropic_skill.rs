//! AnthropicSkill — 解析 Anthropic 官方 SKILL.md 格式,把 markdown body 當作
//! **prompt-augmentation** 注入給 LLM。對齊 `https://agentskills.io/specification`。
//!
//! # 為什麼存在(Stream I / D-light)
//!
//! Anthropic 把 skill 內容寫在 `SKILL.md` 裡:
//!
//! ```text
//! ---
//! name: brand-guidelines
//! description: Use this skill when ...
//! license: Optional
//! ---
//!
//! # Skill body (markdown)
//! ...
//! ```
//!
//! 官方 17 個 skill 大致分兩類:
//! - **純 prompt-知識型**(brand-guidelines / internal-comms / doc-coauthoring /
//!   claude-api / skill-creator / mcp-builder 等 ~10 個):body 本身就是給 LLM 看
//!   的指引,**不執行 `scripts/`**,只要把 body 餵回 LLM 就行
//! - **執行型**(pdf / docx / xlsx / pptx / canvas-design / theme-factory /
//!   webapp-testing 7 個):body 引用 `scripts/` 內 Python 程式,需要 Python
//!   runtime + sandbox 才能完整跑(D-full,本 stream 不做)
//!
//! 本 module 處理 **D-light**:純 markdown body 進 prompt。執行型的 skill 載入後
//! body 也會給 LLM 看,但 `scripts/` 不會被執行 — LLM 嘗試呼叫腳本會自然失敗,
//! 等 D-full 開了再補。
//!
//! # 整合路徑
//!
//! 1. `discover_skills(~/.mori/skills/)` 掃所有 `<name>/SKILL.md`
//! 2. 每個 `AnthropicSkill` wrap 成 [`AnthropicPromptSkill`] (impl `Skill` trait)
//! 3. main.rs 註冊到 [`SkillRegistry`],跟 [`ReadFileSkill`] 同一個 pattern
//! 4. LLM 看 tool list 拿到 `brand-guidelines` 等 entry(description 來自
//!    frontmatter,LLM 自己判斷何時 invoke)
//! 5. LLM tool_call `brand-guidelines` → execute() 回 body markdown,LLM 拿到後
//!    繼續對話,依 body instructions 行動

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use super::{ExecutionTarget, Privacy, Skill, SkillOutput};
use crate::context::Context;

// ─── Errors ────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("missing frontmatter (file does not start with '---')")]
    MissingFrontmatter,
    #[error("unclosed frontmatter (no closing '---' line)")]
    UnclosedFrontmatter,
    #[error("frontmatter YAML parse error: {0}")]
    Yaml(String),
    #[error("missing required field `name`")]
    MissingName,
    #[error("missing required field `description`")]
    MissingDescription,
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("io error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parse error in {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
}

// ─── Data ──────────────────────────────────────────────────────────

/// 解析完的 SKILL.md。`body` 是 frontmatter 之後的整段 markdown。
#[derive(Debug, Clone)]
pub struct AnthropicSkill {
    pub name: String,
    pub description: String,
    pub body: String,
    pub license: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    license: Option<String>,
}

// ─── Parsing ───────────────────────────────────────────────────────

/// 把 SKILL.md 內容拆成 frontmatter + body。
///
/// 故意手切 `---` boundary 而不丟整段 YAML(對齊 [`agent_profile::parse_agent_profile`]
/// 的做法),因為 body 是任意 markdown,可能含 `---` horizontal rule;先抓開頭
/// 那對 fence 再丟 YAML 才不會把 body 內的 `---` 誤判成關閉 fence。
pub fn parse_skill(content: &str) -> Result<AnthropicSkill, ParseError> {
    let trimmed = content.trim_start_matches('\u{feff}'); // 去 BOM
    let trimmed = trimmed.trim_start();
    if !trimmed.starts_with("---") {
        return Err(ParseError::MissingFrontmatter);
    }
    let after_open = trimmed[3..].trim_start_matches(['\r', '\n']);
    let close_pos = after_open
        .find("\n---")
        .ok_or(ParseError::UnclosedFrontmatter)?;
    let fm_str = &after_open[..close_pos];
    let body_raw = &after_open[close_pos + 4..];
    let body = body_raw.trim_start_matches(['\r', '\n']).trim().to_string();

    let fm: Frontmatter =
        serde_yml::from_str(fm_str).map_err(|e| ParseError::Yaml(e.to_string()))?;

    let name = fm
        .name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or(ParseError::MissingName)?;
    let description = fm
        .description
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or(ParseError::MissingDescription)?;

    Ok(AnthropicSkill {
        name,
        description,
        body,
        license: fm.license.filter(|s| !s.trim().is_empty()),
    })
}

// ─── File I/O ──────────────────────────────────────────────────────

/// 讀單一 SKILL.md 檔案並 parse。
pub fn load_skill_from_path(path: &Path) -> Result<AnthropicSkill, LoadError> {
    let content = fs::read_to_string(path).map_err(|e| LoadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    parse_skill(&content).map_err(|e| LoadError::Parse {
        path: path.to_path_buf(),
        source: e,
    })
}

/// 掃 `skills_dir/<name>/SKILL.md`,parse 成功的全收。
///
/// 失敗的個別 skill(壞 frontmatter / IO error)會 log warning 跳過,不會 crash
/// 整個 discover。`skills_dir` 不存在直接回空 vec(對齊「沒裝 skill 是正常狀態」)。
pub fn discover_skills(skills_dir: &Path) -> Vec<AnthropicSkill> {
    let entries = match fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(
                dir = %skills_dir.display(),
                error = %e,
                "skills_dir not readable (no anthropic skills loaded)"
            );
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        match load_skill_from_path(&skill_md) {
            Ok(skill) => {
                tracing::debug!(name = %skill.name, "loaded anthropic skill");
                out.push(skill);
            }
            Err(e) => {
                tracing::warn!(
                    path = %skill_md.display(),
                    error = %e,
                    "skipping invalid anthropic skill"
                );
            }
        }
    }
    out
}

/// `~/.mori/skills/` — Anthropic skill 預設位置。
pub fn default_skills_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(|h| PathBuf::from(h).join(".mori").join("skills"))
        .unwrap_or_else(|_| PathBuf::from(".mori/skills"))
}

// ─── Skill trait wrapper ───────────────────────────────────────────

/// 把 [`AnthropicSkill`] wrap 成 [`Skill`] trait 物件,直接塞進 [`SkillRegistry`]。
///
/// **設計選擇**:body 走 [`SkillOutput::user_message`] 回 LLM。在 multi-turn
/// 模式下,user_message 會被當 tool result 餵回 LLM(對齊 [`mod.rs`] 註解),
/// LLM 拿到 body 後**繼續對話**並依 body instructions 行事 — 這就是
/// 「prompt-augmentation」的整個機制。
///
/// `name()` / `description()` 必須回 `&'static str`,但 Anthropic skill 是動態
/// 載入的,所以借 [`Box::leak`] 把 String 升級成 `'static`(對齊 `ShellSkill`
/// 的 pattern)。skill 在 startup discover 一次性 leak,量小、生命週期跟 process
/// 一致,可接受。
pub struct AnthropicPromptSkill {
    leaked_name: &'static str,
    leaked_description: &'static str,
    body: String,
    /// 留著供 introspection / future use(license 顯示在 UI 等)。
    _license: Option<String>,
}

impl AnthropicPromptSkill {
    pub fn new(skill: AnthropicSkill) -> Self {
        let leaked_name: &'static str = Box::leak(skill.name.into_boxed_str());
        let leaked_description: &'static str = Box::leak(skill.description.into_boxed_str());
        Self {
            leaked_name,
            leaked_description,
            body: skill.body,
            _license: skill.license,
        }
    }

    /// 直接給整 list arc 包好的 wrapper,給 main.rs 註冊用方便。
    pub fn into_arc_skill(self) -> Arc<dyn Skill> {
        Arc::new(self)
    }
}

#[async_trait]
impl Skill for AnthropicPromptSkill {
    fn name(&self) -> &'static str {
        self.leaked_name
    }

    fn description(&self) -> &'static str {
        self.leaked_description
    }

    fn parameters_schema(&self) -> Value {
        // Anthropic SKILL.md 不帶 parameters schema — 整個 skill 就是「給 LLM 看
        // body」,不需要參數。回空 object,LLM 知道不用填 args。
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn target_capability(&self) -> ExecutionTarget {
        // Prompt-augmentation 純文字輸出,任何裝置都能跑。
        ExecutionTarget::Anywhere
    }

    fn privacy(&self) -> Privacy {
        // body 會餵回 LLM,所以對齊現有 Cloud 預設(user 想 local-only 自己選 provider)。
        Privacy::Cloud
    }

    async fn execute(&self, _args: Value, _context: &Context) -> Result<SkillOutput> {
        tracing::info!(skill = %self.leaked_name, "anthropic skill invoked (prompt-augmentation)");
        Ok(SkillOutput {
            user_message: self.body.clone(),
            data: Some(serde_json::json!({
                "skill": self.leaked_name,
                "kind": "anthropic_prompt",
            })),
        })
    }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn brand_skill_md() -> &'static str {
        "---\n\
         name: brand-guidelines\n\
         description: Use this skill when writing brand-aligned copy.\n\
         license: MIT\n\
         ---\n\
         \n\
         # Brand Guidelines\n\
         \n\
         Always write in plain language.\n\
         Use Mori voice: calm, direct, poetic.\n"
    }

    // ─── parse_skill ───────────────────────────────────────────

    #[test]
    fn parse_skill_extracts_frontmatter() {
        let s = parse_skill(brand_skill_md()).expect("valid skill should parse");
        assert_eq!(s.name, "brand-guidelines");
        assert_eq!(
            s.description,
            "Use this skill when writing brand-aligned copy."
        );
        assert_eq!(s.license.as_deref(), Some("MIT"));
        assert!(s.body.contains("# Brand Guidelines"));
        assert!(s.body.contains("Mori voice"));
    }

    #[test]
    fn parse_skill_returns_error_for_missing_name() {
        let content = "---\n\
            description: only desc\n\
            ---\n\
            \n\
            body\n";
        let err = parse_skill(content).expect_err("missing name should error");
        assert!(matches!(err, ParseError::MissingName), "got {err:?}");
    }

    #[test]
    fn parse_skill_returns_error_for_missing_description() {
        let content = "---\n\
            name: foo\n\
            ---\n\
            \n\
            body\n";
        let err = parse_skill(content).expect_err("missing description should error");
        assert!(matches!(err, ParseError::MissingDescription), "got {err:?}");
    }

    #[test]
    fn parse_skill_handles_empty_body() {
        let content = "---\n\
            name: noop\n\
            description: A skill with no body.\n\
            ---\n";
        let s = parse_skill(content).expect("empty body should parse");
        assert_eq!(s.name, "noop");
        assert_eq!(s.description, "A skill with no body.");
        assert_eq!(s.body, "");
        assert!(s.license.is_none());
    }

    #[test]
    fn parse_skill_ignores_license_if_absent() {
        let content = "---\n\
            name: nolic\n\
            description: No license here.\n\
            ---\n\
            \n\
            body text\n";
        let s = parse_skill(content).expect("should parse");
        assert!(s.license.is_none());
    }

    #[test]
    fn parse_skill_errors_without_frontmatter() {
        let content = "just markdown body, no frontmatter\n";
        let err = parse_skill(content).expect_err("no frontmatter should error");
        assert!(matches!(err, ParseError::MissingFrontmatter), "got {err:?}");
    }

    #[test]
    fn parse_skill_errors_on_unclosed_frontmatter() {
        let content = "---\n\
            name: foo\n\
            description: bar\n";
        let err = parse_skill(content).expect_err("unclosed should error");
        assert!(matches!(err, ParseError::UnclosedFrontmatter), "got {err:?}");
    }

    #[test]
    fn parse_skill_preserves_body_horizontal_rules() {
        // body 內含 `---` markdown horizontal rule 不能被誤認 frontmatter 結尾。
        // (parse_skill 抓第一個 `\n---` 當 close,後續 body 內的 `---` 都保留。)
        let content = "---\n\
            name: hr-test\n\
            description: Body contains horizontal rules.\n\
            ---\n\
            \n\
            section 1\n\
            \n\
            ---\n\
            \n\
            section 2\n";
        let s = parse_skill(content).expect("should parse");
        assert!(s.body.contains("section 1"));
        assert!(s.body.contains("section 2"));
        assert!(s.body.contains("---"));
    }

    // ─── load_skill_from_path ─────────────────────────────────

    #[test]
    fn load_skill_from_path_reads_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("SKILL.md");
        fs::write(&path, brand_skill_md()).unwrap();
        let s = load_skill_from_path(&path).expect("load ok");
        assert_eq!(s.name, "brand-guidelines");
    }

    #[test]
    fn load_skill_from_path_io_error_for_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nope.md");
        let err = load_skill_from_path(&path).expect_err("missing file");
        assert!(matches!(err, LoadError::Io { .. }), "got {err:?}");
    }

    // ─── discover_skills ──────────────────────────────────────

    #[test]
    fn discover_skills_scans_subdirs() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let a = root.join("brand-guidelines");
        let b = root.join("internal-comms");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        fs::write(
            a.join("SKILL.md"),
            "---\nname: brand-guidelines\ndescription: A\n---\n\nbody-a\n",
        )
        .unwrap();
        fs::write(
            b.join("SKILL.md"),
            "---\nname: internal-comms\ndescription: B\n---\n\nbody-b\n",
        )
        .unwrap();

        let mut got = discover_skills(root);
        got.sort_by(|x, y| x.name.cmp(&y.name));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "brand-guidelines");
        assert_eq!(got[1].name, "internal-comms");
    }

    #[test]
    fn discover_skills_skips_invalid() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let good = root.join("good");
        let bad = root.join("bad");
        fs::create_dir(&good).unwrap();
        fs::create_dir(&bad).unwrap();
        fs::write(
            good.join("SKILL.md"),
            "---\nname: good\ndescription: ok\n---\n\nbody\n",
        )
        .unwrap();
        // bad: 缺 frontmatter
        fs::write(bad.join("SKILL.md"), "no frontmatter here\n").unwrap();

        let got = discover_skills(root);
        assert_eq!(got.len(), 1, "bad skill should be skipped, not crash");
        assert_eq!(got[0].name, "good");
    }

    #[test]
    fn discover_skills_skips_dirs_without_skill_md() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("empty-dir")).unwrap();
        fs::create_dir(root.join("real")).unwrap();
        fs::write(
            root.join("real").join("SKILL.md"),
            "---\nname: real\ndescription: x\n---\n\n",
        )
        .unwrap();
        let got = discover_skills(root);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "real");
    }

    #[test]
    fn discover_skills_returns_empty_for_missing_dir() {
        let dir = TempDir::new().unwrap();
        let nonexistent = dir.path().join("not-there");
        let got = discover_skills(&nonexistent);
        assert!(got.is_empty());
    }

    #[test]
    fn discover_skills_ignores_files_at_top_level() {
        // ~/.mori/skills/README.md 之類的非目錄 entry 要被忽略。
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::write(root.join("README.md"), "not a skill").unwrap();
        let good = root.join("ok");
        fs::create_dir(&good).unwrap();
        fs::write(
            good.join("SKILL.md"),
            "---\nname: ok\ndescription: ok\n---\n\n",
        )
        .unwrap();
        let got = discover_skills(root);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "ok");
    }

    // ─── AnthropicPromptSkill ─────────────────────────────────

    #[tokio::test]
    async fn anthropic_prompt_skill_returns_body_on_execute() {
        let skill = AnthropicSkill {
            name: "test-skill".to_string(),
            description: "test desc".to_string(),
            body: "# Hello\n\nThis is the body.".to_string(),
            license: None,
        };
        let wrapped = AnthropicPromptSkill::new(skill);
        assert_eq!(wrapped.name(), "test-skill");
        assert_eq!(wrapped.description(), "test desc");

        let ctx = Context::default();
        let out = wrapped
            .execute(serde_json::json!({}), &ctx)
            .await
            .expect("execute ok");
        assert_eq!(out.user_message, "# Hello\n\nThis is the body.");
        let data = out.data.expect("data present");
        assert_eq!(data["skill"], "test-skill");
        assert_eq!(data["kind"], "anthropic_prompt");
    }

    #[tokio::test]
    async fn anthropic_prompt_skill_schema_is_empty_object() {
        let skill = AnthropicSkill {
            name: "x".to_string(),
            description: "y".to_string(),
            body: "z".to_string(),
            license: None,
        };
        let wrapped = AnthropicPromptSkill::new(skill);
        let schema = wrapped.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].is_object());
        assert_eq!(schema["properties"].as_object().unwrap().len(), 0);
    }
}
