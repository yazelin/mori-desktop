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

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use super::python_runner::{run_python_script, RunError};
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

/// 掃出來的單個 skill descriptor。
///
/// `scripts_dir` 為 `Some(path)` 表示 skill 目錄底下有 `scripts/`(可執行型),
/// 主 module 看到這條會額外註冊一個 [`AnthropicScriptSkill`]。
/// `None` 表示純 prompt-augmentation skill,只有 SKILL.md。
#[derive(Debug, Clone)]
pub struct DiscoveredSkill {
    pub skill: AnthropicSkill,
    pub scripts_dir: Option<PathBuf>,
}

/// 掃 `skills_dir/<name>/SKILL.md`,parse 成功的全收。
///
/// 失敗的個別 skill(壞 frontmatter / IO error)會 log warning 跳過,不會 crash
/// 整個 discover。`skills_dir` 不存在直接回空 vec(對齊「沒裝 skill 是正常狀態」)。
///
/// **DF-2 升級**:回 [`DiscoveredSkill`] 而非裸 [`AnthropicSkill`],額外帶
/// `scripts_dir`(若 skill 目錄含 `scripts/` 子資料夾)。caller 可以判斷是否該
/// 另外註冊一個 [`AnthropicScriptSkill`] 給 LLM 跑 Python script。
pub fn discover_skills(skills_dir: &Path) -> Vec<DiscoveredSkill> {
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
        // path.is_dir() follows symlinks(對齊 DF-1 install flatten:Anthropic
        // skills 是 symlink → 也算 dir)。
        if !path.is_dir() {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        match load_skill_from_path(&skill_md) {
            Ok(skill) => {
                let scripts_path = path.join("scripts");
                let scripts_dir = if scripts_path.is_dir() {
                    Some(scripts_path)
                } else {
                    None
                };
                tracing::debug!(
                    name = %skill.name,
                    has_scripts = scripts_dir.is_some(),
                    "loaded anthropic skill"
                );
                out.push(DiscoveredSkill {
                    skill,
                    scripts_dir,
                });
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

// ─── AnthropicScriptSkill ──────────────────────────────────────────

/// 把 Anthropic skill 內的 `scripts/` 子資料夾暴露成單一可呼叫 LLM tool。
///
/// # 跟 [`AnthropicPromptSkill`] 並存,不取代
///
/// 一個 Anthropic skill 可能有兩種互補形態:
/// 1. **SKILL.md body**:給 LLM 「讀」的指引(`AnthropicPromptSkill`)
/// 2. **scripts/**:給 LLM 「跑」的 Python 程式(本 struct)
///
/// 兩個都會註冊到 [`SkillRegistry`](super::SkillRegistry)。LLM 在 tool list
/// 同時看到:
/// - `pdf` — Use this skill when working with PDFs.(prompt-augmentation)
/// - `anthropic_script_pdf` — [scripts] Use this skill when working with PDFs.
///   (subprocess execution)
///
/// LLM 流程通常:先 invoke `pdf`(讀指引,知道有哪些 script、用法),再 invoke
/// `anthropic_script_pdf` 加 `script: "merge_pdfs.py", args: [...]`。
///
/// # 為什麼 name 加 `anthropic_script_` prefix?
///
/// 避免跟 prompt skill 同名 collision(都來自同一個 SKILL.md 的 `name` 欄,例
/// `pdf`)。SkillRegistry 名字 unique;同名後註冊會 warn + replace,行為亂跳。
///
/// # 參數 schema
///
/// LLM 拿到一個 generic 的「跑某 script」介面:
/// - `script`(required):script 檔名,例 `extract_text.py`
/// - `args`(optional):CLI argv list
/// - `stdin`(optional):餵給 script stdin 的內容
///
/// SKILL.md body 內會描述 `scripts/` 內各 script 的用法,LLM 從那邊學;這個
/// schema 故意保簡單,不替 individual script 編 wrapper(那是 follow-up)。
///
/// # 安全 / 信任邊界
///
/// 走 `python3` direct subprocess,沒沙箱、沒 venv。user 安裝官方 Anthropic
/// skill 即同意跑(對齊 DF-1 install 邏輯:user 點 install button = 同意)。
/// 自訂 skill 放進 `~/.mori/skills/<name>/scripts/` 也會被 expose;user 對自己
/// 的 ~/.mori 內容負責。
pub struct AnthropicScriptSkill {
    /// 原 SKILL.md 解析結果。保留 `body` / `name` / `description` 給 introspection。
    skill: AnthropicSkill,
    /// `<skill_dir>/scripts/` 絕對路徑。execute 時 join script filename。
    scripts_dir: PathBuf,
    /// `anthropic_script_<name>`,leaked 成 `'static`(對齊 `McpToolSkill` /
    /// `AnthropicPromptSkill` pattern)。
    leaked_name: &'static str,
    /// `[scripts] <description>`,leaked 成 `'static`。
    leaked_description: &'static str,
}

impl AnthropicScriptSkill {
    /// Build wrapper。`leak` name / description 到 `'static`。
    pub fn new(skill: AnthropicSkill, scripts_dir: PathBuf) -> Self {
        let name_string = format!("anthropic_script_{}", skill.name);
        let desc_string = format!("[scripts] {}", skill.description);
        let leaked_name: &'static str = Box::leak(name_string.into_boxed_str());
        let leaked_description: &'static str = Box::leak(desc_string.into_boxed_str());
        Self {
            skill,
            scripts_dir,
            leaked_name,
            leaked_description,
        }
    }

    /// Convenience constructor — 包成 `Arc<dyn Skill>` 給 main.rs 註冊。
    pub fn into_arc_skill(self) -> Arc<dyn Skill> {
        Arc::new(self)
    }

    /// 給 introspection / debug 用:回 scripts dir path。
    pub fn scripts_dir(&self) -> &Path {
        &self.scripts_dir
    }

    /// 給 introspection / debug 用:回原 Anthropic skill name(無 prefix)。
    pub fn skill_name(&self) -> &str {
        &self.skill.name
    }
}

#[async_trait]
impl Skill for AnthropicScriptSkill {
    fn name(&self) -> &'static str {
        self.leaked_name
    }

    fn description(&self) -> &'static str {
        self.leaked_description
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "script": {
                    "type": "string",
                    "description": "Script filename inside the skill's scripts/ directory (e.g. `extract_text.py`)."
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional CLI arguments forwarded to the script (sys.argv[1:])."
                },
                "stdin": {
                    "type": "string",
                    "description": "Optional stdin content piped into the script."
                }
            },
            "required": ["script"],
            "additionalProperties": false
        })
    }

    fn target_capability(&self) -> ExecutionTarget {
        // python3 subprocess 跑在本機 — 不像 prompt-augmentation 那樣
        // device-agnostic。對齊 ShellSkill / ReadFileSkill 等 process-bound skill。
        ExecutionTarget::Local
    }

    fn privacy(&self) -> Privacy {
        // script 輸出會餵回 LLM 做 multi-turn(LLM 接著解 result);user 想完全
        // local 自己選 local provider。
        Privacy::Cloud
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let script = args
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing required argument `script`"))?;

        // 防 path traversal:script 不能含 `..` 或絕對路徑(LLM 不該跳出 scripts_dir)。
        if script.contains("..") || Path::new(script).is_absolute() {
            return Err(anyhow!(
                "invalid script path `{script}` (no `..` or absolute paths allowed)"
            ));
        }

        let cli_args: Vec<String> = args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let stdin = args
            .get("stdin")
            .and_then(|v| v.as_str())
            .map(String::from);

        let script_path = self.scripts_dir.join(script);
        if !script_path.is_file() {
            return Err(anyhow!(
                "script not found: {} (under {})",
                script,
                self.scripts_dir.display()
            ));
        }

        tracing::info!(
            skill = self.leaked_name,
            script = %script,
            args_count = cli_args.len(),
            has_stdin = stdin.is_some(),
            "anthropic script dispatch",
        );

        let output = run_python_script(&script_path, &cli_args, stdin.as_deref())
            .await
            .map_err(|e| match e {
                RunError::PythonMissing => anyhow!(
                    "python3 not in PATH — install Python 3 to run Anthropic script skills"
                ),
                other => anyhow!("script execution failed: {other}"),
            })?;

        // user_message 給 LLM 看的:script stdout 為主。若 exit_code != 0,把
        // stderr 一起塞進去讓 LLM 看到錯誤詳情(否則 LLM 只看到空 stdout 會困惑)。
        let user_message = if output.exit_code == 0 {
            output.stdout.clone()
        } else {
            format!(
                "Script exited with code {}.\n\nstdout:\n{}\n\nstderr:\n{}",
                output.exit_code, output.stdout, output.stderr
            )
        };

        Ok(SkillOutput {
            user_message,
            data: Some(serde_json::json!({
                "skill": self.skill.name,
                "script": script,
                "stdout": output.stdout,
                "stderr": output.stderr,
                "exit_code": output.exit_code,
                "kind": "anthropic_script",
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
        got.sort_by(|x, y| x.skill.name.cmp(&y.skill.name));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].skill.name, "brand-guidelines");
        assert_eq!(got[1].skill.name, "internal-comms");
        // Neither has scripts/ subdir.
        assert!(got[0].scripts_dir.is_none());
        assert!(got[1].scripts_dir.is_none());
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
        assert_eq!(got[0].skill.name, "good");
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
        assert_eq!(got[0].skill.name, "real");
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
        assert_eq!(got[0].skill.name, "ok");
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

    // ─── discover_skills scripts/ enrichment ──────────────────

    #[test]
    fn discover_skills_detects_scripts_subdir() {
        // skill 目錄底下有 `scripts/` 子資料夾 → DiscoveredSkill.scripts_dir = Some
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let pdf = root.join("pdf");
        fs::create_dir(&pdf).unwrap();
        fs::write(
            pdf.join("SKILL.md"),
            "---\nname: pdf\ndescription: PDF tools\n---\n\nbody\n",
        )
        .unwrap();
        fs::create_dir(pdf.join("scripts")).unwrap();
        fs::write(pdf.join("scripts").join("extract.py"), "print('x')\n").unwrap();

        let got = discover_skills(root);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].skill.name, "pdf");
        let sd = got[0].scripts_dir.as_ref().expect("scripts_dir should be Some");
        assert!(sd.ends_with("scripts"));
        assert!(sd.is_dir());
    }

    #[test]
    fn discover_skills_no_scripts_dir_when_only_skill_md() {
        // 純 prompt skill — `scripts_dir` 必須 None。
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let brand = root.join("brand-guidelines");
        fs::create_dir(&brand).unwrap();
        fs::write(
            brand.join("SKILL.md"),
            "---\nname: brand-guidelines\ndescription: X\n---\n\nbody\n",
        )
        .unwrap();

        let got = discover_skills(root);
        assert_eq!(got.len(), 1);
        assert!(got[0].scripts_dir.is_none());
    }

    // ─── AnthropicScriptSkill ─────────────────────────────────

    /// 整環境是否裝 python3。CI 若沒裝就 skip 相關 test。
    fn python3_available() -> bool {
        std::process::Command::new("python3")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn make_script_skill(scripts_dir: PathBuf) -> AnthropicScriptSkill {
        let skill = AnthropicSkill {
            name: "pdf".to_string(),
            description: "Use this skill when working with PDFs.".to_string(),
            body: "# PDF body".to_string(),
            license: None,
        };
        AnthropicScriptSkill::new(skill, scripts_dir)
    }

    #[test]
    fn script_skill_name_has_prefix() {
        let dir = TempDir::new().unwrap();
        let s = make_script_skill(dir.path().to_path_buf());
        assert_eq!(s.name(), "anthropic_script_pdf");
        assert_eq!(s.skill_name(), "pdf");
        assert!(s.description().starts_with("[scripts]"));
    }

    #[test]
    fn script_skill_schema_requires_script() {
        let dir = TempDir::new().unwrap();
        let s = make_script_skill(dir.path().to_path_buf());
        let schema = s.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["script"].is_object());
        assert!(schema["properties"]["args"].is_object());
        assert!(schema["properties"]["stdin"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "script"));
    }

    #[tokio::test]
    async fn script_skill_execute_errors_on_missing_script_arg() {
        let dir = TempDir::new().unwrap();
        let s = make_script_skill(dir.path().to_path_buf());
        let ctx = Context::default();
        let err = s
            .execute(serde_json::json!({}), &ctx)
            .await
            .expect_err("missing script arg should error");
        assert!(err.to_string().contains("script"));
    }

    #[tokio::test]
    async fn script_skill_execute_errors_on_traversal() {
        let dir = TempDir::new().unwrap();
        let s = make_script_skill(dir.path().to_path_buf());
        let ctx = Context::default();
        let err = s
            .execute(serde_json::json!({ "script": "../etc/passwd" }), &ctx)
            .await
            .expect_err("path traversal should be blocked");
        assert!(err.to_string().to_lowercase().contains("invalid"));
    }

    #[tokio::test]
    async fn script_skill_execute_errors_when_script_missing() {
        let dir = TempDir::new().unwrap();
        let s = make_script_skill(dir.path().to_path_buf());
        let ctx = Context::default();
        let err = s
            .execute(serde_json::json!({ "script": "nope.py" }), &ctx)
            .await
            .expect_err("missing script file should error");
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn script_skill_execute_runs_python() {
        if !python3_available() {
            eprintln!("python3 not available; skipping");
            return;
        }
        let dir = TempDir::new().unwrap();
        // 建一個 scripts/ 子目錄 + 一個 Python script
        let scripts_dir = dir.path().join("scripts");
        fs::create_dir(&scripts_dir).unwrap();
        fs::write(
            scripts_dir.join("hello.py"),
            "import sys\nprint('hi', sys.argv[1] if len(sys.argv) > 1 else '')\n",
        )
        .unwrap();

        let s = make_script_skill(scripts_dir);
        let ctx = Context::default();
        let out = s
            .execute(
                serde_json::json!({
                    "script": "hello.py",
                    "args": ["world"],
                }),
                &ctx,
            )
            .await
            .expect("script should run");
        assert!(out.user_message.contains("hi world"));
        let data = out.data.expect("data present");
        assert_eq!(data["skill"], "pdf");
        assert_eq!(data["script"], "hello.py");
        assert_eq!(data["exit_code"], 0);
        assert_eq!(data["kind"], "anthropic_script");
    }

    #[tokio::test]
    async fn script_skill_execute_surfaces_nonzero_exit() {
        if !python3_available() {
            eprintln!("python3 not available; skipping");
            return;
        }
        let dir = TempDir::new().unwrap();
        let scripts_dir = dir.path().join("scripts");
        fs::create_dir(&scripts_dir).unwrap();
        fs::write(
            scripts_dir.join("fail.py"),
            "import sys\nsys.stderr.write('oops\\n')\nsys.exit(3)\n",
        )
        .unwrap();

        let s = make_script_skill(scripts_dir);
        let ctx = Context::default();
        let out = s
            .execute(serde_json::json!({ "script": "fail.py" }), &ctx)
            .await
            .expect("script runtime ok even if it exits non-zero");
        assert!(out.user_message.contains("exited with code 3"));
        assert!(out.user_message.contains("oops"));
        let data = out.data.unwrap();
        assert_eq!(data["exit_code"], 3);
        assert!(data["stderr"].as_str().unwrap().contains("oops"));
    }
}
