//! BI-4 Cue Center 狀態 — append-only JSONL,replay 後 last-action-wins per event_id。
//! 路徑 `~/.mori/cue-state.jsonl`(由 mori-tauri 決定);本檔只接 &Path。
//! 跟 [`super::permission_audit`] 同 pattern,但這裡讀的是「最後狀態」,不是 tail。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

/// 一筆 cue action 紀錄。append 一行 JSON。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CueStateEntry {
    pub timestamp: String,
    pub event_id: String,
    pub action: CueAction,
}

/// User 對 cue 的動作。`Snooze` 帶 `until`(RFC3339 字串)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CueAction {
    Ack,
    Snooze { until: String },
    Dismiss,
}

/// 寫一筆 entry(append-only,建父目錄)。
pub fn append_state(path: &Path, entry: &CueStateEntry) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(f, "{line}").map_err(|e| e.to_string())?;
    Ok(())
}

/// Replay 整個 log,回 `event_id → 最後一筆 action`。缺檔 / 壞行都降級成空 / 跳過(不 fatal)。
pub fn read_state_map(path: &Path) -> HashMap<String, CueAction> {
    let body = match std::fs::read_to_string(path) {
        Ok(b) => b,
        Err(_) => return HashMap::new(),
    };
    let mut map: HashMap<String, CueAction> = HashMap::new();
    for line in body.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<CueStateEntry>(line) {
            // last-action-wins:append 順序就是時間序,後寫的覆蓋前面。
            map.insert(entry.event_id, entry.action);
        }
    }
    map
}

/// `until` (RFC3339) 跟 `now` (RFC3339) 比,still snoozed = until > now。
/// parse 失敗 → false(視同 expired,讓 cue 復活而不是卡死)。
pub fn is_snooze_active(until: &str, now: &str) -> bool {
    use chrono::DateTime;
    match (
        DateTime::parse_from_rfc3339(until),
        DateTime::parse_from_rfc3339(now),
    ) {
        (Ok(u), Ok(n)) => u > n,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn entry(event_id: &str, action: CueAction) -> CueStateEntry {
        CueStateEntry {
            timestamp: "2026-05-28T10:00:00+08:00".into(),
            event_id: event_id.into(),
            action,
        }
    }

    #[test]
    fn append_then_read_roundtrips_single_entry() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("cue-state.jsonl");
        append_state(&path, &entry("evt-1", CueAction::Ack)).unwrap();
        let map = read_state_map(&path);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("evt-1"), Some(&CueAction::Ack));
    }

    #[test]
    fn last_action_wins_per_event_id() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("cue-state.jsonl");
        append_state(&path, &entry("evt-1", CueAction::Ack)).unwrap();
        append_state(
            &path,
            &entry(
                "evt-1",
                CueAction::Snooze {
                    until: "2026-05-28T11:00:00+08:00".into(),
                },
            ),
        )
        .unwrap();
        append_state(&path, &entry("evt-1", CueAction::Dismiss)).unwrap();
        let map = read_state_map(&path);
        assert_eq!(map.get("evt-1"), Some(&CueAction::Dismiss));
    }

    #[test]
    fn snooze_round_trips_with_until_field() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("cue-state.jsonl");
        let until = "2026-05-28T11:00:00+08:00".to_string();
        append_state(
            &path,
            &entry(
                "evt-snooze",
                CueAction::Snooze {
                    until: until.clone(),
                },
            ),
        )
        .unwrap();
        let map = read_state_map(&path);
        match map.get("evt-snooze") {
            Some(CueAction::Snooze { until: u }) => assert_eq!(u, &until),
            other => panic!("expected snooze, got {:?}", other),
        }
    }

    #[test]
    fn missing_file_returns_empty_map() {
        let tmp = TempDir::new().unwrap();
        let map = read_state_map(&tmp.path().join("nope.jsonl"));
        assert!(map.is_empty());
    }

    #[test]
    fn corrupt_line_is_skipped_not_fatal() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("cue-state.jsonl");
        append_state(&path, &entry("good", CueAction::Ack)).unwrap();
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "{{ not json").unwrap();
        append_state(&path, &entry("good2", CueAction::Dismiss)).unwrap();
        let map = read_state_map(&path);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("good"), Some(&CueAction::Ack));
        assert_eq!(map.get("good2"), Some(&CueAction::Dismiss));
    }

    #[test]
    fn snooze_active_when_until_after_now() {
        assert!(is_snooze_active(
            "2026-05-28T11:00:00+08:00",
            "2026-05-28T10:00:00+08:00",
        ));
    }

    #[test]
    fn snooze_inactive_when_until_before_now() {
        assert!(!is_snooze_active(
            "2026-05-28T09:00:00+08:00",
            "2026-05-28T10:00:00+08:00",
        ));
    }

    #[test]
    fn snooze_inactive_on_parse_failure() {
        assert!(!is_snooze_active("not-a-date", "2026-05-28T10:00:00+08:00"));
    }
}
