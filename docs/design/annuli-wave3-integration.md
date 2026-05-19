# Annuli Wave 3 整合計劃 — mori-desktop 端

> 本文件記錄 mori-desktop ↔ annuli HTTP 整合在「**annuli wave 2 完成、mori-desktop 連線啟用**」當天該怎麼動。
> **mori-desktop 端的 HTTP client 已經完成**(v0.5+ Wave 4 客戶端框架建好了)。卡關不在我們這邊,卡在 annuli wave 2 重構。
> 對應位置:[annuli-memory.md](annuli-memory.md) 是設計,本文是 **執行 checklist**。

---

## TL;DR — 現況

| 元件 | 狀態 | 位置 |
|---|---|---|
| `AnnuliClient`(reqwest 包裝)| ✅ done | `crates/mori-core/src/annuli/client.rs` |
| `AnnuliMemoryStore`(impl `MemoryStore`)| ✅ done | `crates/mori-core/src/memory/annuli.rs` |
| `annuli_supervisor`(spawn python child)| ✅ done | `crates/mori-tauri/src/annuli_supervisor.rs` |
| Hot-reload IPC(`annuli_reload`)| ✅ done | `crates/mori-tauri/src/main.rs:1725` |
| Config UI(endpoint / spirit / token / basic_auth)| ✅ done | ConfigTab > Annuli subtab |
| AnnuliTab 唯讀 browser(SOUL / memory / events)| ✅ done | `src/tabs/AnnuliTab.tsx` |
| **annuli wave 2 service** | 🚧 in progress | `yazelin/annuli` repo `refactor/split-core-creator` branch |

mori-desktop 端基本上沒事可做了 — wave 2 服務 ship、`annuli.enabled = true` 就會走 HTTP。

---

## 9 個 endpoint 連線清單

| Method | Path | Client method | mori-desktop 用途 |
|---|---|---|---|
| GET    | `/health` | `health()` | startup ping、AnnuliTab 連線狀態 |
| GET    | `/spirits/<x>/soul` | `get_soul()` | AnnuliTab SOUL.md 預覽 / memory 注入 |
| PUT    | `/spirits/<x>/soul` | `put_soul()` | (Wave 5+ UI 編輯,需 `X-Soul-Token`) |
| POST   | `/spirits/<x>/events` | `append_event()` | 對話 / 動作完成後 fire-and-forget |
| GET    | `/spirits/<x>/events?...` | `list_events_*` | AnnuliTab 今日 events |
| POST   | `/spirits/<x>/rings/new` | `trigger_sleep()` | Ctrl+Alt+Z / `/sleep` 寫年輪 |
| POST   | `/spirits/<x>/curator/dry-run` | `curator_dry_run()` | (Wave 5+ 整理 memory 預覽)|
| POST   | `/spirits/<x>/curator/apply` | `curator_apply()` | (Wave 5+ apply 整理) |
| POST   | `/spirits/<x>/bootstrap` | `bootstrap()` | 首次啟用,scaffold vault |

---

## Wave 3 ship day checklist(mori-desktop 端)

**前提**:annuli wave 2 部署到 `localhost:5000` 並 `/health` 回 200。

1. **本機驗 client connectivity**
   - Config tab > Annuli subtab → 填 endpoint `http://localhost:5000`,spirit `mori`,user_id 你的名字。
   - 存後 backend 自動 `annuli_reload` → log 應有 `annuli hot-reload → AnnuliMemoryStore`。
   - AnnuliTab → 看 status bar 顯示「**connected**」。

2. **驗 memory store 走 HTTP**
   - Mori 對話時觸發 `remember` skill(例「記住我喜歡黑咖啡」)。
   - 預期:annuli vault `~/mori-universe/spirits/mori/memories/MEMORY.md` 多一行;**不**寫 `~/.mori/memory/` 本機 fallback。
   - 驗證:`ls -la ~/.mori/memory/` 應該還是舊內容,新 memory 在 vault。

3. **驗事件 stream**
   - 對話一兩輪 → AnnuliTab 「今日 events」應顯示 user / assistant pair。
   - 不顯示 = annuli POST `/events` 失敗(看 logs `kind=annuli_post_failed`)。

4. **驗 `/sleep` 寫年輪**
   - 按 Ctrl+Alt+Z 或 AnnuliTab 「/sleep」按鈕。
   - 預期:annuli 跑 reflection LLM 完成,vault `rings/<date>.md` 出現新 ring。

5. **驗 startup 自動 spawn annuli supervisor**(可選 — annuli 跑獨立 process 時跳過)
   - `annuli.supervisor.enabled = true` + `command = "python ~/mori-universe/annuli/main.py"`。
   - mori-tauri 啟動時 spawn,退出時 kill_on_drop。

---

## 預期 / 已知問題

### 1. CORS

Annuli wave 2 Flask 預設只接 `localhost` origin。mori-tauri 內 webview 走 `http://localhost:1420`(dev)或 `tauri://localhost`(prod)。**若連不上 → 先看 CORS headers**。

修法:annuli `app.py` 加 `flask-cors`,允許 `tauri://*` + `http://localhost:*` origin。

### 2. SOUL.md / MEMORY.md 寫入權限邊界

CLAUDE.md 硬規則:**`identity/SOUL.md` 跟 `memories/MEMORY.md` 禁止 ghost-write,要寫先 explicit re-authorize**。client side 對應實作:

- `put_soul()` 要 `X-Soul-Token` header,token 由 user 在 Config 手填(不從 keyring 自動拿)。
- `AnnuliMemoryStore.write()` 寫的是 **memory items**(`memories/items/*.md`),不寫 MEMORY.md index。Index 由 annuli 端 curator 維護。

實作這條:確認 `AnnuliMemoryStore::write` 對應的 annuli endpoint 是 `POST /memory/items` 不是 `PUT /memory/index`。

### 3. Vault 結構初始化

首次啟用 `annuli.enabled = true` 時,vault 可能還沒結構。`AnnuliClient::bootstrap()` 對應 annuli `POST /spirits/<x>/bootstrap`,該 endpoint 應 scaffold:

```
~/mori-universe/spirits/<spirit_name>/
├── identity/
│   └── SOUL.md       ← 空模板
├── memories/
│   ├── MEMORY.md     ← 空 index
│   └── items/        ← 空 dir
├── events/           ← 空 dir
└── rings/            ← 空 dir
```

mori-desktop 端 ship day 時,Config tab 「Initialize vault」按鈕(現在沒做)應該 call `bootstrap`。**TODO** — Wave 3 ship day 加這顆按鈕。

### 4. 異步失敗的 fallback

當前 `event_log::append` 對 annuli POST 失敗只 log warn 不掛 UI 提示。建議 Wave 3 連線後加:

- 連續 N 次 POST `/events` 失敗 → emit `annuli-degraded` event 給 ChatPanel topbar 紅 chip 警告。
- 用戶按 chip → AnnuliTab + 顯示診斷。

實作位置:`crates/mori-tauri/src/main.rs:3155` 那批 annuli `append_event` call 後加失敗 counter。

---

## 對 spirit vault 的承諾

mori-desktop 對 annuli 服務的承諾(寫進 README + CLAUDE.md):

1. **vault 永遠是 user 的**。mori-desktop 只透過 annuli 讀寫,不直接 grep vault 檔。
2. **identity / memories 禁止 ghost-write**(只能透過明示 skill,例 `remember`)。
3. **vault path** 由 annuli 服務的 config 決定,**不**在 mori-desktop 端 hardcode。

---

## 不在 Wave 3 範圍

- 跨 spirit switcher(Wave 5+;UI 加 spirit picker dropdown)
- In-UI 編輯 SOUL.md(Wave 5+;需要 inline markdown editor)
- Curator dry-run / apply UI(Wave 5+;需 diff viewer component)
- Vault 多裝置 sync(Wave 6+;需 annuli 端加 sync protocol)
- Web access 到 vault(Wave 7+;world-tree 公開 lore 反而是 Quartz static site)

---

## 相關文件

- [annuli-memory.md](annuli-memory.md) — Annuli 整體設計
- [architecture.md](../architecture.md) — 森林宇宙 4 層架構
- [`crates/mori-core/src/annuli/`](../../crates/mori-core/src/annuli/) — client 實作
- [world-tree ARCHITECTURE.md](https://github.com/yazelin/world-tree/blob/main/ARCHITECTURE.md) — 4-repo layout 規範
