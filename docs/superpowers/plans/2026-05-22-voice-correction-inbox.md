# Voice Correction Inbox + Rating UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** STT 校正字典 `~/.mori/corrections.md` 半自動成長:對話結束後 LLM audit 偵測諧音錯字 → inbox → user 一鍵接受寫字典 + chat panel 評分 👍/👎/✏️ 平行入口。

**Architecture:** mori-core 加 4 個資料層 module(inbox / feedback / corrections writer / audit)+ mori-tauri 加 config + Tauri commands + voice pipeline hook + 新 CorrectionsTab React UI + ChatPanel/RecordingsTab 評分按鈕。

**Tech Stack:** Rust + serde / serde_json + chrono + uuid + 既有 mori-core LLM Provider trait + Tauri 2 + React + TS。

**Spec:** `docs/superpowers/specs/2026-05-22-voice-correction-inbox-design.md`

---

## File Structure

**新檔(7 個)**:

| 路徑 | 責任 |
|---|---|
| `crates/mori-core/src/correction_inbox.rs` | jsonl I/O + InboxEntry + filter + group by suggested |
| `crates/mori-core/src/voice_feedback.rs` | feedback.json I/O + Chinese char-level diff |
| `crates/mori-core/src/corrections_writer.rs` | atomic append corrections.md `## User` 段 |
| `crates/mori-core/src/correction_audit.rs` | LLM audit prompt + call + JSON parse + filter |
| `crates/mori-tauri/src/correction_audit_config.rs` | `~/.mori/config.json` correction_audit 子樹 + Tauri commands |
| `crates/mori-tauri/src/correction_cmd.rs` | inbox / feedback / corrections viewer 的 Tauri commands |
| `src/tabs/CorrectionsTab.tsx` + `corrections-tab.css` | UI tab(inbox + viewer 雙 section) |

**改既有(5 個)**:

| 路徑 | 改動 |
|---|---|
| `crates/mori-core/src/lib.rs` | re-export 新 modules |
| `crates/mori-tauri/src/main.rs` | mod 註冊 + voice pipeline hook spawn audit + invoke_handler 註冊新 commands |
| `src/MainShell.tsx`(或 tabs config 處) | 加 CorrectionsTab 到 tabs |
| `src/tabs/ConfigTab.tsx` | 加「校正」sub-tab(audit enabled toggle) |
| `src/ChatPanel.tsx` + `src/tabs/RecordingsTab.tsx` | voice message 加 👍/👎/✏️ 三按鈕 |

---

## Task 1: `correction_inbox.rs` — jsonl entry CRUD

**Files:**
- Create: `crates/mori-core/src/correction_inbox.rs`
- Modify: `crates/mori-core/src/lib.rs`(re-export)
- Test: 同檔 `mod tests`

- [ ] **Step 1: 加 failing test**

加在新檔 `crates/mori-core/src/correction_inbox.rs`:

```rust
//! 校正候選 inbox — append-only JSONL,路徑 `~/.mori/correction_inbox.jsonl`。
//!
//! Entry 流向:LLM audit / user edit 產生 candidate → append entry status=pending →
//! user UI 點 [接受]/[忽略] → mark_accepted_batch / mark_dismissed → 寫入新 entry
//! 帶 status=accepted/dismissed(append-only,不改舊行)。
//!
//! `is_dismissed(wrong, suggested)` 用來 audit 階段過濾 — 同 pair 已 dismiss 過就跳過。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InboxStatus {
    Pending,
    Accepted,
    Dismissed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InboxSource {
    LlmAudit,
    UserEdit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InboxEntry {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub source_session: String,
    pub source: InboxSource,
    pub wrong: String,
    pub suggested: String,
    pub confidence: f64,
    pub reason: String,
    pub status: InboxStatus,
    pub accepted_at: Option<DateTime<Utc>>,
    pub dismissed_at: Option<DateTime<Utc>>,
}

impl InboxEntry {
    pub fn new_pending(
        source_session: impl Into<String>,
        source: InboxSource,
        wrong: impl Into<String>,
        suggested: impl Into<String>,
        confidence: f64,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            source_session: source_session.into(),
            source,
            wrong: wrong.into(),
            suggested: suggested.into(),
            confidence,
            reason: reason.into(),
            status: InboxStatus::Pending,
            accepted_at: None,
            dismissed_at: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InboxError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Append 一筆 entry 到 jsonl 檔。
pub fn append_entry(path: &Path, entry: &InboxEntry) -> Result<(), InboxError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(entry)?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// 讀整檔,壞行 skip + log warn。
pub fn list_all(path: &Path) -> Result<Vec<InboxEntry>, InboxError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = match line {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(line = i + 1, ?e, "correction_inbox.jsonl: read line failed, skipping");
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<InboxEntry>(&line) {
            Ok(entry) => out.push(entry),
            Err(e) => {
                tracing::warn!(line = i + 1, ?e, "correction_inbox.jsonl: parse failed, skipping");
            }
        }
    }
    Ok(out)
}

/// 只回 status=Pending 的 entries。
pub fn list_pending(path: &Path) -> Result<Vec<InboxEntry>, InboxError> {
    Ok(list_all(path)?
        .into_iter()
        .filter(|e| matches!(e.status, InboxStatus::Pending))
        .collect())
}

/// 對應的 (wrong, suggested) 是不是已被 dismissed 過?(同 pair filter)
pub fn is_dismissed(path: &Path, wrong: &str, suggested: &str) -> Result<bool, InboxError> {
    Ok(list_all(path)?.iter().any(|e| {
        matches!(e.status, InboxStatus::Dismissed)
            && e.wrong == wrong
            && e.suggested == suggested
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_path() -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("correction_inbox.jsonl");
        (dir, path)
    }

    #[test]
    fn append_and_list_pending_round_trip() {
        let (_dir, path) = tmp_path();
        let entry = InboxEntry::new_pending(
            "session-1",
            InboxSource::LlmAudit,
            "英檔",
            "音檔",
            0.85,
            "raw → cleaned 改寫",
        );
        append_entry(&path, &entry).unwrap();
        let pending = list_pending(&path).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].wrong, "英檔");
        assert_eq!(pending[0].suggested, "音檔");
        assert_eq!(pending[0].source, InboxSource::LlmAudit);
        assert_eq!(pending[0].status, InboxStatus::Pending);
    }

    #[test]
    fn is_dismissed_returns_true_after_dismiss_entry_written() {
        let (_dir, path) = tmp_path();
        let mut entry = InboxEntry::new_pending(
            "session-1",
            InboxSource::LlmAudit,
            "馬當",
            "Markdown",
            0.9,
            "test",
        );
        entry.status = InboxStatus::Dismissed;
        entry.dismissed_at = Some(Utc::now());
        append_entry(&path, &entry).unwrap();
        assert!(is_dismissed(&path, "馬當", "Markdown").unwrap());
        assert!(!is_dismissed(&path, "馬當", "Other").unwrap());
        assert!(!is_dismissed(&path, "Different", "Markdown").unwrap());
    }

    #[test]
    fn list_pending_filters_out_accepted_and_dismissed() {
        let (_dir, path) = tmp_path();
        let pending = InboxEntry::new_pending("s1", InboxSource::LlmAudit, "a", "b", 0.5, "");
        let mut accepted = InboxEntry::new_pending("s2", InboxSource::LlmAudit, "c", "d", 0.5, "");
        accepted.status = InboxStatus::Accepted;
        accepted.accepted_at = Some(Utc::now());
        let mut dismissed = InboxEntry::new_pending("s3", InboxSource::LlmAudit, "e", "f", 0.5, "");
        dismissed.status = InboxStatus::Dismissed;
        dismissed.dismissed_at = Some(Utc::now());
        append_entry(&path, &pending).unwrap();
        append_entry(&path, &accepted).unwrap();
        append_entry(&path, &dismissed).unwrap();
        let result = list_pending(&path).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].wrong, "a");
    }

    #[test]
    fn corrupted_line_skipped_with_warning() {
        let (_dir, path) = tmp_path();
        // 寫一筆好的 + 一筆壞 JSON + 一筆好的
        let good1 = InboxEntry::new_pending("s1", InboxSource::LlmAudit, "x", "y", 0.5, "");
        append_entry(&path, &good1).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{{ not json").unwrap();
        let good2 = InboxEntry::new_pending("s2", InboxSource::LlmAudit, "x2", "y2", 0.5, "");
        append_entry(&path, &good2).unwrap();
        let result = list_pending(&path).unwrap();
        assert_eq!(result.len(), 2, "壞行 skip + 兩筆好 entry 都回");
    }

    #[test]
    fn list_pending_returns_empty_when_file_missing() {
        let (_dir, path) = tmp_path();
        // 沒寫過任何東西
        let result = list_pending(&path).unwrap();
        assert!(result.is_empty());
    }
}
```

- [ ] **Step 2: 加 module declaration + dep**

`crates/mori-core/Cargo.toml` 確認有 `uuid = { version = "1", features = ["v4"] }`(若沒,加)。

`crates/mori-core/src/lib.rs` 加:
```rust
pub mod correction_inbox;
```

- [ ] **Step 3: 跑 tests pass**

Run: `cargo test -p mori-core --lib correction_inbox 2>&1 | tail`
Expected: 5 PASS

- [ ] **Step 4: 全 lib tests 還過**

Run: `cargo test -p mori-core --lib`
Expected: PASS(既有 279 + 新 5 = 284)

- [ ] **Step 5: Commit**

```bash
git add crates/mori-core/src/correction_inbox.rs crates/mori-core/src/lib.rs crates/mori-core/Cargo.toml
git commit -m "feat(mori-core): correction_inbox jsonl CRUD + InboxEntry data model"
```

---

## Task 2: Group inbox by suggested(UI 用)

**Files:**
- Modify: `crates/mori-core/src/correction_inbox.rs`(加 group fn)
- Test: 同檔 `mod tests`

- [ ] **Step 1: 加 failing tests**

在 correction_inbox.rs 既有 tests mod 內加:

```rust
#[test]
fn group_pending_by_suggested_merges_same_target() {
    let (_dir, path) = tmp_path();
    // 三筆同 suggested="音檔",兩個不同 wrong
    let e1 = InboxEntry::new_pending("s1", InboxSource::LlmAudit, "英檔", "音檔", 0.8, "r1");
    let e2 = InboxEntry::new_pending("s2", InboxSource::LlmAudit, "英檔", "音檔", 0.8, "r2");
    let e3 = InboxEntry::new_pending("s3", InboxSource::LlmAudit, "雲檔", "音檔", 0.7, "r3");
    // 一筆不同 suggested
    let e4 = InboxEntry::new_pending("s4", InboxSource::LlmAudit, "馬當", "Markdown", 0.9, "r4");

    for e in [&e1, &e2, &e3, &e4] {
        append_entry(&path, e).unwrap();
    }

    let groups = group_pending_by_suggested(&path).unwrap();
    assert_eq!(groups.len(), 2, "兩個 suggested 字 = 兩個 group");

    let g_yin = groups.iter().find(|g| g.suggested == "音檔").unwrap();
    assert_eq!(g_yin.variants.len(), 2, "音檔 group 含兩個不同 wrong (英檔 / 雲檔)");
    let var_ying = g_yin.variants.iter().find(|v| v.wrong == "英檔").unwrap();
    assert_eq!(var_ying.count, 2, "英檔 出現 2 次");
    let var_yun = g_yin.variants.iter().find(|v| v.wrong == "雲檔").unwrap();
    assert_eq!(var_yun.count, 1);

    let g_md = groups.iter().find(|g| g.suggested == "Markdown").unwrap();
    assert_eq!(g_md.variants.len(), 1);
}

#[test]
fn group_user_edit_source_sorts_before_llm_audit() {
    let (_dir, path) = tmp_path();
    let e_audit = InboxEntry::new_pending("s1", InboxSource::LlmAudit, "a", "X", 0.5, "");
    let e_edit = InboxEntry::new_pending("s2", InboxSource::UserEdit, "b", "Y", 0.95, "");
    append_entry(&path, &e_audit).unwrap();
    append_entry(&path, &e_edit).unwrap();
    let groups = group_pending_by_suggested(&path).unwrap();
    // UserEdit suggested=Y 應該排在 LlmAudit suggested=X 之前
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].suggested, "Y");
    assert_eq!(groups[1].suggested, "X");
}
```

- [ ] **Step 2: 跑 test 看 fail**

Run: `cargo test -p mori-core --lib group_pending_by_suggested -- --nocapture`
Expected: FAIL(fn 不存在)

- [ ] **Step 3: 實作 group + 相關 struct**

在 correction_inbox.rs 加(放在 `impl InboxEntry` 之後):

```rust
#[derive(Debug, Clone, Serialize)]
pub struct InboxVariant {
    /// 錯字
    pub wrong: String,
    /// 該錯字出現次數
    pub count: usize,
    /// 對應的 entry id 列表(UI 接受 / 忽略時走這些 id)
    pub entry_ids: Vec<String>,
    /// 最早出現的 source session(UI 顯示「來源 session」用)
    pub earliest_session: String,
    /// 該變體的最高 confidence
    pub max_confidence: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InboxGroup {
    /// 正字(grouping key)
    pub suggested: String,
    /// 變體列表
    pub variants: Vec<InboxVariant>,
    /// 是否有任何 variant 來自 user_edit(UI 排序用 — user_edit 優先)
    pub has_user_edit: bool,
}

/// 把 pending entries 依 suggested grouping。UserEdit 來源的 group 排前面,LlmAudit 之後。
pub fn group_pending_by_suggested(path: &Path) -> Result<Vec<InboxGroup>, InboxError> {
    let entries = list_pending(path)?;
    let mut by_suggested: std::collections::HashMap<String, Vec<InboxEntry>> =
        std::collections::HashMap::new();
    for e in entries {
        by_suggested.entry(e.suggested.clone()).or_default().push(e);
    }

    let mut groups: Vec<InboxGroup> = by_suggested
        .into_iter()
        .map(|(suggested, entries)| {
            let has_user_edit = entries.iter().any(|e| matches!(e.source, InboxSource::UserEdit));
            // 同 group 內再依 wrong 分 variants
            let mut by_wrong: std::collections::HashMap<String, Vec<&InboxEntry>> =
                std::collections::HashMap::new();
            for e in &entries {
                by_wrong.entry(e.wrong.clone()).or_default().push(e);
            }
            let variants: Vec<InboxVariant> = by_wrong
                .into_iter()
                .map(|(wrong, es)| {
                    let earliest_session = es
                        .iter()
                        .min_by_key(|e| e.created_at)
                        .map(|e| e.source_session.clone())
                        .unwrap_or_default();
                    let max_confidence = es
                        .iter()
                        .map(|e| e.confidence)
                        .fold(0.0_f64, f64::max);
                    InboxVariant {
                        wrong,
                        count: es.len(),
                        entry_ids: es.iter().map(|e| e.id.clone()).collect(),
                        earliest_session,
                        max_confidence,
                    }
                })
                .collect();
            InboxGroup {
                suggested,
                variants,
                has_user_edit,
            }
        })
        .collect();

    // 排序:has_user_edit=true 優先,然後依 suggested 字串 stable
    groups.sort_by(|a, b| {
        b.has_user_edit
            .cmp(&a.has_user_edit)
            .then_with(|| a.suggested.cmp(&b.suggested))
    });
    Ok(groups)
}
```

- [ ] **Step 4: 跑 tests pass**

Run: `cargo test -p mori-core --lib group_pending_by_suggested -- --nocapture`
Expected: 2 PASS

- [ ] **Step 5: Commit**

```bash
git add crates/mori-core/src/correction_inbox.rs
git commit -m "feat(mori-core): group_pending_by_suggested for UI display"
```

---

## Task 3: `corrections_writer.rs` — atomic append `## User` 段

**Files:**
- Create: `crates/mori-core/src/corrections_writer.rs`
- Modify: `crates/mori-core/src/lib.rs`(re-export)
- Test: 同檔

- [ ] **Step 1: 加 failing tests**

```rust
//! Append corrections.md `## User` 段。atomic write(.tmp + rename)避免 voice
//! profile 同時 read 撞 partial write。
//!
//! 規則:
//! - `## User` heading 不存在 → 在檔尾建 + `### 用戶自加` subsection + 加新行
//! - `## User` heading 存在 → 找 `### 用戶自加`,沒有就建,然後同 suggested
//!   存在的 line merge wrong variants 進去,否則加新行
//! - Baseline 段(`## Baseline`)嚴格不動
//! - line format:`- 錯字1, 錯字2 -> 正字`(對齊既有 baseline 風格)

use std::fs;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum WriterError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrections.md 結構異常: {0}")]
    StructureError(String),
}

/// Append (wrong_variants -> suggested) 到 corrections.md User 段。
///
/// `wrong_variants`:要寫入的錯字 list(可能多個 variant 一次寫,例 ["英檔", "雲檔"])。
/// `suggested`:正字。
///
/// 若該 suggested 已在 User 段存在 → 合進那行(去重後);否則加新行。
/// 若 `## User` heading 不存在 → 在檔尾建。
pub fn append_correction(
    path: &Path,
    wrong_variants: &[String],
    suggested: &str,
) -> Result<(), WriterError> {
    if wrong_variants.is_empty() {
        return Err(WriterError::StructureError("wrong_variants 不能空".into()));
    }

    let content = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };

    let new_content = merge_into_user_section(&content, wrong_variants, suggested)?;

    // atomic write
    let tmp_path = path.with_extension("md.tmp");
    fs::write(&tmp_path, new_content)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// 純函式版本,給 test 用 / 容易 reason。
pub(crate) fn merge_into_user_section(
    content: &str,
    wrong_variants: &[String],
    suggested: &str,
) -> Result<String, WriterError> {
    // 找 ## User heading 跟 ### 用戶自加 subsection 位置
    let lines: Vec<&str> = content.lines().collect();
    let user_h2_idx = lines.iter().position(|l| l.trim() == "## User");

    if user_h2_idx.is_none() {
        // ## User heading 不存在,在檔尾建整段
        let mut out = content.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str("## User\n\n### 用戶自加\n\n");
        out.push_str(&format_line(wrong_variants, suggested));
        out.push('\n');
        return Ok(out);
    }

    let user_h2_idx = user_h2_idx.unwrap();
    // 找 ## User 之後的 ### 用戶自加 subsection
    let subsection_idx = lines
        .iter()
        .enumerate()
        .skip(user_h2_idx + 1)
        .find(|(_, l)| l.trim() == "### 用戶自加")
        .map(|(i, _)| i);

    if subsection_idx.is_none() {
        // 有 ## User 但沒 ### 用戶自加,在 ## User 之後插入 subsection + entry
        let mut out_lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        // 在 user_h2_idx 之後插入空行 + ### 用戶自加 + 空行 + entry
        out_lines.insert(user_h2_idx + 1, String::new());
        out_lines.insert(user_h2_idx + 2, "### 用戶自加".into());
        out_lines.insert(user_h2_idx + 3, String::new());
        out_lines.insert(user_h2_idx + 4, format_line(wrong_variants, suggested));
        return Ok(join_lines(&out_lines));
    }

    let subsection_idx = subsection_idx.unwrap();
    // 找 subsection 段內是否有同 suggested 的 line
    // subsection 結束:下一個 ## 或 ### heading,或檔尾
    let subsection_end = lines
        .iter()
        .enumerate()
        .skip(subsection_idx + 1)
        .find(|(_, l)| {
            let t = l.trim();
            t.starts_with("## ") || t.starts_with("### ")
        })
        .map(|(i, _)| i)
        .unwrap_or(lines.len());

    let mut out_lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    let target_suffix = format!(" -> {suggested}");
    // 找 subsection_idx..subsection_end 範圍內 "-" 起頭、結尾是 " -> <suggested>" 的 line
    let merge_line_idx = (subsection_idx + 1..subsection_end).find(|&i| {
        let l = out_lines[i].trim();
        l.starts_with("- ") && l.ends_with(&target_suffix)
    });

    if let Some(line_idx) = merge_line_idx {
        // merge:解 existing wrongs,union 新的,format 回寫
        let l = out_lines[line_idx].trim().to_string();
        let inner = l.strip_prefix("- ").unwrap();
        let arrow_pos = inner.rfind(" -> ").ok_or_else(|| {
            WriterError::StructureError(format!("既有 line 找不到 ' -> ': {l}"))
        })?;
        let existing_wrongs_str = &inner[..arrow_pos];
        let mut all_wrongs: Vec<String> = existing_wrongs_str
            .split(", ")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        for w in wrong_variants {
            if !all_wrongs.iter().any(|e| e == w) {
                all_wrongs.push(w.clone());
            }
        }
        out_lines[line_idx] = format_line(&all_wrongs, suggested);
    } else {
        // 沒同 suggested line,在 subsection_end 之前(若 subsection_end 是 lines.len(),
        // 等於 push 到 subsection 末尾)插入新 line。
        // 跳過 trailing empty lines 找最後一個 non-empty line 之後插
        let mut insert_at = subsection_end;
        while insert_at > subsection_idx + 1
            && out_lines[insert_at - 1].trim().is_empty()
        {
            insert_at -= 1;
        }
        out_lines.insert(insert_at, format_line(wrong_variants, suggested));
    }

    Ok(join_lines(&out_lines))
}

fn format_line(wrong_variants: &[String], suggested: &str) -> String {
    format!("- {} -> {}", wrong_variants.join(", "), suggested)
}

fn join_lines(lines: &[String]) -> String {
    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_user_section_when_missing() {
        let content = "# Mori STT 校正字典\n\n## Baseline\n\n- 馬當 -> Markdown\n";
        let result = merge_into_user_section(content, &["英檔".into()], "音檔").unwrap();
        assert!(result.contains("## User"));
        assert!(result.contains("### 用戶自加"));
        assert!(result.contains("- 英檔 -> 音檔"));
        // baseline 不動
        assert!(result.contains("- 馬當 -> Markdown"));
    }

    #[test]
    fn merges_variant_into_existing_line() {
        let content = "## User\n\n### 用戶自加\n\n- 英檔 -> 音檔\n";
        let result = merge_into_user_section(content, &["雲檔".into()], "音檔").unwrap();
        assert!(result.contains("- 英檔, 雲檔 -> 音檔"));
        // 不該變兩行
        assert_eq!(result.matches(" -> 音檔").count(), 1);
    }

    #[test]
    fn adds_new_line_for_different_suggested() {
        let content = "## User\n\n### 用戶自加\n\n- 英檔 -> 音檔\n";
        let result = merge_into_user_section(content, &["馬當".into()], "Markdown").unwrap();
        assert!(result.contains("- 英檔 -> 音檔"));
        assert!(result.contains("- 馬當 -> Markdown"));
    }

    #[test]
    fn dedupes_variants() {
        let content = "## User\n\n### 用戶自加\n\n- 英檔 -> 音檔\n";
        // 「英檔」已存在,加同 variant 不應重複
        let result = merge_into_user_section(content, &["英檔".into(), "雲檔".into()], "音檔").unwrap();
        let line = result
            .lines()
            .find(|l| l.contains(" -> 音檔"))
            .unwrap();
        assert_eq!(line.matches("英檔").count(), 1);
        assert!(line.contains("雲檔"));
    }

    #[test]
    fn appends_subsection_when_user_section_exists_but_subsection_missing() {
        let content = "## User\n\n(空白)\n";
        let result = merge_into_user_section(content, &["英檔".into()], "音檔").unwrap();
        assert!(result.contains("### 用戶自加"));
        assert!(result.contains("- 英檔 -> 音檔"));
    }

    #[test]
    fn baseline_section_untouched() {
        let content = "## Baseline\n\n### 常見對話\n\n- 馬當 -> Markdown\n\n## User\n\n### 用戶自加\n\n- 英檔 -> 音檔\n";
        let result = merge_into_user_section(content, &["雲檔".into()], "音檔").unwrap();
        // baseline 馬當 line 該完整存在,且不該有 baseline 內被改動
        let baseline_line = result.lines().find(|l| l.contains("馬當")).unwrap();
        assert_eq!(baseline_line.trim(), "- 馬當 -> Markdown");
    }

    #[test]
    fn writes_atomically_via_tmp_rename() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("corrections.md");
        std::fs::write(&path, "## Baseline\n\n- 馬當 -> Markdown\n").unwrap();
        append_correction(&path, &["英檔".into()], "音檔").unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("- 英檔 -> 音檔"));
        // .tmp 不該留下
        assert!(!path.with_extension("md.tmp").exists());
    }
}
```

- [ ] **Step 2: lib.rs 加 mod**

```rust
pub mod corrections_writer;
```

- [ ] **Step 3: 跑 tests pass**

Run: `cargo test -p mori-core --lib corrections_writer -- --nocapture`
Expected: 7 PASS

- [ ] **Step 4: 全 lib tests 還過**

Run: `cargo test -p mori-core --lib`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/mori-core/src/corrections_writer.rs crates/mori-core/src/lib.rs
git commit -m "feat(mori-core): corrections_writer — atomic append User section in corrections.md"
```

---

## Task 4: `voice_feedback.rs` — feedback.json + token diff

**Files:**
- Create: `crates/mori-core/src/voice_feedback.rs`
- Modify: `crates/mori-core/src/lib.rs`(re-export)

- [ ] **Step 1: 加 failing tests**

```rust
//! Voice session 評分 — `~/.mori/recordings/<session>/feedback.json`。
//!
//! 三種 rating:
//! - `Good`:純信號 👍
//! - `Bad`:純信號 👎
//! - `Edit`:user 改寫過 transcript(corrected_transcript 必填)
//!
//! diff_words:對 cleaned vs corrected 跑 char-level LCS,回 (wrong, corrected)
//! span pair list — 給 inbox 候選用。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackRating {
    Good,
    Bad,
    Edit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Feedback {
    pub rating: FeedbackRating,
    pub rated_at: DateTime<Utc>,
    pub corrected_transcript: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum FeedbackError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// 寫 feedback.json 到 session 目錄。會建 parent dir。
pub fn write_feedback(session_dir: &Path, feedback: &Feedback) -> Result<(), FeedbackError> {
    std::fs::create_dir_all(session_dir)?;
    let path = session_dir.join("feedback.json");
    let json = serde_json::to_string_pretty(feedback)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// 讀 feedback.json,不存在回 None。
pub fn read_feedback(session_dir: &Path) -> Result<Option<Feedback>, FeedbackError> {
    let path = session_dir.join("feedback.json");
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path)?;
    Ok(Some(serde_json::from_str(&raw)?))
}

/// Char-level LCS-based diff。回 (wrong_span, corrected_span) pair list。
/// 兩 span 完全相同的不回(無變化)。
///
/// 用簡單演算法:用 `similar` crate Myers diff at char granularity,合併 adjacent
/// Delete + Insert 變 (wrong, corrected) pair。
pub fn diff_words(original: &str, corrected: &str) -> Vec<(String, String)> {
    if original == corrected {
        return Vec::new();
    }
    use similar::{ChangeTag, TextDiff};
    let diff = TextDiff::configure().diff_chars(original, corrected);

    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut current_wrong = String::new();
    let mut current_corrected = String::new();

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                // flush 累積的 pair(若有非空)
                if !current_wrong.is_empty() || !current_corrected.is_empty() {
                    if !current_wrong.is_empty() && !current_corrected.is_empty() {
                        pairs.push((
                            current_wrong.clone(),
                            current_corrected.clone(),
                        ));
                    }
                    current_wrong.clear();
                    current_corrected.clear();
                }
            }
            ChangeTag::Delete => {
                current_wrong.push_str(change.value());
            }
            ChangeTag::Insert => {
                current_corrected.push_str(change.value());
            }
        }
    }
    // flush 結尾累積
    if !current_wrong.is_empty() && !current_corrected.is_empty() {
        pairs.push((current_wrong, current_corrected));
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn write_read_round_trip() {
        let dir = TempDir::new().unwrap();
        let fb = Feedback {
            rating: FeedbackRating::Good,
            rated_at: Utc::now(),
            corrected_transcript: None,
            comment: None,
        };
        write_feedback(dir.path(), &fb).unwrap();
        let loaded = read_feedback(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.rating, FeedbackRating::Good);
    }

    #[test]
    fn read_returns_none_when_missing() {
        let dir = TempDir::new().unwrap();
        let result = read_feedback(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn edit_with_corrected_transcript_persists() {
        let dir = TempDir::new().unwrap();
        let fb = Feedback {
            rating: FeedbackRating::Edit,
            rated_at: Utc::now(),
            corrected_transcript: Some("正確版本".into()),
            comment: None,
        };
        write_feedback(dir.path(), &fb).unwrap();
        let loaded = read_feedback(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.rating, FeedbackRating::Edit);
        assert_eq!(loaded.corrected_transcript.as_deref(), Some("正確版本"));
    }

    #[test]
    fn diff_words_identical_returns_empty() {
        let pairs = diff_words("一樣的文字", "一樣的文字");
        assert!(pairs.is_empty());
    }

    #[test]
    fn diff_words_single_swap() {
        // 「英檔」改成「音檔」,前後 context 不變
        let pairs = diff_words("我看英檔內容", "我看音檔內容");
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0], ("英".to_string(), "音".to_string()));
    }

    #[test]
    fn diff_words_multi_changes() {
        let pairs = diff_words("英檔跟馬當", "音檔跟Markdown");
        // 兩處改:英→音 + 馬當→Markdown
        assert_eq!(pairs.len(), 2);
    }
}
```

- [ ] **Step 2: Cargo.toml 加 `similar` dep**

`crates/mori-core/Cargo.toml` 加:
```toml
similar = "2"
```

- [ ] **Step 3: lib.rs 加 mod**

```rust
pub mod voice_feedback;
```

- [ ] **Step 4: 跑 tests pass**

Run: `cargo test -p mori-core --lib voice_feedback -- --nocapture`
Expected: 6 PASS

- [ ] **Step 5: 全 lib tests 還過**

Run: `cargo test -p mori-core --lib`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/mori-core/src/voice_feedback.rs crates/mori-core/src/lib.rs crates/mori-core/Cargo.toml
git commit -m "feat(mori-core): voice_feedback — feedback.json I/O + char-level diff via similar crate"
```

---

## Task 5: `correction_audit.rs` — LLM audit prompt + JSON parse

**Files:**
- Create: `crates/mori-core/src/correction_audit.rs`
- Modify: `crates/mori-core/src/lib.rs`(re-export)

- [ ] **Step 1: 加 failing tests + struct**

```rust
//! LLM-based STT 諧音錯字偵測。對話結束後 spawn 一個 task 跑,結果寫進
//! `correction_inbox.jsonl` 等 user 在 CorrectionsTab 確認。
//!
//! Prompt 對齊 spec §4.2。Provider: groq,model 預設 openai/gpt-oss-120b。

use crate::llm::{ChatMessage, LlmProvider, ToolDefinition};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::sync::Arc;

/// LLM 標出的單筆候選。
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AuditCandidate {
    pub wrong: String,
    pub suggested: String,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    #[serde(default)]
    pub reason: String,
}

fn default_confidence() -> f64 {
    0.7
}

/// 跑 audit。Provider trait 由 caller 給(平常 Tauri 端 inject groq provider)。
///
/// 三條過濾:
/// - LLM 回的 `wrong` 不在 transcript_raw 內 → drop(LLM 幻覺)
/// - wrong / suggested 任一空 → drop
/// - 兩者完全相同 → drop
pub async fn audit(
    provider: Arc<dyn LlmProvider>,
    transcript_raw: &str,
    transcript_cleaned: &str,
    corrections_md: &str,
) -> Result<Vec<AuditCandidate>> {
    let system = build_prompt(transcript_raw, transcript_cleaned, corrections_md);
    let messages = vec![ChatMessage::system(&system)];
    let tools: Vec<ToolDefinition> = Vec::new();
    let response = provider
        .chat(messages, tools)
        .await
        .context("correction audit provider chat")?;
    let content = response.content.unwrap_or_default();
    let candidates = parse_response(&content)?;
    Ok(filter(candidates, transcript_raw))
}

fn build_prompt(transcript_raw: &str, transcript_cleaned: &str, corrections_md: &str) -> String {
    format!(
        r#"你是 STT 諧音錯字偵測器。我會給你三段:
1. transcript_raw — STT 直接輸出(可能有諧音錯字、無標點)
2. transcript_cleaned — LLM 校正後版本(已用 corrections.md 處理過 + LLM 自己加標點 segmentation)
3. corrections.md — 現有校正字典(格式:錯字, 錯字2 -> 正字)

任務:列出 transcript_raw 內**可能是 STT 諧音錯字**的詞 + 建議正字。

判斷依據:
- raw → cleaned 過程被改寫的詞(LLM 校過 = 高機率錯字)
- 跟 corrections.md 內錯字組同音 / 形近(已知類型擴散)
- 語義不合 / 諧音怪 / 不像合理中文用語(新類型)

排除:
- 純標點 / segmentation 差異(LLM 加標點不算錯字)
- 已在 corrections.md 內的 entries(那條已經會用)

回 **純 JSON array**,每筆 {{ wrong, suggested, confidence: 0..1, reason: 一句中文說明 }}。
找不到候選回 []。**不要包 markdown code fence,不要附前後說明,只回 JSON**。

==== transcript_raw ====
{transcript_raw}

==== transcript_cleaned ====
{transcript_cleaned}

==== corrections.md ====
{corrections_md}
"#
    )
}

fn parse_response(content: &str) -> Result<Vec<AuditCandidate>> {
    let trimmed = content.trim();
    // 容忍 LLM 偶爾包 ```json fence
    let json_str = if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            after[..end].trim()
        } else {
            after
        }
    } else if trimmed.starts_with("```") {
        let after = &trimmed[3..];
        if let Some(end) = after.find("```") {
            after[..end].trim()
        } else {
            after
        }
    } else {
        trimmed
    };
    serde_json::from_str(json_str)
        .map_err(|e| anyhow!("audit response JSON parse failed: {e} (content: {content:?})"))
}

fn filter(candidates: Vec<AuditCandidate>, transcript_raw: &str) -> Vec<AuditCandidate> {
    candidates
        .into_iter()
        .filter(|c| {
            !c.wrong.is_empty()
                && !c.suggested.is_empty()
                && c.wrong != c.suggested
                && transcript_raw.contains(&c.wrong)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_json_array() {
        let content = r#"[{"wrong":"英檔","suggested":"音檔","confidence":0.85,"reason":"同音"}]"#;
        let result = parse_response(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].wrong, "英檔");
        assert_eq!(result[0].suggested, "音檔");
        assert!((result[0].confidence - 0.85).abs() < 1e-9);
    }

    #[test]
    fn parse_with_json_fence() {
        let content = r#"```json
[{"wrong":"英檔","suggested":"音檔","confidence":0.85,"reason":"x"}]
```"#;
        let result = parse_response(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].wrong, "英檔");
    }

    #[test]
    fn parse_empty_array_ok() {
        let result = parse_response("[]").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_malformed_returns_err() {
        let result = parse_response("not json");
        assert!(result.is_err());
    }

    #[test]
    fn filter_drops_hallucinated_wrong_not_in_raw() {
        let c1 = AuditCandidate {
            wrong: "英檔".into(),
            suggested: "音檔".into(),
            confidence: 0.9,
            reason: "".into(),
        };
        let c2 = AuditCandidate {
            wrong: "不存在的字".into(),
            suggested: "X".into(),
            confidence: 0.9,
            reason: "".into(),
        };
        let result = filter(vec![c1, c2], "我看英檔的內容");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].wrong, "英檔");
    }

    #[test]
    fn filter_drops_identical_wrong_and_suggested() {
        let c1 = AuditCandidate {
            wrong: "音檔".into(),
            suggested: "音檔".into(),
            confidence: 0.9,
            reason: "".into(),
        };
        let result = filter(vec![c1], "音檔");
        assert!(result.is_empty());
    }

    #[test]
    fn filter_drops_empty_fields() {
        let c1 = AuditCandidate {
            wrong: "".into(),
            suggested: "X".into(),
            confidence: 0.9,
            reason: "".into(),
        };
        let c2 = AuditCandidate {
            wrong: "X".into(),
            suggested: "".into(),
            confidence: 0.9,
            reason: "".into(),
        };
        let result = filter(vec![c1, c2], "X");
        assert!(result.is_empty());
    }
}
```

- [ ] **Step 2: lib.rs 加 mod**

```rust
pub mod correction_audit;
```

- [ ] **Step 3: 跑 tests pass**

Run: `cargo test -p mori-core --lib correction_audit -- --nocapture`
Expected: 7 PASS

- [ ] **Step 4: workspace check**

Run: `cargo check --workspace --all-targets`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/mori-core/src/correction_audit.rs crates/mori-core/src/lib.rs
git commit -m "feat(mori-core): correction_audit — LLM prompt builder + JSON parse + hallucination filter"
```

---

## Task 6: `correction_audit_config.rs` — Settings

**Files:**
- Create: `crates/mori-tauri/src/correction_audit_config.rs`
- Modify: `crates/mori-tauri/src/main.rs`(mod)

對齊既有 `notification_config.rs` pattern。

- [ ] **Step 1: 寫 module**

```rust
//! 2026-05-22:reminder 語音校正 audit 設定 — `~/.mori/config.json` 的 `correction_audit` 子樹。
//!
//! 對齊 `notification_config.rs` / `hotkey_config.rs` 既有 pattern:呼叫時讀檔 + 缺欄走預設,
//! 寫入時 round-trip 整個 JSON。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CorrectionAuditConfig {
    /// 對話結束後跑 LLM audit。預設 true。
    pub enabled: bool,
    /// LLM provider。預設 "groq"。
    pub provider: String,
    /// model。預設 "openai/gpt-oss-120b"(便宜)。
    pub model: String,
}

impl Default for CorrectionAuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: "groq".into(),
            model: "openai/gpt-oss-120b".into(),
        }
    }
}

impl CorrectionAuditConfig {
    pub fn load(config_path: &Path) -> Self {
        let raw = match std::fs::read_to_string(config_path) {
            Ok(s) => s,
            Err(_) => return Self::default(),
        };
        let json: Value = match serde_json::from_str(&raw) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(?e, "config.json malformed, correction_audit fall back to defaults");
                return Self::default();
            }
        };
        let sub = match json.get("correction_audit") {
            Some(v) => v.clone(),
            None => return Self::default(),
        };
        serde_json::from_value(sub).unwrap_or_else(|e| {
            tracing::warn!(?e, "correction_audit subtree malformed, falling back to defaults");
            Self::default()
        })
    }

    pub fn write(&self, config_path: &Path) -> Result<(), String> {
        let raw = std::fs::read_to_string(config_path).unwrap_or_else(|_| "{}".to_string());
        let mut json: Value =
            serde_json::from_str(&raw).map_err(|e| format!("parse config.json: {e}"))?;
        let obj = json
            .as_object_mut()
            .ok_or_else(|| "config.json root not object".to_string())?;
        obj.insert(
            "correction_audit".to_string(),
            serde_json::to_value(self).map_err(|e| e.to_string())?,
        );
        let pretty = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
        std::fs::write(config_path, pretty).map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn get_correction_audit_config() -> CorrectionAuditConfig {
    CorrectionAuditConfig::load(&crate::mori_dir().join("config.json"))
}

#[tauri::command]
pub fn set_correction_audit_config(cfg: CorrectionAuditConfig) -> Result<(), String> {
    cfg.write(&crate::mori_dir().join("config.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_returns_defaults_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        let cfg = CorrectionAuditConfig::load(&path);
        assert_eq!(cfg, CorrectionAuditConfig::default());
        assert!(cfg.enabled);
        assert_eq!(cfg.provider, "groq");
        assert_eq!(cfg.model, "openai/gpt-oss-120b");
    }

    #[test]
    fn write_preserves_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, r#"{"providers":{"groq":{}},"hotkeys":{"toggle":"X"}}"#).unwrap();
        CorrectionAuditConfig {
            enabled: false,
            provider: "groq".into(),
            model: "x".into(),
        }
        .write(&path)
        .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains(r#""providers""#));
        assert!(raw.contains(r#""hotkeys""#));
        assert!(raw.contains(r#""enabled": false"#));
    }

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, "{}").unwrap();
        let original = CorrectionAuditConfig {
            enabled: false,
            provider: "anthropic".into(),
            model: "claude-haiku".into(),
        };
        original.write(&path).unwrap();
        let loaded = CorrectionAuditConfig::load(&path);
        assert_eq!(loaded, original);
    }
}
```

- [ ] **Step 2: main.rs 加 mod**

加在合適位置(對齊既有 mod 排序,在 `mod correction_cmd;`(下一 task 加)之前):

```rust
mod correction_audit_config;
```

- [ ] **Step 3: 跑 tests pass**

Run: `cargo test -p mori-tauri --lib correction_audit_config -- --nocapture`
Expected: 3 PASS

- [ ] **Step 4: Commit**

```bash
git add crates/mori-tauri/src/correction_audit_config.rs crates/mori-tauri/src/main.rs
git commit -m "feat(mori-tauri): correction_audit_config — ~/.mori/config.json correction_audit subtree"
```

---

## Task 7: `correction_cmd.rs` — Tauri commands wrapper

**Files:**
- Create: `crates/mori-tauri/src/correction_cmd.rs`
- Modify: `crates/mori-tauri/src/main.rs`(mod + invoke_handler)

Commands:
- `correction_inbox_list()` → 給 UI 用的 grouped list
- `correction_inbox_accept(suggested, wrong_variants[])` → append corrections.md + mark all matching entries accepted
- `correction_inbox_dismiss(suggested, wrong_variants[])` → mark dismissed(後續 audit 跳過同 pair)
- `correction_inbox_change_suggestion(entry_id, new_suggested, accept: bool)` → 改建議,可選一起 accept
- `voice_feedback_set(session, rating, corrected?)` → 寫 feedback.json + 若 edit 跑 diff 寫 inbox candidates
- `corrections_md_content()` → 字典全文(viewer)
- `recordings_session_path(session_id)` → 給 UI 跳 RecordingsTab link 用

- [ ] **Step 1: 寫 module**

```rust
//! Correction Inbox / Voice Feedback / Corrections viewer 的 Tauri commands wrapper。
//!
//! 對應 spec §4.4-4.7。

use chrono::Utc;
use mori_core::correction_inbox::{
    self, group_pending_by_suggested, InboxEntry, InboxGroup, InboxSource, InboxStatus,
};
use mori_core::corrections_writer::append_correction;
use mori_core::voice_feedback::{diff_words, write_feedback, Feedback, FeedbackRating};
use serde::Deserialize;
use std::path::PathBuf;

fn inbox_path() -> PathBuf {
    crate::mori_dir().join("correction_inbox.jsonl")
}

fn corrections_md_path() -> PathBuf {
    crate::mori_dir().join("corrections.md")
}

fn recordings_dir() -> PathBuf {
    crate::mori_dir().join("recordings")
}

#[tauri::command]
pub fn correction_inbox_list() -> Result<Vec<InboxGroup>, String> {
    group_pending_by_suggested(&inbox_path()).map_err(|e| e.to_string())
}

/// `wrong_variants` 是要寫進 corrections.md 那行的 wrong 字 list(已 dedupe by UI)。
/// 後台:append corrections.md + 對 inbox 內所有 (wrong ∈ wrong_variants, suggested) 都
/// append 一筆 status=Accepted 的 entry(append-only,舊 pending entries 留)。
#[tauri::command]
pub fn correction_inbox_accept(suggested: String, wrong_variants: Vec<String>) -> Result<(), String> {
    if wrong_variants.is_empty() {
        return Err("wrong_variants 空".into());
    }
    append_correction(&corrections_md_path(), &wrong_variants, &suggested)
        .map_err(|e| format!("append corrections.md: {e}"))?;

    let now = Utc::now();
    for wrong in &wrong_variants {
        let mut accepted = InboxEntry::new_pending(
            "marker",
            InboxSource::LlmAudit,
            wrong,
            &suggested,
            1.0,
            "user accepted",
        );
        accepted.status = InboxStatus::Accepted;
        accepted.accepted_at = Some(now);
        correction_inbox::append_entry(&inbox_path(), &accepted)
            .map_err(|e| format!("append accepted entry: {e}"))?;
    }
    mori_core::event_log::append(serde_json::json!({
        "kind": "correction_inbox_accepted",
        "suggested": suggested,
        "wrong_variants": wrong_variants,
    }));
    Ok(())
}

#[tauri::command]
pub fn correction_inbox_dismiss(suggested: String, wrong_variants: Vec<String>) -> Result<(), String> {
    if wrong_variants.is_empty() {
        return Err("wrong_variants 空".into());
    }
    let now = Utc::now();
    for wrong in &wrong_variants {
        let mut dismissed = InboxEntry::new_pending(
            "marker",
            InboxSource::LlmAudit,
            wrong,
            &suggested,
            0.0,
            "user dismissed",
        );
        dismissed.status = InboxStatus::Dismissed;
        dismissed.dismissed_at = Some(now);
        correction_inbox::append_entry(&inbox_path(), &dismissed)
            .map_err(|e| format!("append dismissed entry: {e}"))?;
    }
    mori_core::event_log::append(serde_json::json!({
        "kind": "correction_inbox_dismissed",
        "suggested": suggested,
        "wrong_variants": wrong_variants,
    }));
    Ok(())
}

#[derive(Deserialize)]
pub struct ChangeSuggestionArgs {
    pub suggested: String,
    pub wrong_variants: Vec<String>,
    pub new_suggested: String,
}

#[tauri::command]
pub fn correction_inbox_change_suggestion(args: ChangeSuggestionArgs) -> Result<(), String> {
    // dismiss 原 (wrong, old_suggested) + accept 新 (wrong, new_suggested)
    correction_inbox_dismiss(args.suggested, args.wrong_variants.clone())?;
    correction_inbox_accept(args.new_suggested, args.wrong_variants)
}

#[derive(Deserialize)]
pub struct VoiceFeedbackArgs {
    pub session_id: String,
    pub rating: FeedbackRating,
    pub corrected_transcript: Option<String>,
    pub original_transcript: Option<String>,
    pub comment: Option<String>,
}

#[tauri::command]
pub fn voice_feedback_set(args: VoiceFeedbackArgs) -> Result<(), String> {
    let session_dir = recordings_dir().join(&args.session_id);
    let feedback = Feedback {
        rating: args.rating,
        rated_at: Utc::now(),
        corrected_transcript: args.corrected_transcript.clone(),
        comment: args.comment,
    };
    write_feedback(&session_dir, &feedback).map_err(|e| format!("write feedback.json: {e}"))?;

    // 若 rating=Edit 且兩段 transcript 都齊 → 跑 diff,把 (wrong, suggested) 寫進 inbox
    if matches!(args.rating, FeedbackRating::Edit) {
        if let (Some(orig), Some(corr)) = (args.original_transcript, args.corrected_transcript) {
            if orig != corr {
                let pairs = diff_words(&orig, &corr);
                for (wrong, suggested) in pairs {
                    // skip 純空白變化
                    if wrong.trim().is_empty() || suggested.trim().is_empty() {
                        continue;
                    }
                    let entry = InboxEntry::new_pending(
                        &args.session_id,
                        InboxSource::UserEdit,
                        &wrong,
                        &suggested,
                        0.95,
                        "user edit transcript diff",
                    );
                    correction_inbox::append_entry(&inbox_path(), &entry)
                        .map_err(|e| format!("append user_edit inbox entry: {e}"))?;
                }
            }
        }
    }
    mori_core::event_log::append(serde_json::json!({
        "kind": "voice_feedback_rated",
        "session_id": args.session_id,
        "rating": format!("{:?}", feedback.rating),
    }));
    Ok(())
}

#[tauri::command]
pub fn corrections_md_content() -> Result<String, String> {
    let path = corrections_md_path();
    if !path.exists() {
        return Ok(String::new());
    }
    std::fs::read_to_string(&path).map_err(|e| format!("read corrections.md: {e}"))
}
```

- [ ] **Step 2: main.rs 加 mod + invoke_handler**

```rust
mod correction_cmd;
```

`tauri::generate_handler![ ... ]` 加(對齊既有風格,加在 reminders_cmd 系列之後):
```rust
correction_audit_config::get_correction_audit_config,
correction_audit_config::set_correction_audit_config,
correction_cmd::correction_inbox_list,
correction_cmd::correction_inbox_accept,
correction_cmd::correction_inbox_dismiss,
correction_cmd::correction_inbox_change_suggestion,
correction_cmd::voice_feedback_set,
correction_cmd::corrections_md_content,
```

- [ ] **Step 3: workspace check**

Run: `cargo check --workspace --all-targets`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/mori-tauri/src/correction_cmd.rs crates/mori-tauri/src/main.rs
git commit -m "feat(mori-tauri): correction_cmd — Tauri commands for inbox / feedback / corrections viewer"
```

---

## Task 8: Voice pipeline hook — spawn audit after voice complete

**Files:**
- Modify: `crates/mori-tauri/src/main.rs`(`run_voice_input_pipeline` 結尾 spawn audit task)

- [ ] **Step 1: 找 voice pipeline end 位置**

Run: `grep -n 'event_log::append.*voice_input_completed' crates/mori-tauri/src/main.rs`

定位該行。緊接在它之後加 spawn audit task。

- [ ] **Step 2: 加 spawn audit task**

在 `voice_input_completed` event_log append 之後加:

```rust
// 2026-05-22:Correction audit — 對話結束後 background LLM 跑一次,把可能諧音錯字
// 候選寫進 inbox。失敗 silent(僅 log),不擋 voice pipeline。可在 ConfigTab
// 校正 sub-tab toggle 關掉。
{
    let cfg = correction_audit_config::CorrectionAuditConfig::load(
        &mori_dir().join("config.json"),
    );
    if cfg.enabled {
        let raw = transcript.clone();
        let cleaned = cleaned_text.clone();
        let session_id = state
            .recording_session
            .lock()
            .as_ref()
            .map(|s| s.session_id().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let routing_path = mori_dir().join("config.json");
        tauri::async_runtime::spawn(async move {
            // build provider via routing
            let routing = match mori_core::llm::Routing::build_from_config(Some(&routing_path)) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(?e, "correction_audit: routing build failed, skip");
                    return;
                }
            };
            let provider = routing.skill_provider("correction_audit");

            // 讀 corrections.md 全文
            let corrections_md_path = mori_dir().join("corrections.md");
            let corrections_md = std::fs::read_to_string(&corrections_md_path).unwrap_or_default();

            mori_core::event_log::append(serde_json::json!({
                "kind": "correction_audit_started",
                "session_id": session_id,
            }));

            match mori_core::correction_audit::audit(provider, &raw, &cleaned, &corrections_md).await {
                Ok(candidates) => {
                    let inbox_path = mori_dir().join("correction_inbox.jsonl");
                    let mut written = 0usize;
                    for c in &candidates {
                        // 已 dismiss 過(同 wrong+suggested)→ skip
                        let is_dismissed = mori_core::correction_inbox::is_dismissed(
                            &inbox_path,
                            &c.wrong,
                            &c.suggested,
                        )
                        .unwrap_or(false);
                        if is_dismissed {
                            continue;
                        }
                        let entry = mori_core::correction_inbox::InboxEntry::new_pending(
                            &session_id,
                            mori_core::correction_inbox::InboxSource::LlmAudit,
                            &c.wrong,
                            &c.suggested,
                            c.confidence,
                            &c.reason,
                        );
                        if let Err(e) = mori_core::correction_inbox::append_entry(&inbox_path, &entry) {
                            tracing::warn!(?e, "correction_audit: append inbox entry failed");
                            continue;
                        }
                        written += 1;
                    }
                    mori_core::event_log::append(serde_json::json!({
                        "kind": "correction_audit_completed",
                        "session_id": session_id,
                        "candidates_total": candidates.len(),
                        "candidates_written": written,
                    }));
                }
                Err(e) => {
                    tracing::warn!(?e, "correction_audit failed");
                    mori_core::event_log::append(serde_json::json!({
                        "kind": "correction_audit_failed",
                        "session_id": session_id,
                        "error": format!("{e:#}"),
                    }));
                }
            }
        });
    }
}
```

注意:`recording_session` 是 `Mutex<Option<SessionRecord>>`,要 verify 那個 method 名是 `session_id()`。若不同名,grep 找出 + 對齊。

`mori_core::llm::Routing::skill_provider("correction_audit")` 需要 routing config 內可能沒有 `correction_audit` key,fallback 應該回 default(對齊既有 `skill_provider` 行為)。確認既有 routing 行為。若有疑問,改成直接 build groq provider:

```rust
let groq = mori_core::llm::groq::GroqProvider::from_config(&routing_path)
    .map(|p| Arc::new(p) as Arc<dyn mori_core::llm::LlmProvider>);
```

(simplest path:既有 ReminderService::new 是怎麼拿 provider 的就照那條 pattern。)

- [ ] **Step 3: workspace check**

Run: `cargo check --workspace --all-targets`
Expected: PASS

- [ ] **Step 4: 全 tests 還過**

Run: `cargo test -p mori-core --lib && cargo test -p mori-tauri --lib`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/mori-tauri/src/main.rs
git commit -m "feat(mori-tauri): voice pipeline spawns correction_audit task after voice_input_completed"
```

---

## Task 9: `CorrectionsTab.tsx` UI — inbox + viewer

**Files:**
- Create: `src/tabs/CorrectionsTab.tsx`
- Create: `src/tabs/corrections-tab.css`

對齊既有 ConfigTab / RecordingsTab style。

- [ ] **Step 1: 寫 component**

```tsx
// 2026-05-22:校正盒 — STT 諧音錯字 inbox + corrections.md viewer。
//
// 兩個 section:
// 1. Inbox(校正盒)— pending entries 按 suggested 字 grouping,每行 row
//    顯示 variants(wrong × count),接受 / 改建議 / 忽略
// 2. corrections.md viewer — readonly markdown view,顯示既有字典

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./corrections-tab.css";

type InboxVariant = {
  wrong: string;
  count: number;
  entry_ids: string[];
  earliest_session: string;
  max_confidence: number;
};

type InboxGroup = {
  suggested: string;
  variants: InboxVariant[];
  has_user_edit: boolean;
};

function CorrectionsTab() {
  const [groups, setGroups] = useState<InboxGroup[]>([]);
  const [corrections, setCorrections] = useState<string>("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<Record<string, string>>({}); // suggested → newSuggested
  const [busy, setBusy] = useState<Record<string, boolean>>({}); // suggested → true 鎖住按鈕

  const refresh = async () => {
    setLoading(true);
    try {
      const [g, c] = await Promise.all([
        invoke<InboxGroup[]>("correction_inbox_list"),
        invoke<string>("corrections_md_content"),
      ]);
      setGroups(g);
      setCorrections(c);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const onAccept = async (group: InboxGroup, override?: string) => {
    const target = override ?? group.suggested;
    setBusy((b) => ({ ...b, [group.suggested]: true }));
    try {
      const wrongs = group.variants.map((v) => v.wrong);
      if (override && override !== group.suggested) {
        await invoke("correction_inbox_change_suggestion", {
          args: {
            suggested: group.suggested,
            wrong_variants: wrongs,
            new_suggested: override,
          },
        });
      } else {
        await invoke("correction_inbox_accept", {
          suggested: target,
          wrongVariants: wrongs,
        });
      }
      await refresh();
    } catch (e) {
      alert(`接受失敗:${e}`);
    } finally {
      setBusy((b) => ({ ...b, [group.suggested]: false }));
      setEditing((s) => {
        const copy = { ...s };
        delete copy[group.suggested];
        return copy;
      });
    }
  };

  const onDismiss = async (group: InboxGroup) => {
    setBusy((b) => ({ ...b, [group.suggested]: true }));
    try {
      const wrongs = group.variants.map((v) => v.wrong);
      await invoke("correction_inbox_dismiss", {
        suggested: group.suggested,
        wrongVariants: wrongs,
      });
      await refresh();
    } catch (e) {
      alert(`忽略失敗:${e}`);
    } finally {
      setBusy((b) => ({ ...b, [group.suggested]: false }));
    }
  };

  if (loading) return <div className="corrections-tab">載入中...</div>;

  return (
    <div className="corrections-tab">
      {error && <div className="corrections-error">⚠ {error}</div>}

      <section className="corrections-section">
        <h2>
          🔔 校正盒
          {groups.length > 0 && (
            <span className="corrections-count">({groups.length} 個 pending)</span>
          )}
        </h2>

        {groups.length === 0 ? (
          <p className="corrections-empty">沒有待處理候選。Mori 對話結束後會自動偵測諧音錯字放進來。</p>
        ) : (
          <ul className="corrections-inbox-list">
            {groups.map((group) => {
              const isBusy = busy[group.suggested];
              const editingValue = editing[group.suggested];
              const isEditing = editingValue !== undefined;
              return (
                <li
                  key={group.suggested}
                  className={`corrections-inbox-row ${group.has_user_edit ? "is-user-edit" : ""}`}
                >
                  <div className="corrections-row-head">
                    {isEditing ? (
                      <input
                        type="text"
                        className="corrections-suggested-edit"
                        value={editingValue}
                        onChange={(e) =>
                          setEditing((s) => ({ ...s, [group.suggested]: e.target.value }))
                        }
                      />
                    ) : (
                      <span className="corrections-suggested">{group.suggested}</span>
                    )}
                    <span className="corrections-arrow">←</span>
                    <span className="corrections-variants">
                      {group.variants.map((v, i) => (
                        <span key={v.wrong}>
                          {i > 0 && ", "}
                          {v.wrong}
                          {v.count > 1 && (
                            <span className="corrections-count-badge">×{v.count}</span>
                          )}
                        </span>
                      ))}
                    </span>
                  </div>
                  <div className="corrections-row-actions">
                    {isEditing ? (
                      <>
                        <button
                          disabled={isBusy}
                          onClick={() => onAccept(group, editingValue)}
                        >
                          ✓ 接受新建議
                        </button>
                        <button
                          disabled={isBusy}
                          onClick={() =>
                            setEditing((s) => {
                              const copy = { ...s };
                              delete copy[group.suggested];
                              return copy;
                            })
                          }
                        >
                          取消
                        </button>
                      </>
                    ) : (
                      <>
                        <button disabled={isBusy} onClick={() => onAccept(group)}>
                          ✓ 接受
                        </button>
                        <button
                          disabled={isBusy}
                          onClick={() =>
                            setEditing((s) => ({ ...s, [group.suggested]: group.suggested }))
                          }
                        >
                          改建議
                        </button>
                        <button disabled={isBusy} onClick={() => onDismiss(group)}>
                          ✗ 忽略
                        </button>
                      </>
                    )}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      <section className="corrections-section">
        <h2>📖 字典 corrections.md</h2>
        <pre className="corrections-md-viewer">{corrections || "(空)"}</pre>
      </section>
    </div>
  );
}

export default CorrectionsTab;
```

- [ ] **Step 2: 寫 CSS**

`src/tabs/corrections-tab.css`:

```css
.corrections-tab {
  padding: 16px;
  font-family: system-ui, sans-serif;
  font-size: 14px;
  color: var(--text-color, #f1eee0);
}

.corrections-error {
  background: rgba(200, 80, 80, 0.2);
  border: 1px solid rgba(200, 80, 80, 0.5);
  padding: 8px 12px;
  border-radius: 6px;
  margin-bottom: 12px;
}

.corrections-section {
  margin-bottom: 24px;
}

.corrections-section h2 {
  font-size: 16px;
  margin: 0 0 12px 0;
  display: flex;
  align-items: baseline;
  gap: 8px;
}

.corrections-count {
  font-size: 12px;
  color: rgba(255, 255, 255, 0.55);
  font-weight: normal;
}

.corrections-empty {
  color: rgba(255, 255, 255, 0.55);
  font-style: italic;
}

.corrections-inbox-list {
  list-style: none;
  padding: 0;
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.corrections-inbox-row {
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 8px;
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.corrections-inbox-row.is-user-edit {
  border-color: rgba(201, 162, 77, 0.4);
}

.corrections-row-head {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.corrections-suggested {
  font-weight: 600;
  color: #c9a24d;
  font-size: 15px;
}

.corrections-suggested-edit {
  background: rgba(255, 255, 255, 0.08);
  color: inherit;
  border: 1px solid rgba(201, 162, 77, 0.6);
  border-radius: 4px;
  padding: 2px 6px;
  font-size: 14px;
}

.corrections-arrow {
  color: rgba(255, 255, 255, 0.4);
}

.corrections-variants {
  color: rgba(255, 255, 255, 0.85);
}

.corrections-count-badge {
  font-size: 11px;
  color: rgba(255, 255, 255, 0.55);
  margin-left: 2px;
}

.corrections-row-actions {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.corrections-row-actions button {
  background: rgba(255, 255, 255, 0.08);
  color: inherit;
  border: 1px solid rgba(255, 255, 255, 0.15);
  border-radius: 6px;
  padding: 4px 10px;
  cursor: pointer;
  font-size: 12px;
}

.corrections-row-actions button:hover:not(:disabled) {
  background: rgba(255, 255, 255, 0.14);
}

.corrections-row-actions button:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.corrections-md-viewer {
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 6px;
  padding: 12px;
  font-family: monospace;
  font-size: 12px;
  white-space: pre-wrap;
  max-height: 60vh;
  overflow-y: auto;
}
```

- [ ] **Step 3: TS check**

Run: `npx tsc --noEmit`
Expected: 0 errors

- [ ] **Step 4: build pass**

Run: `npm run build`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/tabs/CorrectionsTab.tsx src/tabs/corrections-tab.css
git commit -m "feat(ui): CorrectionsTab — inbox grouped list + corrections.md viewer"
```

---

## Task 10: 註冊 CorrectionsTab 進 MainShell

**Files:**
- Modify: `src/MainShell.tsx`(加 tab)

- [ ] **Step 1: 找既有 tabs config**

Run: `grep -n 'ConfigTab\|RecordingsTab' src/MainShell.tsx`

對齊既有 tabs array 結構,加 CorrectionsTab。

- [ ] **Step 2: import + 加 tab entry**

加 import:
```tsx
import CorrectionsTab from "./tabs/CorrectionsTab";
```

加 tab entry(對齊既有 schema,通常 `{ id, label, component, icon? }`):
```tsx
{
  id: "corrections",
  label: "校正",  // i18n follow-up
  component: <CorrectionsTab />,
}
```

放在 RecordingsTab 後 / ConfigTab 前(或對齊邏輯位置)。

- [ ] **Step 3: TS check + build**

Run: `npx tsc --noEmit && npm run build`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/MainShell.tsx
git commit -m "feat(ui): register CorrectionsTab in MainShell"
```

---

## Task 11: ConfigTab 校正 sub-tab — audit enabled toggle

**Files:**
- Modify: `src/tabs/ConfigTab.tsx`(加 sub-tab + toggle)

對齊既有「通知」sub-tab pattern。

- [ ] **Step 1: 找通知 sub-tab 加進去的位置**

Run: `grep -n 'notification_config\|set_notification_config\|通知' src/tabs/ConfigTab.tsx`

對齊既有 sub-tab 機制(可能是 sub-tab tab bar + content switch)。

- [ ] **Step 2: 加 sub-tab + state + UI**

對齊既有 NotificationConfig section pattern,加:

```tsx
type CorrectionAuditConfig = {
  enabled: boolean;
  provider: string;
  model: string;
};

const [auditCfg, setAuditCfg] = useState<CorrectionAuditConfig>({
  enabled: true,
  provider: "groq",
  model: "openai/gpt-oss-120b",
});

useEffect(() => {
  invoke<CorrectionAuditConfig>("get_correction_audit_config")
    .then(setAuditCfg)
    .catch((e) => console.warn("get_correction_audit_config failed", e));
}, []);

const saveAudit = async (next: CorrectionAuditConfig) => {
  setAuditCfg(next);
  try {
    await invoke("set_correction_audit_config", { cfg: next });
  } catch (e) {
    alert(`儲存失敗:${e}`);
  }
};
```

UI section(放在通知 section 旁,作為新 sub-tab 或同層 section):

```tsx
<section className="mori-config-section">
  <h3 className="mori-config-section-title">校正(LLM audit)</h3>
  <div className="mori-form-row">
    <label className="config-toggle">
      <input
        type="checkbox"
        checked={auditCfg.enabled}
        onChange={(e) => saveAudit({ ...auditCfg, enabled: e.target.checked })}
      />
      <span>對話結束後自動偵測諧音錯字</span>
    </label>
    <small className="mori-config-section-hint">
      Mori 對話結束後跑一次 LLM(預設 Groq gpt-oss-120b 便宜),把可能的 STT 諧音錯字
      候選放進「校正盒」分頁,等你確認加入字典。關掉就不跑。
    </small>
  </div>
</section>
```

(若 ConfigTab 既有 sub-tab 機制,加新 sub-tab `"corrections"`,否則 inline 加進通知 section 旁。實作時對齊現有結構。)

- [ ] **Step 3: TS check + build**

Run: `npx tsc --noEmit && npm run build`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/tabs/ConfigTab.tsx
git commit -m "feat(ui): ConfigTab — add correction_audit.enabled toggle (校正 section)"
```

---

## Task 12: ChatPanel + RecordingsTab — 👍/👎/✏️ 評分按鈕

**Files:**
- Modify: `src/ChatPanel.tsx`(每筆 voice message 加按鈕)
- Modify: `src/tabs/RecordingsTab.tsx`(歷史 session 同樣按鈕)

- [ ] **Step 1: 加 RatingButtons 元件**

新一個 reusable 子元件(in `src/ChatPanel.tsx` 內 inline 或拉新檔 `src/RatingButtons.tsx`):

```tsx
type RatingButtonsProps = {
  sessionId: string;
  originalTranscript: string;
};

function RatingButtons({ sessionId, originalTranscript }: RatingButtonsProps) {
  const [editing, setEditing] = useState(false);
  const [correctedText, setCorrectedText] = useState(originalTranscript);
  const [busy, setBusy] = useState(false);

  const setRating = async (rating: "good" | "bad") => {
    setBusy(true);
    try {
      await invoke("voice_feedback_set", {
        args: {
          session_id: sessionId,
          rating,
          corrected_transcript: null,
          original_transcript: null,
          comment: null,
        },
      });
    } catch (e) {
      alert(`評分失敗:${e}`);
    } finally {
      setBusy(false);
    }
  };

  const submitEdit = async () => {
    setBusy(true);
    try {
      await invoke("voice_feedback_set", {
        args: {
          session_id: sessionId,
          rating: "edit",
          corrected_transcript: correctedText,
          original_transcript: originalTranscript,
          comment: null,
        },
      });
      setEditing(false);
    } catch (e) {
      alert(`改寫儲存失敗:${e}`);
    } finally {
      setBusy(false);
    }
  };

  if (editing) {
    return (
      <div className="rating-edit-box">
        <textarea
          value={correctedText}
          onChange={(e) => setCorrectedText(e.target.value)}
          rows={3}
        />
        <div>
          <button disabled={busy} onClick={submitEdit}>儲存</button>
          <button disabled={busy} onClick={() => setEditing(false)}>取消</button>
        </div>
      </div>
    );
  }

  return (
    <div className="rating-buttons">
      <button disabled={busy} onClick={() => setRating("good")} title="這次轉文字準確">👍</button>
      <button disabled={busy} onClick={() => setRating("bad")} title="這次轉文字不準">👎</button>
      <button disabled={busy} onClick={() => setEditing(true)} title="改寫成正確的">✏️</button>
    </div>
  );
}
```

- [ ] **Step 2: ChatPanel.tsx 內,voice message render 旁加 `<RatingButtons />`**

找既有 voice message render(role === "voice_input")block,在 copy 按鈕同 row 加 `<RatingButtons sessionId={...} originalTranscript={...} />`。

關鍵問題:**ChatPanel 怎麼拿 sessionId**?從 conversation state 看,如果 ChatMessage 已有 `session_id`(或從 archive log 對齊)則直接用。若沒有,要先讓 voice pipeline 把 session_id 放進 ChatMessage(這是 ChatPanel.tsx 改動的 scope shift,可能要回 mori-tauri 端加)。

**簡化路徑**:MVP 用「最近一筆 session 從 RecordingsTab 載入」當 fallback;or 直接在 voice pipeline 結束時把 session_id append 進 conversation push 進來(ChatMessage 加 optional `session_id` field)。看實作時哪條最少改動就走。

`grep -n 'role.*voice_input\|conversation.lock' crates/mori-tauri/src/main.rs` 看既有結構。

- [ ] **Step 3: RecordingsTab.tsx 加按鈕**

對 RecordingsTab 每筆 session row 加同樣 `<RatingButtons sessionId={s.id} originalTranscript={s.transcript} />`。從 RecordingsTab 已經能拿到 session_id + transcript,直接加。

- [ ] **Step 4: TS check + build**

Run: `npx tsc --noEmit && npm run build`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/ChatPanel.tsx src/tabs/RecordingsTab.tsx src/RatingButtons.tsx
git commit -m "feat(ui): RatingButtons — 👍/👎/✏️ on voice messages in ChatPanel + RecordingsTab"
```

---

## Task 13: Workspace verify + manual smoke + final commit

- [ ] **Step 1: 全 workspace verify**

Run: `bash scripts/verify.sh`
Expected: PASS(`npm run build` + `cargo test -p mori-core --lib` + `cargo check --workspace --all-targets`)

- [ ] **Step 2: Manual smoke**(逐項 user 互動驗證,**勾不到就回頭修**)

啟動 `npm run tauri dev`,然後:

- [ ] 對 Mori 講「英檔有什麼內容」(故意諧音)→ Mori 應該校成「音檔」→ 對話結束 ~2 秒後 LogsTab 看到 `kind: correction_audit_completed`,CorrectionsTab 校正盒一筆「音檔 ← 英檔」
- [ ] 點 [✓ 接受] → corrections.md User 段多一行「英檔 -> 音檔」(用 `cat ~/.mori/corrections.md | tail` 驗)
- [ ] 同詞再講一次 → audit 應該跳過(因為 corrections.md 已收錄,prompt 排除規則生效),inbox 不再多新 entry
- [ ] 對另一個諧音講 →「馬當」→ Mori 校「Markdown」→ inbox 出現 → 點 [✗ 忽略] → CorrectionsTab list 該 entry 消失,DB 確認 dismissed entry 寫入(`tail ~/.mori/correction_inbox.jsonl`)
- [ ] 同「馬當」再講一次 → audit 跳過(is_dismissed 過濾)
- [ ] Chat panel 對某 voice message 點 ✏️ → inline editor → 改文字 → 儲存 → `cat ~/.mori/recordings/<latest>/feedback.json` 看 rating=edit + corrected_transcript;且 inbox 多 user_edit source 的 entry
- [ ] Chat panel 點 👍 → feedback.json rating=good
- [ ] RecordingsTab 對歷史 session 同樣三按鈕運作
- [ ] ConfigTab 切「校正(LLM audit)」toggle off → 對話結束後不跑 audit(LogsTab 無 correction_audit_started event)
- [ ] CorrectionsTab corrections.md viewer 區段顯示完整字典內容(包含剛 append 進去的條目)
- [ ] 故意把 corrections.md 改成 chaos format(沒 ## User heading)→ 點接受還是該 append(fallback path)

- [ ] **Step 3: 全 smoke 過 → 最終 commit(若還有 lint 修補)**

```bash
git status  # 看有沒有 lint/format 殘留
git add -A
git commit -m "chore: smoke verify pass — voice correction inbox MVP ready"  # 若沒額外改可省略
```

- [ ] **Step 4: 整理 push + PR**

```bash
git log --oneline main..feat/correction-inbox  # 預期 12-13 個 commits
```

push + 開 PR(對應 spec doc + plan link)。

---

## Self-review checklist

- [x] 每 task 有 file paths 明確
- [x] 每 step 有 verbatim code,沒有「按 X 風格實作」模糊
- [x] TDD:Task 1-7 都先寫 failing test 再實作;Task 8-12 是 wiring / UI,manual smoke 主要驗證
- [x] DRY:correction_inbox / corrections_writer / voice_feedback / correction_audit 各自單一責任,Tauri commands 純 wrapper
- [x] Spec coverage:
  - §4.1 資料流 → Tasks 1, 4, 5, 8
  - §4.2 LLM prompt → Task 5
  - §4.3 storage → Tasks 1, 4
  - §4.4 CorrectionsTab UI → Task 9
  - §4.5 評分 UI → Task 12
  - §4.6 Settings → Tasks 6, 11
  - §4.7 corrections.md User 段 → Task 3
  - §5 error handling → 每 task 內 error path tests 覆蓋
  - §6 testing → Tasks 1-7 unit;Task 13 manual smoke
- [x] 命名一致:`InboxEntry` / `InboxGroup` / `InboxVariant` / `InboxStatus` / `InboxSource` / `Feedback` / `FeedbackRating` / `CorrectionAuditConfig` / `AuditCandidate` 全文對齊
- [x] camelCase / snake_case 對齊:Rust serde `#[serde(rename_all = "snake_case")]` 預設;TS 接到 snake_case(對齊既有 NotificationConfig pattern)
