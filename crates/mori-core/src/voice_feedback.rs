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
