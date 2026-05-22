# In-app Reminder Popup — Design Spec

**Status**: Approved (2026-05-22)
**Owner**: yazelin
**Source brainstorm**: 2026-05-22 session(skill_server + sticky 通知系列 follow-up)

## 1. 背景

時之鳥(`mori-time`)的 reminder fire 路徑目前依賴 `notify-rust` → libdbus → GNOME shell。Linux GNOME 環境下 trace 證實:即使 hint 完整(`urgency=Critical` + `resident=true` + `timeout=Never`),GNOME shell 在 30ms 內 emit `NotificationClosed reason=2` — banner 沒機會 surface,通知抽屜也沒入庫。根因是 mori-tauri 自身有 2 個 `_NET_WM_STATE=ABOVE` 視窗(`Mori (floating)` + `Mori (chat)`),GNOME 對「有 ABOVE 視窗的 app」直接抑制 banner。Critical urgency 在這條路徑被吃掉。

User 設的 reminder 沒人看得到,reminder 等於失效。OS notification 在 Linux 上脆弱性太多(per-DE / per-shell-extension / focus / fullscreen / ABOVE state),不能單押。

## 2. 目標

讓 reminder fire 時 user **一定看得到**,而且能 actionable(稍後 / 關閉)。同時保留 OS 通知作 backup(user 切到其他 app 時還能 surface),並提供診斷給 user 知道 OS 那條當下是否被 DE 抑制。

## 3. 非目標

- LLM skill 開放 cron reminder 設定(`remind_me_cron`)— 框架 popup 吃得下,但 user-facing 設定 path 不在此 MVP
- 通知歷史頁 `/reminders` history view — popup 折疊「+N 條歷史」link 預留入口,實際 history 頁 follow-up
- 評分按鈕(👍/👎/✏️)— Q2 follow-up,放在 voice-input chat panel,不在此 popup
- 跨 app / 跨 fullscreen 強推送(MVP 決定 only-in-Mori-app group)

## 4. 設計

### 4.1 架構與元件

**5 處修改**(2 新檔 + 3 既有檔改):

1. **`reminder_popup` Tauri window**(新)
   - `tauri.conf.json` 內 windows array 加 entry
   - 設定:`transparent: true`, `decorations: false`, `alwaysOnTop: true`, `skipTaskbar: true`, `visible: true`
   - 初始位置:`(-10000, -10000)` 離屏
   - 初始尺寸:`320×96`(被動 chip 為 `32×32`)

2. **`src/ReminderPopup.tsx`**(新)
   - 對齊 `ChatBubble.tsx` 既有 pattern(`emit`/`listen` 加位置同步)
   - listener:`reminder-fire-show`(新增 reminder push 進 queue) + `sprite-moved`(隨 sprite 拖動更新位置)
   - 啟動 mount 後 invoke `reminder_active_queue` 主動拉「fired 但未 dismissed」reminders
   - 內部 state:`queue: ActiveReminder[]` + `mode: 'popup' | 'chip'`

3. **`crates/mori-tauri/src/reminders_cmd.rs` 新 Tauri commands**(改既有檔)
   - `reminder_active_queue() -> Vec<ActiveReminder>` — 回傳「status=Fired 且 dismissed_at IS NULL 且 fired_at > now - 7d」的 list
   - `reminder_dismiss(id: i64)` — DB 寫 `dismissed_at = now`
   - `reminder_snooze(id: i64, minutes: u32)` — 走既有 `ReminderService::snooze_reminder`

4. **`crates/mori-time/src/commands.rs` `ReminderService::on_fire` callback 修改**(改既有檔)
   - 新增 `app_handle: AppHandle` 注入 `ReminderService::new` 簽名
   - on_fire 內除 spawn_blocking notifier.fire(),**並 emit `reminder-fire-show`** 帶 payload `{ id, text, due_at, fired_at }`
   - emit 失敗只 log warn,不擋 mark_fired

5. **DB schema migration**(改 `mori-time/src/schema.rs`)
   - reminders 表加 `dismissed_at TIMESTAMP NULL` 欄(default NULL)
   - SQLite 沒有 `ADD COLUMN IF NOT EXISTS`,migrate 用 `PRAGMA table_info(reminders)` 檢查欄是否存在,沒有才 `ALTER TABLE ADD COLUMN dismissed_at TEXT`(SQLite TIMESTAMP 實際存 ISO8601 text)
   - 對齊既有 migrate idempotent pattern

**Capabilities**:`crates/mori-tauri/capabilities/default.json` 內 `windows` array 加 `"reminder_popup"`,沿用既有 permissions(無需新增 permission key)。

**Settings**(在 Settings/Deps 頁加「通知」區段):

MVP 要實際出 UI 的:
- `popup_enabled: bool = true` — **MVP 含 toggle UI**(checkbox / switch)
- `os_notification_enabled: bool = true` — **MVP 含 toggle UI**

MVP 寫死 default 不出 UI 的(follow-up 加):
- `popup_position` MVP **只實作 `sprite-adjacent`**(其他選項 `top-center` / `top-right` / `bottom-right` follow-up,連 enum 本身都 MVP 不引入,避免假象有得選)
- `popup_auto_chip_minutes: u32 = 5`

Diagnostic chip(MVP 靜態說明):Settings 頁「桌面通知」toggle 旁附小字「⚠ 在 Linux GNOME 上可能被 shell 抑制,建議 in-app popup 保持開啟」。**主動偵測** follow-up

**Edge case**:user 把 popup + OS 兩個 toggle 都關掉 → reminder fire 後 silent mark_fired,user 收不到任何通知。**這是 user 的選擇,不做強制至少留一個** — 但 Settings UI 兩個 toggle 都 off 時 inline 顯示警告「⚠ 你關掉了所有通知,reminder fire 不會主動告訴你」

### 4.2 Data flow

```
ReminderService::on_fire (mori-time/commands.rs)
   ├─ spawn_blocking → notifier.fire()         ← OS 通知(可 toggle 關)
   └─ app_handle.emit("reminder-fire-show", payload)

ReminderPopup.tsx
   listen("reminder-fire-show") → debounce 200ms → setQueue(prev => [...prev, new])
   useEffect on queue/mode → resize + reposition + show
```

**Mount 時 ready 競態解法**:popup window React 元件 mount 完 → invoke `reminder_active_queue` 主動拉資料(對 Rust 端是 idempotent query,可隨時呼叫)。這條 path **同時 cover 三種情境**:
- mori-tauri 啟動 reload-pending 過早 emit、popup webview 還沒 ready → mount 後 query 補上
- mori-tauri 重啟,user 重看「上次未 dismiss」reminder
- popup webview 自己 crash 重啟

### 4.3 Catch-up 折疊行為

- mori-tauri restart → `ReminderService::new` 既有 reload-pending logic 不動
- overdue reminder(delta ≤ 0)依 due_at 時間順序排,1ms 內各自觸發 on_fire callback,各自 emit
- popup 200ms debounce 把多條 emit 合成一次 render
- popup queue ≥ 2 條 → 折疊 list view,每筆一行(text + 「原定 HH:MM」chip + [稍後] [關閉]),最多 5 筆
- > 5 條 → 顯示「+N 條歷史提醒」chip(MVP 不可互動,只是視覺指示 history 頁 follow-up 才開放)
- **超過 grace 7 天**的 overdue reminder:reload-pending 時 silently 寫 `dismissed_at = fired_at`,不 emit,不 popup,user 不會被 7+ 天前的 spam 淹

### 4.4 Active queue & Mode 切換

Tauri 預設 serde 不做 case 轉換,Rust struct 用 `serde(rename_all = "camelCase")` 一致對外 → TS 收到 camelCase:

```rust
// crates/mori-tauri/src/reminders_cmd.rs
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveReminder {
    pub id: i64,
    pub text: String,
    pub due_at: String,    // ISO8601
    pub fired_at: String,
}
```

```ts
// src/ReminderPopup.tsx
type ActiveReminder = { id: number; text: string; dueAt: string; firedAt: string }
const [queue, setQueue] = useState<ActiveReminder[]>([])
const [mode, setMode] = useState<'popup' | 'chip'>('popup')
```

`emit("reminder-fire-show", ...)` 那筆 payload 同此 shape。

- `queue.length === 0` → setSize(1, 1) + 移 off-screen(完全 dismiss 過渡)
- `queue.length > 0` 且 `mode === 'popup'` → setSize(320, dynamic) + 貼 sprite 顯示
- `mode === 'chip'` → setSize(32, 32) + sprite 旁固定 anchor + render 小 badge「N」
- `popup_auto_chip_minutes` 計時內無互動 → setMode('chip');chip 點一下 → setMode('popup')
- chip 留到 user 處理完(無限,不要違背「不能錯過」初衷)

### 4.5 動作

**[稍後 5 分]**(snooze button):
- invoke `reminder_snooze({id, minutes: 5})` → 既有 `ReminderService::snooze_reminder` 走完(更新 due_at + 重排 scheduler job)
- popup queue 內這筆移除
- 5 分鐘後 scheduler 重新 fire → 走相同 emit path 再進 queue

**[關閉]**(dismiss button):
- invoke `reminder_dismiss({id})` → DB 寫 `dismissed_at = now`
- popup queue 內這筆移除
- queue 空 → setSize(1,1) + 移 off-screen

**錯誤路徑**:command 回 `Result<(), String> = Err(e)` → popup inline 紅字 chip「動作失敗,reminder 仍在」,不從 queue 移除,user 重試

### 4.6 視窗位置策略

- 沿用 `ChatBubble.tsx` 同套機制:Rust 端拿 sprite 位置算 absolute logical position,透過 event 帶給 popup
- popup `setPosition(spritePos.x, spritePos.y + spriteHeight + gap)` — 貼 sprite 下方
- sprite 拖動 → emit `sprite-moved` → popup 跟著 reposition
- hide 雙保險:`setPosition(-10000, -10000)` + `setSize(1, 1)`(Wayland mutter 對 transparent+decorationless window setPosition 偶爾沒成功,1×1 確保 click 不擋)

## 5. Error handling

| 情境 | 處理 |
|---|---|
| popup webview 還沒 mount,emit 來了 | `reminder_active_queue` query 補抓未 dismissed |
| AppHandle 拿不到(理論不會發生 — 構造時注入) | log error,reminder 走純 OS path,不擋 mark_fired |
| Snooze / dismiss command 失敗 | popup 顯示 inline 錯誤 chip,reminder 留 queue |
| Catch-up storm > 5 條 | 折疊 list + 「+N 條」link |
| Overdue > 7 天 | 自動 mark dismissed,不 emit |
| Settings 讀檔失敗 | 用 defaults,log warn |

## 6. Testing

**Unit / integration**(`cargo test -p mori-time --lib` / `cargo test -p mori-tauri --lib` 必過):

- `mori-time/commands.rs::on_fire_emits_event` — mock AppHandle 收 event,assert payload 對
- `mori-time/schema.rs::migration_adds_dismissed_at_idempotent` — 新欄重複 migrate 不爆
- `mori-tauri/reminders_cmd.rs::reminder_dismiss_writes_dismissed_at`
- `mori-tauri/reminders_cmd.rs::reminder_active_queue_filters_dismissed_and_super_overdue`
- `mori-tauri/reminders_cmd.rs::reminder_snooze_reschedules` — 用 in-memory store
- popup queue dedup + 折疊 TS test(`src/ReminderPopup.test.tsx`)

**Manual smoke**(寫進 PR description checklist):
- [ ] 設「1 分鐘後」→ fire 時 popup 出現貼 sprite + 不自動消失(5 分內可手動 dismiss)
- [ ] 點 [稍後 5 分] → popup 消 + 5 分後重彈
- [ ] 點 [關閉] → popup 消 + DB `dismissed_at` 寫入
- [ ] 5 分無互動 → popup 縮 chip,點 chip 拉回完整 popup
- [ ] 重啟 mori-tauri → 未 dismiss reminder 重新進 queue + popup
- [ ] 連設 3 條 5 分鐘內 + 重啟 → popup 折疊顯示 3 筆
- [ ] OS notification toggle off → fire 時只 popup 不發 OS
- [ ] popup toggle off → fire 時只發 OS,popup 視窗保持離屏不彈
- [ ] popup + OS 都 off → Settings UI 顯示警告 chip,fire 後無任何通知(預期行為)
- [ ] DB 塞 1 筆 8 天前 overdue → 重啟 reload-pending 不 popup + 自動 mark dismissed

## 7. 開放決定 / Follow-up

- **Settings 完整 UI**:位置 dropdown(`popup_position`)/ chip 自動退場分鐘輸入框(`popup_auto_chip_minutes`)。MVP 兩個核心 toggle 出 UI,其他預設寫死
- **History 頁 `/reminders`**:列所有 fired / dismissed / cancelled reminder
- **LLM skill `remind_me_cron`**:NL → cron 表達式 parser
- **OS 通知健康診斷主動偵測**:listen `org.freedesktop.Notifications.NotificationClosed`,reason=2 在 fire 後 < 100ms 內收到 N 次 → 標「被抑制」,Settings 頁 chip 變紅。MVP 不做,先顯示靜態警示文字
- **評分按鈕(Q2 follow-up)**:位置在 voice-input chat panel / 歷史紀錄 copy 按鈕旁,獨立 brainstorm

## 8. 變更影響

- DB schema:`reminders` 表加 `dismissed_at` 欄(idempotent migration)
- Rust API:`ReminderService::new` 簽名加 `app_handle: AppHandle` 參數 — caller(`main.rs` 5265 行附近的 init)要跟著改
- Tauri config:`tauri.conf.json` 加 window,`capabilities/default.json` 加 window 名
- 既有 `notifier.fire` / sticky / spawn_blocking 修法**不動**(那條 OS 通知保留為 backup,toggle 可關)
