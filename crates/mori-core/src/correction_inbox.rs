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

/// 只回真正 pending 的 entries。
///
/// Append-only jsonl 裡,accept/dismiss 是寫新 marker entry(新 uuid),不改舊行。
/// 所以要先收集所有已有 non-Pending marker 的 (wrong, suggested) pair,
/// 再過濾掉同 pair 的舊 Pending entries,讓 UI refresh 後 entry 正確消失。
pub fn list_pending(path: &Path) -> Result<Vec<InboxEntry>, InboxError> {
    let all = list_all(path)?;
    // 先收集所有已被標 Accepted/Dismissed 的 (wrong, suggested) pair
    let resolved: std::collections::HashSet<(String, String)> = all
        .iter()
        .filter(|e| !matches!(e.status, InboxStatus::Pending))
        .map(|e| (e.wrong.clone(), e.suggested.clone()))
        .collect();
    // 回 status=Pending 且 (wrong, suggested) 不在 resolved set 內的 entries
    Ok(all
        .into_iter()
        .filter(|e| matches!(e.status, InboxStatus::Pending))
        .filter(|e| !resolved.contains(&(e.wrong.clone(), e.suggested.clone())))
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

    #[test]
    fn list_pending_excludes_pair_after_accepted_marker() {
        let (_dir, path) = tmp_path();
        // 1. Pending entry
        let pending = InboxEntry::new_pending("s1", InboxSource::LlmAudit, "英檔", "音檔", 0.8, "");
        append_entry(&path, &pending).unwrap();
        assert_eq!(list_pending(&path).unwrap().len(), 1, "Pending 應該回 1");

        // 2. 同 pair 加 Accepted marker
        let mut accepted = InboxEntry::new_pending("marker", InboxSource::LlmAudit, "英檔", "音檔", 1.0, "");
        accepted.status = InboxStatus::Accepted;
        accepted.accepted_at = Some(Utc::now());
        append_entry(&path, &accepted).unwrap();

        // 3. list_pending 該回 0(舊 Pending 被新 Accepted marker 蓋掉)
        assert_eq!(
            list_pending(&path).unwrap().len(),
            0,
            "Pending entry for same pair should be filtered after Accepted marker exists"
        );
    }

    #[test]
    fn list_pending_excludes_pair_after_dismissed_marker() {
        let (_dir, path) = tmp_path();
        let pending = InboxEntry::new_pending("s1", InboxSource::LlmAudit, "馬當", "Markdown", 0.9, "");
        append_entry(&path, &pending).unwrap();

        let mut dismissed = InboxEntry::new_pending("marker", InboxSource::LlmAudit, "馬當", "Markdown", 0.0, "");
        dismissed.status = InboxStatus::Dismissed;
        dismissed.dismissed_at = Some(Utc::now());
        append_entry(&path, &dismissed).unwrap();

        assert_eq!(list_pending(&path).unwrap().len(), 0);
    }

    #[test]
    fn list_pending_keeps_other_pairs_when_one_resolved() {
        let (_dir, path) = tmp_path();
        // 一筆 (a, X) Pending
        append_entry(&path, &InboxEntry::new_pending("s1", InboxSource::LlmAudit, "a", "X", 0.5, "")).unwrap();
        // 一筆 (b, Y) Pending
        append_entry(&path, &InboxEntry::new_pending("s2", InboxSource::LlmAudit, "b", "Y", 0.5, "")).unwrap();
        // (a, X) Accepted marker
        let mut marker = InboxEntry::new_pending("marker", InboxSource::LlmAudit, "a", "X", 1.0, "");
        marker.status = InboxStatus::Accepted;
        marker.accepted_at = Some(Utc::now());
        append_entry(&path, &marker).unwrap();

        let result = list_pending(&path).unwrap();
        assert_eq!(result.len(), 1, "另一個 pair 不該被影響");
        assert_eq!(result[0].wrong, "b");
    }
}
