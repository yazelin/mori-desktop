//! Phase 5H: 從 Agent profile 動態載入的 shell skill 實作。
//!
//! Profile frontmatter 內定義的 `shell_skills:` 陣列在 profile 載入時轉成
//! 一群 `ShellSkill` 物件註冊到 SkillRegistry。LLM 看到他們就跟看到其他 skill 一樣。
//!
//! ## 安全
//! 1. `command` 是 array，第一個是 binary，其餘是 args。`Command::arg()` 直接交給
//!    OS exec，**不走 shell parsing**，所以 `;` `&&` `|` `$()` 都被當字面字串。
//! 2. 參數值用 `{{name}}` 在每個 arg 元素內做字串替換，仍是字面字串。
//! 3. profile 由使用者寫（信任來源），LLM 沒能力動 `command`。

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use mori_core::agent_profile::{ParamDef, ShellSkillDef};
use mori_core::context::Context;
use mori_core::skill::{Skill, SkillOutput};
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;

/// 限制 shell skill 的 stdout 給 LLM 看的大小，避免巨量輸出灌爆 context。
const STDOUT_MAX_BYTES: usize = 4 * 1024;
/// stderr 也截斷，但更小（diagnostic 用）
const STDERR_MAX_BYTES: usize = 1 * 1024;

pub struct ShellSkill {
    def: ShellSkillDef,
    /// 經過 `Box::leak` 的 name 字串，給 `&'static str` 介面用。
    /// 每次 profile 切換 leak 一次（量小、生命週期一致，可接受）。
    leaked_name: &'static str,
    leaked_description: &'static str,
}

impl ShellSkill {
    pub fn new(def: ShellSkillDef) -> Self {
        let leaked_name: &'static str = Box::leak(def.name.clone().into_boxed_str());
        let leaked_description: &'static str =
            Box::leak(def.description.clone().into_boxed_str());
        Self {
            def,
            leaked_name,
            leaked_description,
        }
    }
}

#[async_trait]
impl Skill for ShellSkill {
    fn name(&self) -> &'static str {
        self.leaked_name
    }

    fn description(&self) -> &'static str {
        self.leaked_description
    }

    fn parameters_schema(&self) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required: Vec<String> = vec![];
        for (name, param) in &self.def.parameters {
            let mut p = serde_json::Map::new();
            p.insert("type".into(), json!(param.kind));
            if let Some(desc) = &param.description {
                p.insert("description".into(), json!(desc));
            }
            if let Some(default) = &param.default {
                p.insert("default".into(), json!(default));
            }
            properties.insert(name.clone(), json!(p));
            if param.required {
                required.push(name.clone());
            }
        }
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
        })
    }

    async fn execute(&self, args: Value, _ctx: &Context) -> Result<SkillOutput> {
        // ── 收集 + 驗證參數值 ─────────────────────────────────────
        let arg_map = collect_args(&self.def.parameters, &args)?;

        // ── 把 {{name}} 替換到 command 每個元素中 ───────────────
        let resolved_command: Vec<String> = self
            .def
            .command
            .iter()
            .map(|item| substitute(item, &arg_map))
            .collect();

        if resolved_command.is_empty() {
            return Err(anyhow!("command array is empty"));
        }
        let (binary, exec_args) = resolved_command.split_first().unwrap();
        let binary_expanded = expand_tilde(binary);

        tracing::info!(
            skill = self.def.name,
            binary = %binary_expanded,
            args = ?exec_args,
            "executing shell_skill"
        );

        // ── 執行 ──────────────────────────────────────────────────
        let mut cmd = TokioCommand::new(&binary_expanded);
        for a in exec_args {
            cmd.arg(a);
        }
        if let Some(wd) = &self.def.working_dir {
            cmd.current_dir(expand_tilde(wd));
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        // brand-3 follow-up: 跟 bash_cli_agent 一致設 kill_on_drop,讓 Ctrl+Alt+Esc
        // abort pipeline task 後,子程序(以及它 spawn 的 xclip / ydotool 等)連帶
        // SIGKILL — 避免殘留 process 卡 X11 clipboard ownership / ydotool 半發 key
        // sequence,造成下次 shell_skill 異常(如 ZeroType agent 永久叫不動)。
        cmd.kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn shell_skill '{}' binary {}", self.def.name, binary_expanded))?;

        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();

        // Read output with limits + apply timeout
        let timeout = Duration::from_secs(self.def.timeout_secs);
        let wait_with_io = async {
            let mut stdout_buf = vec![];
            let mut stderr_buf = vec![];
            if let Some(h) = stdout_handle {
                h.take(STDOUT_MAX_BYTES as u64 + 256)
                    .read_to_end(&mut stdout_buf)
                    .await
                    .ok();
            }
            if let Some(h) = stderr_handle {
                h.take(STDERR_MAX_BYTES as u64 + 256)
                    .read_to_end(&mut stderr_buf)
                    .await
                    .ok();
            }
            let status = child.wait().await.context("wait shell_skill child")?;
            anyhow::Ok((status, stdout_buf, stderr_buf))
        };

        let result = tokio::time::timeout(timeout, wait_with_io).await;
        let (status, stdout_bytes, stderr_bytes) = match result {
            Ok(Ok(triple)) => triple,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                tracing::warn!(skill = self.def.name, timeout_secs = self.def.timeout_secs, "shell_skill timed out");
                return Ok(SkillOutput {
                    user_message: format!(
                        "Shell skill '{}' timed out after {}s",
                        self.def.name, self.def.timeout_secs
                    ),
                    data: Some(json!({ "error": "timeout", "timeout_secs": self.def.timeout_secs })),
                });
            }
        };

        // ── 整理 output ─────────────────────────────────────────
        let stdout = truncate_utf8(stdout_bytes, STDOUT_MAX_BYTES);
        let stderr = truncate_utf8(stderr_bytes, STDERR_MAX_BYTES);
        let exit_code = status.code();

        let user_message = if let Some(template) = &self.def.success_message {
            template
                .replace("{{stdout}}", &stdout)
                .replace("{{name}}", &self.def.name)
        } else if status.success() {
            format!("已執行 {}", self.def.name)
        } else {
            format!(
                "Shell skill '{}' 失敗（exit code {:?}）：{}",
                self.def.name, exit_code, stderr
            )
        };

        Ok(SkillOutput {
            user_message,
            data: Some(json!({
                "skill": self.def.name,
                "exit_code": exit_code,
                "ok": status.success(),
                "stdout": stdout,
                "stderr": stderr,
            })),
        })
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────

/// 從 LLM 給的 `args` JSON object 提出每個 parameter 的字串值。
/// `required: true` 但沒給 → Err。可選有 default 用 default，沒 default 用空字串。
fn collect_args(
    params: &HashMap<String, ParamDef>,
    args: &Value,
) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    for (name, def) in params {
        let value = args.get(name);
        let resolved = match value {
            Some(v) => v
                .as_str()
                .map(str::to_string)
                .or_else(|| Some(v.to_string()))
                .unwrap_or_default(),
            None => match &def.default {
                Some(d) => d.clone(),
                None => {
                    if def.required {
                        return Err(anyhow!("required parameter '{}' missing", name));
                    }
                    String::new()
                }
            },
        };
        out.insert(name.clone(), resolved);
    }
    Ok(out)
}

/// 在字串中替換 `{{name}}` 為 args 對應的值。多個 placeholder 都會替換。
fn substitute(template: &str, args: &HashMap<String, String>) -> String {
    let mut out = template.to_string();
    for (k, v) in args {
        let placeholder = format!("{{{{{k}}}}}"); // {{key}}
        out = out.replace(&placeholder, v);
    }
    out
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    path.to_string()
}

fn truncate_utf8(bytes: Vec<u8>, max: usize) -> String {
    let s = String::from_utf8_lossy(&bytes);
    if s.len() <= max {
        return s.into_owned();
    }
    let truncated: String = s.chars().take_while(|c| c.len_utf8() <= max).collect();
    let mut head = String::new();
    let mut bytes_so_far = 0;
    for c in s.chars() {
        let cb = c.len_utf8();
        if bytes_so_far + cb > max {
            break;
        }
        head.push(c);
        bytes_so_far += cb;
    }
    let _ = truncated;
    format!("{head}\n... [output truncated at {max} bytes]")
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitute_basic() {
        let mut args = HashMap::new();
        args.insert("host".into(), "dev01".into());
        args.insert("port".into(), "22".into());
        assert_eq!(substitute("ssh {{host}}:{{port}}", &args), "ssh dev01:22");
    }

    #[test]
    fn substitute_no_placeholders() {
        let args = HashMap::new();
        assert_eq!(substitute("ls -la", &args), "ls -la");
    }

    #[test]
    fn substitute_special_chars_stay_literal() {
        // LLM 想注入 shell metachar，但因為 substitute 後仍是字面字串，
        // 之後 Command::arg() 直接 exec 不會被 shell 解析。
        let mut args = HashMap::new();
        args.insert("host".into(), "evil; rm -rf ~".into());
        let r = substitute("ssh {{host}}", &args);
        assert_eq!(r, "ssh evil; rm -rf ~");
        // 注意：這個字面字串會被當成 "ssh" binary 的一個 argv 元素，
        // ssh 看到的 hostname 就是 "evil; rm -rf ~"（會解析失敗），不會執行 rm。
    }

    #[test]
    fn collect_args_required_missing_fails() {
        let mut params = HashMap::new();
        params.insert(
            "host".into(),
            ParamDef {
                kind: "string".into(),
                required: true,
                description: None,
                default: None,
            },
        );
        let args = json!({});
        assert!(collect_args(&params, &args).is_err());
    }

    #[test]
    fn collect_args_uses_default() {
        let mut params = HashMap::new();
        params.insert(
            "ns".into(),
            ParamDef {
                kind: "string".into(),
                required: false,
                description: None,
                default: Some("default".into()),
            },
        );
        let args = json!({});
        let r = collect_args(&params, &args).unwrap();
        assert_eq!(r.get("ns"), Some(&"default".to_string()));
    }

    #[test]
    fn expand_tilde_basic() {
        std::env::set_var("HOME", "/home/test");
        assert_eq!(expand_tilde("~/bin/foo"), "/home/test/bin/foo");
        assert_eq!(expand_tilde("/abs/path"), "/abs/path");
    }
}
