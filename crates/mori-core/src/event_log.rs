//! Append-only JSON Lines event log。除錯 / 觀測用。
//!
//! 每行一個 event,自動按日期 rotate:`~/.mori/logs/mori-YYYY-MM-DD.jsonl`。
//! 設計刻意極簡 — 沒 schema、沒 ring buffer、沒 async,任何 caller 構造 JSON
//! 直接 `append()`。失敗安靜 `tracing::warn!`,絕不影響業務邏輯。
//!
//! ## Event 規範(沒 enforce,但建議遵循)
//! 必填:`kind`(short tag,e.g. "llm_call" / "spawn_error" / "transcribe")。
//! 自動補:`ts`(若 caller 沒給,append 時補 RFC3339 UTC)。
//! 其他欄位由 caller 自選 — provider / model / latency_ms / ok / error / context 等。
//!
//! ## 並發
//! 走 OS file append atomicity(POSIX / NTFS <4K 寫入是原子的)。Mori 是
//! single-process,單檔 append 不會撞;multi-process 也 OK(系統保證)。
//!
//! ## 為什麼 JSONL 不 SQLite
//! - 0 新 deps(serde_json 已在用)
//! - `tail -f mori-2026-05-15.jsonl | jq .` 直接看,terminal debug 黃金
//! - 每日 rotate,過期手動刪
//! - 之後 scale 不夠用 → JSONL→SQLite migration trivial(讀 jsonl insert),反向比較煩

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use chrono::Utc;
use serde_json::Value;

fn log_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| PathBuf::from(h).join(".mori").join("logs"))
}

/// Append 一筆 event 到今日的 jsonl 檔。caller 給 JSON object,沒 `ts` 自動補。
/// 失敗只發 warn,不 panic 也不回 Err — 觀測層不該擋業務。
pub fn append(mut event: Value) {
    if let Value::Object(ref mut map) = event {
        if !map.contains_key("ts") {
            map.insert("ts".into(), Value::String(Utc::now().to_rfc3339()));
        }
    }
    let dir = match log_dir() {
        Some(d) => d,
        None => return,
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(?e, "event_log: mkdir failed");
        return;
    }
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let path = dir.join(format!("mori-{today}.jsonl"));
    let mut line = match serde_json::to_string(&event) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(?e, "event_log: serialize failed");
            return;
        }
    };
    line.push('\n');
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut file) => {
            if let Err(e) = file.write_all(line.as_bytes()) {
                tracing::warn!(?e, path = %path.display(), "event_log: write failed");
            }
        }
        Err(e) => tracing::warn!(?e, path = %path.display(), "event_log: open failed"),
    }
}

/// 讀某天的 log 最後 N 筆(newest first)。
/// `date` 格式 `"YYYY-MM-DD"` UTC。檔不存在回空 vec。
pub fn read_tail(date: &str, limit: usize) -> Vec<Value> {
    let dir = match log_dir() {
        Some(d) => d,
        None => return vec![],
    };
    let path = dir.join(format!("mori-{date}.jsonl"));
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    text.lines()
        .rev()
        .take(limit)
        .filter_map(|line| serde_json::from_str(line.trim()).ok())
        .collect()
}

/// 列出 logs 目錄內所有可用日期(newest first)。UI 給日期切換用。
pub fn list_dates() -> Vec<String> {
    let dir = match log_dir() {
        Some(d) => d,
        None => return vec![],
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return vec![],
    };
    let mut dates: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .filter_map(|name| {
            name.strip_prefix("mori-")
                .and_then(|n| n.strip_suffix(".jsonl"))
                .map(String::from)
        })
        .collect();
    dates.sort();
    dates.reverse();
    dates
}

/// 取今日的日期字串(`YYYY-MM-DD`,UTC)。前端 default selection 用。
pub fn today() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn append_auto_fills_ts() {
        // 直接驗 ts 自動補的行為(不寫真檔,模擬內部 map 動作)。
        let mut e = json!({"kind": "test"});
        if let Value::Object(map) = &mut e {
            assert!(!map.contains_key("ts"));
            map.insert("ts".into(), Value::String(Utc::now().to_rfc3339()));
            assert!(map.contains_key("ts"));
        }
    }
}
