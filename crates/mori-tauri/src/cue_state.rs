//! BI-4:Cue state 的 mori-tauri 薄 shim。
//! 跟 BI-2 permission_broker 同 pattern:路徑決定 + RFC3339 timestamp 生成 + 跨平台 path 開啟。
//! 純邏輯(append / read)在 mori_core::body::cue_state,本檔不重複。

use mori_core::body::{append_state, read_state_map, CueAction, CueStateEntry};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// `~/.mori/cue-state.jsonl` — 跟 BI-2 audit log 同根。
pub fn state_path() -> PathBuf {
    crate::mori_dir().join("cue-state.jsonl")
}

/// 寫一筆 cue action(對指定路徑,可測)。
pub fn append_at(
    path: &Path,
    event_id: &str,
    action: CueAction,
    now: &str,
) -> Result<(), String> {
    let entry = CueStateEntry {
        timestamp: now.to_string(),
        event_id: event_id.to_string(),
        action,
    };
    append_state(path, &entry)
}

/// 對真實 ~/.mori 路徑寫一筆,timestamp 現在生。
pub fn append_now(event_id: &str, action: CueAction) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    append_at(&state_path(), event_id, action, &now)
}

/// 讀整個狀態 map(`event_id → 最後 action`)。
pub fn list() -> HashMap<String, CueAction> {
    read_state_map(&state_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_at_then_read_roundtrips() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("cue-state.jsonl");
        append_at(&path, "evt-1", CueAction::Ack, "2026-05-28T10:00:00+08:00").unwrap();
        let map = read_state_map(&path);
        assert_eq!(map.get("evt-1"), Some(&CueAction::Ack));
    }

    #[test]
    fn append_at_writes_to_provided_path_not_home() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("custom.jsonl");
        append_at(
            &path,
            "evt-x",
            CueAction::Dismiss,
            "2026-05-28T10:00:00+08:00",
        )
        .unwrap();
        assert!(path.exists());
    }
}
