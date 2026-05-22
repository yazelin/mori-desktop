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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
        let result = list_pending(&path).unwrap();
        assert!(result.is_empty());
    }
}
