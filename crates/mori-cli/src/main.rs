//! `mori` CLI — proxy 給外部 AI agent 透過 Bash tool 呼叫 Mori skills。
//!
//! 設計目標:
//! - 跑得起來不依賴 Mori 主程式以外的東西(只需 ~/.mori/runtime.json)
//! - 每個 skill 一個 typed subcommand,arguments 用 clap derive
//! - stdout 輸出純文字(skill 的 user_message) — claude/codex/gemini Bash
//!   tool 直接讀
//! - 錯誤訊息走 stderr,exit code != 0 — 讓 agent 知道 skill 失敗
//!
//! Usage(LLM 視角):
//! ```text
//! mori skill list
//! mori skill translate --text "你好" --target en
//! mori skill polish --text "..." --tone formal
//! ```

use anyhow::{anyhow, Context as _, Result};
use clap::{Parser, Subcommand};
use mori_core::runtime::RuntimeInfo;
use serde_json::{json, Value};

#[derive(Parser)]
#[command(
    name = "mori",
    about = "Mori CLI proxy — let an AI agent (claude/codex/gemini) dispatch Mori skills via Bash tool.",
    version
)]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 跟 Mori skill 互動(translate / polish / etc.)
    Skill {
        #[command(subcommand)]
        sub: SkillCmd,
    },
    /// 顯示 ~/.mori/runtime.json 內容(debug 用)
    Status,
}

#[derive(Subcommand)]
enum SkillCmd {
    /// 列出可用的 skills(name + 一行描述)。LLM 想知道有什麼 skill 時跑這個。
    List,

    /// 翻譯文字。
    Translate {
        /// 要翻譯的原文
        #[arg(short, long)]
        text: String,
        /// 目標語言(zh-TW / zh-CN / en / ja / ko ...)。沒指定預設 zh-TW。
        #[arg(short = 'l', long, default_value = "zh-TW")]
        target: String,
    },

    /// 潤飾 / 改寫文字成指定 tone。
    Polish {
        /// 要潤飾的原文
        #[arg(short, long)]
        text: String,
        /// 語氣 — formal | casual | concise | friendly | neutral
        #[arg(long, default_value = "neutral")]
        tone: String,
    },

    /// 摘要文字成指定格式。
    Summarize {
        /// 要摘要的原文
        #[arg(short, long)]
        text: String,
        /// 風格 — bullet | paragraph | tldr
        #[arg(long, default_value = "bullet")]
        style: String,
    },

    /// 草擬新文字(email / message / essay / social_post)。
    Compose {
        /// 種類 — email | message | essay | social_post
        #[arg(long)]
        kind: String,
        /// 要寫什麼的指示
        #[arg(short, long)]
        prompt: String,
        /// 收件對象 / 場合(可選)
        #[arg(short, long)]
        audience: Option<String>,
    },
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Command::Status => cmd_status(),
        Command::Skill { sub } => match sub {
            SkillCmd::List => cmd_list(),
            SkillCmd::Translate { text, target } => post_skill(
                "translate",
                json!({ "source_text": text, "target_lang": target }),
            ),
            SkillCmd::Polish { text, tone } => {
                post_skill("polish", json!({ "source_text": text, "tone": tone }))
            }
            SkillCmd::Summarize { text, style } => post_skill(
                "summarize",
                json!({ "source_text": text, "style": style }),
            ),
            SkillCmd::Compose {
                kind,
                prompt,
                audience,
            } => {
                let mut body = json!({ "kind": kind, "prompt": prompt });
                if let Some(a) = audience {
                    body["audience"] = json!(a);
                }
                post_skill("compose", body)
            }
        },
    }
}

fn cmd_status() -> Result<()> {
    let info = RuntimeInfo::read_from_default()?;
    println!("port:           {}", info.port);
    println!("auth_token:     {}…(隱藏)", &info.auth_token[..8.min(info.auth_token.len())]);
    println!("pid:            {}", info.pid);
    println!("started_at:     {}", info.started_at_epoch);
    println!("base_url:       {}", info.base_url());
    Ok(())
}

fn cmd_list() -> Result<()> {
    let info = RuntimeInfo::read_from_default()?;
    let url = format!("{}/skill/list", info.base_url());
    let resp = reqwest::blocking::Client::new()
        .get(&url)
        .header("authorization", info.bearer())
        .send()
        .context("GET /skill/list — Mori 主程式有在跑嗎?")?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}: {}", resp.status(), resp.text().unwrap_or_default()));
    }
    let v: Value = resp.json().context("parse list response")?;
    let skills = v
        .get("skills")
        .and_then(|s| s.as_array())
        .ok_or_else(|| anyhow!("no skills field in response"))?;
    for s in skills {
        let name = s.get("name").and_then(|n| n.as_str()).unwrap_or("?");
        let desc = s
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("(no description)");
        let first_line = desc.lines().next().unwrap_or("");
        println!("{:<12} {}", name, first_line);
    }
    Ok(())
}

fn post_skill(name: &str, body: Value) -> Result<()> {
    let info = RuntimeInfo::read_from_default()?;
    let url = format!("{}/skill/{}", info.base_url(), name);
    let resp = reqwest::blocking::Client::new()
        .post(&url)
        .header("authorization", info.bearer())
        .json(&body)
        .send()
        .with_context(|| format!("POST /skill/{} — Mori 主程式有在跑嗎?", name))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().unwrap_or_default();
        return Err(anyhow!("HTTP {}: {}", status, txt));
    }
    let txt = resp.text().context("read response body")?;
    print!("{}", txt);
    if !txt.ends_with('\n') {
        println!();
    }
    Ok(())
}
