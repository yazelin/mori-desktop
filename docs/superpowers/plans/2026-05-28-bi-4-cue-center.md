# BI-4 Cue Center Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 BI-3 PulseTab 的唯讀 cue 列升級成可操作 — 每筆 cue 可以 acknowledge / snooze / jump / dismiss,狀態跨 app 重啟保留。

**Architecture:** 純 mori-desktop 端工作,不動 AgentPulse(cwd 它已經在 `/sessions` 帶過來)。狀態用 append-only JSONL(`~/.mori/cue-state.jsonl`)持久化,跟 BI-2 `permission-audit.jsonl` 同根、同模式;讀的時候 replay 整個 log,**last-action-wins per event_id**,得到 `Map<event_id, CueState>`。模型放在 `mori_core::body::cue_state`(純函式 + tempfile 測試),Tauri shim 在 `crates/mori-tauri/src/cue_state.rs`。「jump」走 `action_skills::platform::open_url` 把 cwd 當路徑丟給 xdg-open / ShellExecuteExW,跟現成的 `open_profile_dir` 一樣。

**Tech Stack:** Rust(mori-core / mori-tauri)+ React + TypeScript(PulseTab),Tauri v2,serde JSON。

---

## 為什麼這樣分

| 檔 | 責任 |
|---|---|
| `crates/mori-core/src/body/cue_state.rs`(新)| `CueAction` enum + `CueStateEntry` record + `append_state` + `read_state_map`(replay JSONL, last-action-wins)+ 純函式 `is_snooze_active`。**沒有平台依賴**,可單元測。 |
| `crates/mori-core/src/body/mod.rs`(改)| `pub mod cue_state;` + re-export |
| `crates/mori-tauri/src/cue_state.rs`(新)| 薄 shim:`state_path()` = `mori_dir().join("cue-state.jsonl")`、`append_now()` 把 timestamp 生出來丟 mori-core |
| `crates/mori-tauri/src/main.rs`(改)| `mod cue_state;` 註冊;3 個 Tauri command:`cue_state_list` / `cue_state_set` / `cue_open_path`;塞進 `invoke_handler!` |
| `src/tabs/PulseTab.tsx`(改)| `SessionInfo` 加 `cwd?`;新 state hook 載 cue state map;render 過濾 dismissed / snoozed-active;每筆 cue 加 action row;30 秒 tick 復活 expired snooze |
| `src/i18n/locales/{en,zh-TW}.json`(改)| `pulse_tab.cue_ack` / `cue_snooze` / `cue_snooze_5m` / `cue_snooze_15m` / `cue_snooze_1h` / `cue_jump` / `cue_dismiss` / `cue_acked` / `cue_no_cwd` |

跟 BI-1 / BI-2 一樣 — 純邏輯放 mori-core 可單元測,Tauri shim 只做 path / timestamp / 平台 binding。**沒有新 crate**,沒有新依賴。

---

### Task 1: Cue state model in mori-core

**Files:**
- Create: `crates/mori-core/src/body/cue_state.rs`
- Modify: `crates/mori-core/src/body/mod.rs`

- [ ] **Step 1: Write the failing tests**

`crates/mori-core/src/body/cue_state.rs`:

```rust
//! BI-4 Cue Center 狀態 — append-only JSONL,replay 後 last-action-wins per event_id。
//! 路徑 `~/.mori/cue-state.jsonl`(由 mori-tauri 決定);本檔只接 &Path。
//! 跟 [`permission_audit`] 同 pattern,但這裡讀的是「最後狀態」,不是 tail。

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

/// `until` (RFC3339) 跟 `now` (RFC3339) 比,still snoozed = until > now。parse 失敗 → false(視同 expired)。
pub fn is_snooze_active(until: &str, now: &str) -> bool {
    use chrono::DateTime;
    match (DateTime::parse_from_rfc3339(until), DateTime::parse_from_rfc3339(now)) {
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
                CueAction::Snooze { until: "2026-05-28T11:00:00+08:00".into() },
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
            &entry("evt-snooze", CueAction::Snooze { until: until.clone() }),
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
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
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
```

- [ ] **Step 2: Wire mod.rs**

`crates/mori-core/src/body/mod.rs` 加在現有 `pub mod permission_audit;` 之後:

```rust
pub mod cue_state;

pub use cue_state::{
    append_state, is_snooze_active, read_state_map, CueAction, CueStateEntry,
};
```

`chrono` 已經是 mori-core 既有 dep,不用動 Cargo.toml(實際打開 `crates/mori-core/Cargo.toml` 確認;若沒有就加 `chrono = { workspace = true }`,跟其他 body/ 檔對齊)。

- [ ] **Step 3: Run tests to verify they fail and then pass**

```bash
cd ~/mori-universe/mori-desktop
cargo test -p mori-core --lib body::cue_state -- --nocapture
```

Expected:第一次跑寫完 step 1 + 2 都會 PASS(因為 step 1 已把 impl + tests 一次寫進去)。如果 chrono 沒在 mori-core,parse_from_rfc3339 編譯失敗 → 加 dep 再跑。

- [ ] **Step 4: Commit**

```bash
git checkout -b feat/bi-4-cue-center
git add crates/mori-core/src/body/cue_state.rs crates/mori-core/src/body/mod.rs
git commit -m "$(cat <<'EOF'
feat(bi-4): cue state model — append JSONL + replay last-action-wins

Pure logic in mori-core::body::cue_state. Mirrors permission_audit
pattern; reader returns Map<event_id, CueAction> instead of tail.
CueAction = Ack | Snooze { until } | Dismiss.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Tauri commands + opener wrapper

**Files:**
- Create: `crates/mori-tauri/src/cue_state.rs`
- Modify: `crates/mori-tauri/src/main.rs` (module decl + 3 commands + invoke_handler)

- [ ] **Step 1: Write the shim**

`crates/mori-tauri/src/cue_state.rs`:

```rust
//! BI-4:Cue state 的 mori-tauri 薄 shim。路徑決定 + RFC3339 timestamp 生成 + open path 包裝。

use mori_core::body::{append_state, read_state_map, CueAction, CueStateEntry};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// `~/.mori/cue-state.jsonl` — 跟 BI-2 audit log 同根。
pub fn state_path() -> PathBuf {
    crate::mori_dir().join("cue-state.jsonl")
}

/// 寫一筆 cue action(對指定路徑,可測)。
pub fn append_at(path: &Path, event_id: &str, action: CueAction, now: &str) -> Result<(), String> {
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
    fn append_at_then_list_roundtrips() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("cue-state.jsonl");
        append_at(&path, "evt-1", CueAction::Ack, "2026-05-28T10:00:00+08:00").unwrap();
        let map = read_state_map(&path);
        assert_eq!(map.get("evt-1"), Some(&CueAction::Ack));
    }

    #[test]
    fn append_at_writes_to_provided_path_not_home() {
        // 確保 shim 沒走 state_path()。
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("custom.jsonl");
        append_at(&path, "evt-x", CueAction::Dismiss, "2026-05-28T10:00:00+08:00").unwrap();
        assert!(path.exists());
    }
}
```

- [ ] **Step 2: Register module + commands**

`crates/mori-tauri/src/main.rs` — 在 `mod permission_broker;` 那一行附近加:

```rust
mod cue_state;
```

然後在 `permission_policy_list` 後面(約 line 2246)加 3 個 command:

```rust
/// BI-4:列出 cue 狀態 map(event_id → 最後 action)。唯讀。
#[tauri::command]
fn cue_state_list() -> std::collections::HashMap<String, mori_core::body::CueAction> {
    crate::cue_state::list()
}

/// BI-4:寫一筆 cue action。`snooze_until` 只在 action="snooze" 時才被讀。
#[tauri::command]
fn cue_state_set(
    event_id: String,
    action: String,
    snooze_until: Option<String>,
) -> Result<(), String> {
    let act = match action.as_str() {
        "ack" => mori_core::body::CueAction::Ack,
        "dismiss" => mori_core::body::CueAction::Dismiss,
        "snooze" => {
            let until = snooze_until.ok_or_else(|| "snooze requires snooze_until".to_string())?;
            mori_core::body::CueAction::Snooze { until }
        }
        other => return Err(format!("unknown action: {other}")),
    };
    crate::cue_state::append_now(&event_id, act)
}

/// BI-4:把 session cwd 丟給系統開檔器(jump action)。
/// 走 action_skills::platform::open_url(同一份 xdg-open / ShellExecuteExW 實作),
/// 跟既有 `open_profile_dir` 一樣 — 對目錄就會開 file manager。
#[tauri::command]
fn cue_open_path(path: String) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("empty path".to_string());
    }
    crate::action_skills::open_url_for_quickstart(trimmed)
        .map_err(|e| format!("open {trimmed}: {e}"))
}
```

- [ ] **Step 3: Add to invoke_handler**

在 `permission_policy_list,` 後面(約 line 6347)插:

```rust
            cue_state_list,
            cue_state_set,
            cue_open_path,
```

- [ ] **Step 4: Verify**

```bash
cd ~/mori-universe/mori-desktop
cargo test -p mori-core --lib body::cue_state -- --nocapture
cargo check -p mori-tauri --all-targets
```

Expected:全綠。`cargo test -p mori-tauri` 不必跑(很慢),但 `cargo check` 要過。

- [ ] **Step 5: Commit**

```bash
git add crates/mori-tauri/src/cue_state.rs crates/mori-tauri/src/main.rs
git commit -m "$(cat <<'EOF'
feat(bi-4): cue_state_list / cue_state_set / cue_open_path commands

Tauri shim wires mori_core cue_state to ~/.mori/cue-state.jsonl.
cue_open_path reuses action_skills platform::open_url so jump works
on Linux (xdg-open) + Windows (ShellExecuteExW).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: PulseTab — load state + filter dismissed/snoozed

**Files:**
- Modify: `src/tabs/PulseTab.tsx`

- [ ] **Step 1: Extend TS types + add state hook**

在 PulseTab.tsx 頂部 type 區把 `SessionInfo` 加 cwd(AgentPulse 已經送了),並新增 cue state 型別:

```typescript
interface SessionInfo {
  id: string; provider: string; state: string; project_name: string;
  cwd?: string | null;
  is_active: boolean; formatted_time: string;
}

// last-action-wins 的本地 state map(由 cue_state_list 回填,SSE 不帶這個)。
type CueAction =
  | { kind: "ack" }
  | { kind: "snooze"; until: string }
  | { kind: "dismiss" };
type CueStateMap = Record<string, CueAction>;

// effective 狀態(combine cue 本身 + state map + now)。
type Effective = "unread" | "acked" | "snoozed" | "dismissed";
```

在 component body 內,跟 `cues` state 同層加:

```typescript
const [cueState, setCueState] = useState<CueStateMap>({});
const [now, setNow] = useState<string>(() => new Date().toISOString());

const reloadCueState = async () => {
  try {
    const m = await invoke<CueStateMap>("cue_state_list");
    setCueState(m ?? {});
  } catch { /* 缺檔 → 空 map,不擾使用者 */ }
};

useEffect(() => { reloadCueState(); }, []);

// 每 30 秒前進一次 now,把過期的 snooze 自動復活(re-render)。
useEffect(() => {
  const id = setInterval(() => setNow(new Date().toISOString()), 30_000);
  return () => clearInterval(id);
}, []);
```

- [ ] **Step 2: Effective-state helper + filter**

在 component function 外加 pure helper:

```typescript
function effectiveState(cueId: string, state: CueStateMap, nowIso: string): Effective {
  const a = state[cueId];
  if (!a) return "unread";
  if (a.kind === "ack") return "acked";
  if (a.kind === "dismiss") return "dismissed";
  // snooze:until > now → snoozed;否則回到 unread(過期復活)
  if (a.kind === "snooze") return a.until > nowIso ? "snoozed" : "unread";
  return "unread";
}
```

> ⚠️ ISO-8601 字串字典序比較對 UTC `Z` / 數字偏移**部分情境**會出錯。簡化處理:存進去前一律 `new Date(...).toISOString()` → 統一帶 `Z`,前端只比帶 `Z` 的字串。Tauri 端 `chrono::Utc::now().to_rfc3339()` 出來是 `+00:00`,要在 TS 比之前 `new Date(s).getTime() > new Date(nowIso).getTime()`。**改成**:

```typescript
function effectiveState(cueId: string, state: CueStateMap, nowIso: string): Effective {
  const a = state[cueId];
  if (!a) return "unread";
  if (a.kind === "ack") return "acked";
  if (a.kind === "dismiss") return "dismissed";
  if (a.kind === "snooze") {
    return new Date(a.until).getTime() > new Date(nowIso).getTime() ? "snoozed" : "unread";
  }
  return "unread";
}
```

改 cue render(原本是 `cues.map((c) => ...)`)— filter 掉 dismissed / snoozed,並把 effective 傳下去:

```tsx
<div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
  {cues
    .map((c) => ({ c, eff: effectiveState(c.event_id, cueState, now) }))
    .filter(({ eff }) => eff !== "dismissed" && eff !== "snoozed")
    .map(({ c, eff }) => (
      <CueRow
        key={c.event_id}
        cue={c}
        effective={eff}
        session={sessions.find((s) => s.id === c.session_id) ?? null}
        onChanged={reloadCueState}
      />
    ))}
</div>
```

(`CueRow` 在 Task 4 寫。先用 placeholder:)

```tsx
function CueRow(
  { cue, effective }: { cue: Cue; effective: Effective; session: SessionInfo | null; onChanged: () => void }
) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13, opacity: effective === "acked" ? 0.5 : 1 }}>
      <CueBadge type={cue.type} />
      <span>{cue.summary}</span>
      <span style={{ fontSize: 10, opacity: 0.4, marginLeft: "auto" }}>{cue.time}</span>
    </div>
  );
}
```

- [ ] **Step 3: Manual smoke test(可選,但 task 4 之前先確認沒崩)**

```bash
cd ~/mori-universe/mori-desktop
npm run build
```

Expected:build pass,沒有 TS error。

- [ ] **Step 4: Commit**

```bash
git add src/tabs/PulseTab.tsx
git commit -m "$(cat <<'EOF'
feat(bi-4): PulseTab loads cue_state map + filters dismissed/snoozed

Extend SessionInfo with optional cwd (AgentPulse already sends it).
30s tick re-evaluates expired snoozes. Action row placeholder for
Task 4.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: PulseTab — action buttons + i18n

**Files:**
- Modify: `src/tabs/PulseTab.tsx` (CueRow 改成完整版)
- Modify: `src/i18n/locales/en.json`
- Modify: `src/i18n/locales/zh-TW.json`

- [ ] **Step 1: i18n keys**

`src/i18n/locales/zh-TW.json` — 在現有 `pulse_tab` block 結尾(`"cue_done": "✓ 跑完"` 後面)加:

```json
    ,"cue_ack": "已看",
    "cue_snooze": "稍後",
    "cue_snooze_5m": "5 分後",
    "cue_snooze_15m": "15 分後",
    "cue_snooze_1h": "1 小時後",
    "cue_jump": "開資料夾",
    "cue_dismiss": "移除",
    "cue_acked": "(已看)",
    "cue_no_cwd": "找不到資料夾"
```

`src/i18n/locales/en.json` 同位置加:

```json
    ,"cue_ack": "Ack",
    "cue_snooze": "Snooze",
    "cue_snooze_5m": "5 min",
    "cue_snooze_15m": "15 min",
    "cue_snooze_1h": "1 hour",
    "cue_jump": "Open folder",
    "cue_dismiss": "Dismiss",
    "cue_acked": "(acked)",
    "cue_no_cwd": "No cwd"
```

- [ ] **Step 2: Write the real CueRow**

把 Task 3 的 placeholder `CueRow` 換成完整版(放 PulseTab.tsx 檔尾,跟 `StateBadge` / `CueBadge` 同層):

```tsx
function CueRow(
  { cue, effective, session, onChanged }:
    { cue: Cue; effective: Effective; session: SessionInfo | null; onChanged: () => void }
) {
  const { t } = useTranslation();
  const cwd = session?.cwd ?? null;

  const set = async (action: "ack" | "dismiss" | "snooze", snooze_until?: string) => {
    try {
      await invoke("cue_state_set", { eventId: cue.event_id, action, snoozeUntil: snooze_until ?? null });
      onChanged();
    } catch { /* 寫失敗忽略(audit 落地由 mori-tauri Err 處理,UI 不卡) */ }
  };
  const snooze = (mins: number) =>
    set("snooze", new Date(Date.now() + mins * 60_000).toISOString());

  const jump = async () => {
    if (!cwd) return;
    try { await invoke("cue_open_path", { path: cwd }); } catch { /* 開不到不擾 */ }
  };

  const acked = effective === "acked";

  return (
    <div
      style={{
        display: "flex", alignItems: "center", gap: 8, fontSize: 13,
        opacity: acked ? 0.55 : 1,
        border: "1px solid var(--c-border)", borderRadius: 6, padding: "6px 8px",
      }}
    >
      <CueBadge type={cue.type} />
      <span>{cue.summary}</span>
      {acked && (
        <span className="mori-pill-badge tone-neutral" style={{ fontSize: 10 }}>
          {t("pulse_tab.cue_acked")}
        </span>
      )}
      <span style={{ fontSize: 10, opacity: 0.4, marginLeft: "auto" }}>{cue.time}</span>

      <div style={{ display: "flex", gap: 4 }}>
        {!acked && (
          <button className="mori-btn small ghost" onClick={() => set("ack")}>
            {t("pulse_tab.cue_ack")}
          </button>
        )}
        {!acked && (
          <SnoozeMenu
            label={t("pulse_tab.cue_snooze")}
            options={[
              { mins: 5, label: t("pulse_tab.cue_snooze_5m") },
              { mins: 15, label: t("pulse_tab.cue_snooze_15m") },
              { mins: 60, label: t("pulse_tab.cue_snooze_1h") },
            ]}
            onPick={snooze}
          />
        )}
        <button
          className="mori-btn small ghost"
          disabled={!cwd}
          title={cwd ?? t("pulse_tab.cue_no_cwd")}
          onClick={jump}
        >
          {t("pulse_tab.cue_jump")}
        </button>
        <button className="mori-btn small ghost" onClick={() => set("dismiss")}>
          {t("pulse_tab.cue_dismiss")}
        </button>
      </div>
    </div>
  );
}

function SnoozeMenu(
  { label, options, onPick }:
    { label: string; options: { mins: number; label: string }[]; onPick: (mins: number) => void }
) {
  const [open, setOpen] = useState(false);
  return (
    <span style={{ position: "relative" }}>
      <button className="mori-btn small ghost" onClick={() => setOpen((v) => !v)}>
        {label}
      </button>
      {open && (
        <div
          style={{
            position: "absolute", right: 0, top: "100%", marginTop: 4,
            background: "var(--c-surface)", border: "1px solid var(--c-border)",
            borderRadius: 6, padding: 4, zIndex: 10,
            display: "flex", flexDirection: "column", gap: 2, minWidth: 96,
          }}
        >
          {options.map((o) => (
            <button
              key={o.mins}
              className="mori-btn small ghost"
              style={{ justifyContent: "flex-start" }}
              onClick={() => { onPick(o.mins); setOpen(false); }}
            >
              {o.label}
            </button>
          ))}
        </div>
      )}
    </span>
  );
}
```

> 🎨 配色一律走 `var(--c-*)` token(`--c-border` / `--c-surface`),按鈕用既有 `.mori-btn small ghost`,徽章用 `.mori-pill-badge`。不寫死 rgba / hex。

⚠️ Tauri command arg naming:Tauri v2 自動把 Rust snake_case 轉成 JS camelCase。`event_id` → `eventId`、`snooze_until` → `snoozeUntil`。上面 invoke 已用 camelCase。

- [ ] **Step 3: TypeScript build**

```bash
cd ~/mori-universe/mori-desktop
npm run build
```

Expected:build pass。如果 `var(--c-surface)` 不存在,直接改 `var(--c-bg)`(看 shell.css `:root` 實際定義)。

- [ ] **Step 4: Commit**

```bash
git add src/tabs/PulseTab.tsx src/i18n/locales/en.json src/i18n/locales/zh-TW.json
git commit -m "$(cat <<'EOF'
feat(bi-4): cue action row — ack / snooze (5m/15m/1h) / jump / dismiss

Ack mutes the cue (stays in log). Snooze hides until expiry then
revives. Jump opens session cwd via xdg-open / ShellExecuteExW.
Dismiss removes from view. All persisted via cue_state_set.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Verify + update backlog doc + memory

**Files:**
- Modify: `docs/body-interface-backlog.md` (mark BI-4 done)

- [ ] **Step 1: Run shared verification**

```bash
cd ~/mori-universe/mori-desktop
bash scripts/verify.sh
```

Expected:`npm run build` + `cargo test -p mori-core --lib` + `cargo check --workspace --all-targets` 全綠。任何紅燈都修到綠再下一步。

- [ ] **Step 2: Manual smoke (yazelin)**

跑 `npm run dev` + AgentPulse 一起開,用一個 Codex / Claude session 觸發 `waiting_input` / `done`,逐項點:
- ack → cue 變淡 + 顯示「已看」
- snooze 5m → cue 從列表消失;`~/.mori/cue-state.jsonl` 有對應 entry
- jump → 開檔案管理器到 session cwd
- dismiss → cue 消失,重啟 app 仍消失

⚠️ **動 mori-desktop Rust code 前先問 yazelin** — `npm run dev` rebuild 會切斷對話。這份 plan 本身只是文件,不會 trigger rebuild,但 Task 1-4 動 Rust + npm 都要 build。subagent 跑時若 yazelin 正在語音,**暫停等指示**。

- [ ] **Step 3: Update backlog doc**

`docs/body-interface-backlog.md` 約 L96(BI-4 那一行)+ L101(完成判準)— 加 BI-4 done 標記。

`docs/body-interface-backlog.md` table 第 96 行,把:

```
| **BI-4** | **Cue Center**:把 BI-3 的 cue surface 升級成可操作(acknowledge / snooze / jump / dismiss)| Cue lifecycle | BI-3 | mori-desktop | ✅ now |
```

不動(已經 ✅ now 表示「現在做」)。但在「各 stage 的完成判準」section(約 L103-108)中,**在 BI-3 done 那段後面**新增:

```markdown
- **BI-4 done** ✅(2026-05-28,branch `feat/bi-4-cue-center`)= PulseTab 每筆 cue 有 ack / snooze (5m/15m/1h) / jump / dismiss;狀態 append 到 `~/.mori/cue-state.jsonl`(replay last-action-wins per event_id);ack 淡顯仍在 log,snooze 過期自動復活,jump 走 `xdg-open`/`ShellExecuteExW` 開 session cwd,dismiss 消失。**未做**(刻意):cross-cue bulk ops、cue 歷史 tab(留給未來)。
```

並更新檔尾 `Last updated:` → `2026-05-28`、`Next action:` → `寫 BI-5 (Meeting Recorder) per-slice plan`。

- [ ] **Step 4: Commit + open PR**

```bash
git add docs/body-interface-backlog.md
git commit -m "$(cat <<'EOF'
docs(bi-4): mark Cue Center done

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"

git push -u origin feat/bi-4-cue-center
gh pr create --title "BI-4: Cue Center — operable cue surface" --body "$(cat <<'EOF'
## Summary
- Cue state model in `mori-core::body::cue_state` (append-only JSONL, replay last-action-wins).
- Tauri commands: `cue_state_list` / `cue_state_set` / `cue_open_path`.
- PulseTab per-cue actions: ack / snooze (5m/15m/1h) / jump (opens session cwd) / dismiss.
- Persists to `~/.mori/cue-state.jsonl` — survives restart.

## Test plan
- [ ] `bash scripts/verify.sh` green
- [ ] Manual: open AgentPulse + a Codex session → trigger waiting_input → ack / snooze / jump / dismiss each verified
- [ ] Restart app → dismissed/acked state preserved

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)" --label "trunk-based"
```

trunk-based 工作流:PR 開好直接設 auto-merge(`gh pr merge --auto --squash`),別 stack。

- [ ] **Step 5: Update memory**

`/home/ct/.claude/projects/-home-ct/memory/project_mori_body_interface.md`:把 `next=BI-4 Cue Center` 改成 `BI-0→BI-4 ✅ done(2026-05-28),next=BI-5 Meeting Recorder standalone`。

---

## Self-review

**Spec coverage:**
- BI-4 = ack / snooze / jump / dismiss ✓(Task 4)
- 跨 app 重啟保留 ✓(Task 1 JSONL + Task 3 載入)
- "Cue lifecycle" 契約 ✓(Effective 狀態 enum)
- "完成判準" doc 更新 ✓(Task 5)

**Placeholder scan:**
- 沒有 TBD / TODO
- 每個 step 都有 code 或 command
- 有一處「ISO 字串比較不對」我直接 inline 改成 `new Date().getTime()` 比

**Type consistency:**
- `CueAction` 在 mori-core(`#[serde(tag = "kind")]` Snake / Snooze 含 `until`)
- TS 端 `CueAction = { kind: "ack" } | { kind: "snooze"; until } | { kind: "dismiss" }` ← 跟 serde tag 對齊
- `cue_state_set` Tauri command args:`event_id` / `action` / `snooze_until` → JS 端 `eventId` / `action` / `snoozeUntil`(Tauri v2 自動轉)
- `Effective` 是 TS-only 衍生型,沒跨層
