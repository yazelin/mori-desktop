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
        /// 語氣 — formal | casual | concise | detailed | auto
        #[arg(long, default_value = "auto")]
        tone: String,
    },

    /// 摘要文字成指定格式。
    Summarize {
        /// 要摘要的原文
        #[arg(short, long)]
        text: String,
        /// 風格 — bullet_points | one_paragraph | tldr
        #[arg(long, default_value = "bullet_points")]
        style: String,
    },

    /// 草擬新文字(email / message / essay / social_post / other)。
    Compose {
        /// 種類 — email | message | essay | social_post | other
        #[arg(long)]
        kind: String,
        /// 要寫什麼的主題 / 大綱
        #[arg(short, long)]
        topic: String,
        /// 收件對象 / 場合(可選)
        #[arg(short, long)]
        audience: Option<String>,
        /// 長度提示 — short | medium | long(可選,預設 medium)
        #[arg(long)]
        length_hint: Option<String>,
    },

    /// 把一件事存進 Mori 的長期記憶。
    Remember {
        /// 簡短標題(3-15 字)
        #[arg(short, long)]
        title: String,
        /// 完整內容
        #[arg(short, long)]
        content: String,
        /// 類別 — user_identity | preference | project | reference | other
        #[arg(long, default_value = "other")]
        category: String,
    },

    /// 讀取單筆記憶的完整內容。
    RecallMemory {
        /// Memory id(檔名不含 .md)
        #[arg(long)]
        id: String,
    },

    /// 刪除一筆記憶(destructive)。
    ForgetMemory {
        /// Memory id(檔名不含 .md)
        #[arg(long)]
        id: String,
    },

    /// 更新既有記憶的內容。
    EditMemory {
        /// Memory id(檔名不含 .md)
        #[arg(long)]
        id: String,
        /// 整合後的完整新內容
        #[arg(short, long)]
        content: String,
        /// 更新索引行的短描述(可選,不給則保留舊的)
        #[arg(long)]
        description: Option<String>,
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
                // PolishSkill 用 `text`(沒 source_ 前綴)。tone enum 也不同於 translate。
                post_skill("polish", json!({ "text": text, "tone": tone }))
            }
            SkillCmd::Summarize { text, style } => {
                // SummarizeSkill 也用 `text`。style enum 是 bullet_points / one_paragraph / tldr。
                post_skill("summarize", json!({ "text": text, "style": style }))
            }
            SkillCmd::Compose {
                kind,
                topic,
                audience,
                length_hint,
            } => {
                let mut body = json!({ "kind": kind, "topic": topic });
                if let Some(a) = audience {
                    body["audience"] = json!(a);
                }
                if let Some(l) = length_hint {
                    body["length_hint"] = json!(l);
                }
                post_skill("compose", body)
            }
            SkillCmd::Remember { title, content, category } => {
                post_skill("remember", json!({ "title": title, "content": content, "category": category }))
            }
            SkillCmd::RecallMemory { id } => {
                post_skill("recall_memory", json!({ "id": id }))
            }
            SkillCmd::ForgetMemory { id } => {
                post_skill("forget_memory", json!({ "id": id }))
            }
            SkillCmd::EditMemory { id, content, description } => {
                let mut body = json!({ "id": id, "new_content": content });
                if let Some(d) = description {
                    body["new_description"] = json!(d);
                }
                post_skill("edit_memory", body)
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
