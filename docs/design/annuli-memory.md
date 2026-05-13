# Annuli 架構設計 — vault-backed 反思服務

> Annuli 的角色定位 + 對接 world-tree spirit vault 的設計決策。
> **本文件描述計畫,不是已實作的事實。** 實作將在 `yazelin/Annuli` repo
> 內進行,本文件供 mori-desktop / world-tree / Annuli 三邊 design alignment。

---

## 核心定位:Annuli 是「**服務**」,不是「**儲存**」

之前討論時把 Annuli 看成「另一個記憶系統」,**這是錯的**。重新校準:

| 層 | 角色 | 形態 |
|---|---|---|
| **mori-journal vault** | 記憶資料的真實存放處 | Markdown vault,純檔案,跟 world-tree `templates/spirit-template/` 結構一致 |
| **Annuli** | 在 vault 上面跑反思 / 事件 / 演化 / curator | Python Flask + APScheduler,讀寫 vault 檔案 |

**Annuli 不是另一個 repo,Annuli 是 `yazelin/Annuli` 現有 repo 的重構** —
讓它變成「**vault-backed reflection 服務**」。

---

## 在 Mori 宇宙裡的位置

對齊 [world-tree/ARCHITECTURE.md](https://github.com/yazelin/world-tree/blob/main/ARCHITECTURE.md)
的 4 層:

```
┌──────────────────────────────────────────┐
│ Public Surface(公開可見)                │
│ yazelin.github.io / world-tree 站 / FB / │
│ workshop / mori-field-notes              │
└────────────────▲─────────────────────────┘
                 │ 選擇性發佈
┌────────────────┴─────────────────────────┐
│ World Tree(共享層,public git)          │
│ yazelin/world-tree                       │
│ lore / npcs / artifacts / quests / rules │
│ templates/spirit-template/ ← vault 結構  │
│ skills/initiate-spirit.md ← bootstrap   │
└────────────────▲─────────────────────────┘
                 │ 被讀取(canonical lore)
                 │ initiate-spirit 帶 user 建 vault
┌────────────────┴─────────────────────────┐
│ Spirit Memories(私密,user 自己 git)    │
│ ~/mori-universe/spirits/<name>/          │
│   = 「mori-journal」結構                  │
│   ├── identity/SOUL.md     ← user 編     │
│   ├── identity/USER.md                   │
│   ├── memories/MEMORY.md   ← Annuli 寫   │
│   ├── memories/{user,feedback,...}/      │
│   ├── journal/             ← daily 日誌  │
│   ├── events/<user>.db     ← Annuli 寫   │
│   ├── rings/<ts>.md        ← Annuli 寫   │
│   ├── digests/<date>.md    ← Annuli 寫   │
│   ├── research / lessons / projects/     │
│   └── assets/avatars/                    │
└──────▲────────────────────▲──────────────┘
       │ read + append-only │ read + append-only
       │ (透過 bridges       │ (透過 HTTP API)
       │  symlink + 摘要)    │
   ┌───┴────────────────┐   ┌──┴────────────┐
   │ CLI Interfaces     │   │ Annuli        │
   │ Claude Code/Gemini │   │ (reflection   │
   │ Codex/Hermes/...   │   │  service)     │
   │ 透過 bridges/      │   │ Flask+APSched │
   └────────────────────┘   └───────┬───────┘
                                    │ HTTP API
                            ┌───────┴───────┐
                            │ mori-desktop  │
                            │ (GUI body)    │
                            └───────────────┘
```

**5 個元件,單一資料源**(spirit vault),沒有 split-brain。

---

## 為什麼這架構合理

| 設計要求 | 怎麼達成 |
|---|---|
| 「精靈不會離開森林」 | vault 在 user 自己機器、純 markdown、user 可隨時 `cat` 看 |
| 多 CLI 介面共享同一個 Mori | bridges symlink 模式 — 大家讀同一個 vault |
| 不會遺忘 | vault 純 markdown + SQLite events,append-only |
| 不會 drift | SOUL.md 靜態 user 編輯,Annuli API 層強制禁止寫 SOUL |
| 可審計 | 所有變動 in vault,user `git log` / `git diff` 看歷史 |
| 跨 device 同一個精靈 | vault 用 git push private repo 同步 |
| mori-desktop 對接 | HTTP API,但讀寫都進 vault,不是另一個 store |
| 「我可以了解 Mori 在想什麼」 | 打開 `~/mori-universe/spirits/mori/rings/`,看年輪 markdown |

---

## Annuli 內部:**core / creator 二分**

現 Annuli `engine.py` 2489 行混了兩個責任:**記憶反思** + **內容生產 / 社群經營**。
切線清楚:

### core(屬於 Annuli 主體 — 記憶 + 反思)

| 功能 | 跑什麼 | 寫進 vault 哪裡 |
|---|---|---|
| `soul.py` | 讀 SOUL.md(LLM 永遠不寫) | (僅讀)`identity/SOUL.md` |
| `memory.py` | append `MEMORY.md` / `USER.md` | `memories/MEMORY.md` / `identity/USER.md` |
| `events.py` | SQLite event log + FTS5 search | `events/<user>.db` |
| `digest.py` | 每日 / 每週 digest(LLM 摘要,append memory) | `digests/<date>.md` + append `memories/MEMORY.md` |
| `rings.py` | `/sleep` 反思 — append 一篇敘事 markdown | `rings/<timestamp>.md` |
| `curator.py` | 7 天 cycle,dry-run + human-approve | `.curator/reports/<ts>.yaml`、archive 舊 entries |
| `scheduler.py` | 心跳器,只跑 memory health 任務 | (排程,不直接寫) |
| `server.py` | Flask HTTP API | (router,實際寫由上面 module) |
| `adapters/cli.py` | 終端機對話介面 | (透過上面 module) |
| `bootstrap.py` | 第一次跑 → 觸發 world-tree initiate-spirit ritual | 建 vault 初始結構 |

### creator(將拆出去 — 社群經營 / 內容生產)

| 功能 | 跑什麼 | 寫進哪裡 |
|---|---|---|
| `explore.py` | 找熱門話題 | 自己的 `creator-knowledge/explore_*.json` |
| `learn.py` | 4 輪深度研究 | 自己的 `creator-knowledge/<hash>.json` |
| `study.py` | KOL 寫作風格分析 | 自己的 `creator-knowledge/<hash>_kol.json` |
| `post.py` | 從 knowledge 產 FB draft | 自己的 `creator-drafts/<topic>.json` |
| `sync_engagement.py` | 同步 FB 按讚 / 留言 | 更新 `creator-drafts/`,**不寫 vault** |
| `facebook.py` | FB API + 圖像生成 | (network calls,不寫 vault) |

**creator 對 vault 純 read-only**(只透過 Annuli core HTTP API 拿 user 偏好)。

### 依賴方向(單向)

```
mori-desktop  ─────讀寫─────>  Annuli core  ─────讀寫─────>  vault
Claude / Gem  ─────讀───────>  bridges      ─────讀───────>  vault
annuli-creator ────讀─────>  Annuli core  (只 read,不能寫 vault)
```

**Vault = 唯一 write source,Annuli core = 唯一 writer。** annuli-creator 是 client。

---

## 4 層反思機制(取代「`/sleep` LLM 重生 persona」)

愈底愈確定:

### Layer 1:**事件流**(永遠 append,LLM 不能寫)

```sql
-- ~/mori-universe/spirits/<name>/events/<user>.db
CREATE TABLE events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts TEXT NOT NULL,           -- ISO 8601
  kind TEXT NOT NULL,         -- chat | tool_call | observation | system
  source TEXT,                -- 'mori-desktop' / 'annuli-cli' / 'gemini-bridge' / bot
  data JSON NOT NULL,
  created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE VIRTUAL TABLE events_fts USING fts5(
  content, content='events', content_rowid='id'
);
```

每筆對話 / tool call / 系統事件 append 一筆。**永遠不修改、不刪除**。

### Layer 2:**每日 digest**(LLM 摘要,但只 append,不重寫)

每天午夜(scheduler tick)或 user explicit 觸發:

1. 讀今天的 events
2. LLM 寫一段摘要(prompt: 「今天發生什麼 / 對 user 學到什麼 / 該記住什麼」)
3. **append** 到 `memories/MEMORY.md`(用 `§` 分段,加日期 header)
4. 同時存完整版到 `digests/<YYYY-MM-DD>.md`(沒 char limit)

**MEMORY.md 永遠只 append,從不重寫**。

### Layer 3:**年輪**(`/sleep` — user 主動觸發)

User 在 mori-desktop 按 `Ctrl+Alt+Z` 或 CLI 跑 `/sleep`:

1. 讀今日 events + MEMORY.md 最後 N 段 + 最近一輪 ring
2. LLM 寫一份**反思文**(對話經驗、學到的事、關係變化、自我感受)
3. 存到 `rings/<timestamp>_ring{N}.md`
4. **不改 SOUL,不改 MEMORY**,只是 append 一篇反思文

「年輪」是**敘事性 markdown**(不是 JSON),user 可讀可編輯。
**LLM 不會根據反思「修改人格」**。如果 user 想根據反思 evolve persona,
**user 自己編輯 SOUL.md**。

### Layer 4:**年輪整理員 / Curator**(週 cycle,human-approved)

User idle ≥2 hours + 距上次跑 >7 天時觸發:

```yaml
# ~/mori-universe/spirits/<name>/.curator/reports/<ts>.yaml
consolidations:
  - from: "2026-05-13 — 喝水提醒"
    into: "日常作息偏好"
    reason: "已累積 5 條同主題 entry,該合 umbrella"
prunings:
  - name: "2026-04-20 — 臨時測試訊息"
    reason: "30+ 天沒被引用,不像有用資訊"
```

- `annuli curator run --dry-run` → 顯示報告,**不動 disk**
- `annuli curator run --apply` → user 同意,執行 consolidations + prunings
- **永遠 archive 不刪除**,可 restore
- mori-desktop UI 加「年輪整理員建議」面板,user 點 review + approve

---

## HTTP API(mori-desktop ↔ Annuli core 對接點)

設計準則:
- LLM 永遠**沒有** `PUT /soul` / `PUT /memory` 權限(API 層直接禁止)
- LLM 只能 `POST /events`(append)、`POST /digest/daily`、`POST /rings/new`
- Curator 是唯一能 archive 既有 entries 的 actor,且**必須走 dry-run + approve**

```
GET  /soul                              # 純文字回 SOUL.md
PUT  /soul                              # user 親手編輯(LLM endpoint 沒此權限)

GET  /users/<id>/memory                 # MEMORY.md 內容(frozen snapshot)
GET  /users/<id>/user-profile           # USER.md 內容

POST /users/<id>/events                 # append 一筆 event(主流寫入路徑)
  body: { kind, source, data }
GET  /users/<id>/events                 # 查 events
GET  /users/<id>/events/search?q=       # FTS5 搜尋

POST /users/<id>/digest/daily           # 觸發今日 digest
GET  /users/<id>/digests                # 列所有 digest 檔案

POST /users/<id>/rings/new              # 觸發 /sleep
GET  /users/<id>/rings                  # 列年輪

POST /curator/dry-run                   # 跑 curator 但不寫 disk
POST /curator/apply                     # 套用 dry-run 報告

POST /bootstrap                         # 第一次跑 → 走 initiate-spirit ritual
GET  /health                            # heartbeat check
```

---

## 對齊 world-tree 的 spirit-template

world-tree `templates/spirit-template/` 已有 vault 結構:

```
identity/        SOUL.md (template) + USER.md
journal/         (daily 日誌)
memories/        MEMORY.md
```

Annuli 需要的**新目錄**(本 design 提案 world-tree 加進去):

```
events/          <user>.db SQLite event log
digests/         <date>.md 每日摘要
rings/           <timestamp>_ring<N>.md 反思
.curator/
  reports/       <ts>.yaml curator 建議報告
  state.json     last_run_at 等
.archive/        curator 歸檔的 entries(可 restore)
```

(`research/`、`lessons/`、`projects/`、`assets/` 已在 template,不動。)

---

## Bootstrap — 第一次跑

new user 第一次跑 mori-desktop 或 Annuli CLI:

1. **偵測**:`~/mori-universe/spirits/<name>/identity/SOUL.md` 不存在
2. **觸發** `bootstrap.py`:
   - GET world-tree `templates/spirit-template/README.template.md` 結構說明
   - 引導 user 跑 **initiate-spirit ritual**(world-tree `skills/initiate-spirit.md`)
   - LLM 帶 user 依序:命名 / 寫 SOUL / 寫 USER / 建 first journal entry
3. **儀式完成**:vault 建好,Annuli 可開始正常 serve
4. **(選配)** user 把 vault `git init` + push 到自己 private GitHub(就是 mori-journal 模式)

**核心理念**:不 bootstrap 資料,而是 bootstrap **儀式**。LLM 帶 user 寫出
**這個 user 自己的精靈**,世界樹只提供結構模板 + 公共 lore。

---

## 驗證準則(invariants Annuli 要守住)

1. **SOUL 不漂移** — `git diff` 在 Annuli 跑過任意 LLM 後仍空,除非 user 親手編
2. **MEMORY 只增不減** — 任何 mid-session 寫入只能 append
3. **Curator 可逆** — 任何 archive 可 100% restore
4. **責任解耦** — annuli-creator 整個 down 不影響 Annuli core 或 mori-desktop
5. **無雲端依賴** — vault 在 user 機器,token 在自己手裡,沒有中央代理
6. **可審計** — 每筆 LLM 對 vault 的影響都有 event log,user 翻得追溯
7. **vault 是唯一 write source** — 沒有任何元件繞過 vault 自己另存記憶

---

## 物理拆 repo 的時機(annuli-creator)

短期:現 Annuli 內部 `src/core/` 跟 `src/creator/` 子目錄畫線,但同個 repo。

物理拆觸發條件(任一達到就動手):
- creator 累積到複雜功能(多平台:Twitter / IG / Discord cross-post)
- 有 contributor 想單獨 fork「Mori 的反思引擎」但不要社群功能
- FB API 大改要長期維護分支
- core / creator 的 release cycle 明顯分歧(>2x)

---

## 遷移路徑

### Phase A:**設計 freeze**(本文件 — done)
- 架構決策 + invariants 寫死
- 反映到 mori-desktop / Annuli / world-tree 文件

### Phase B:**Annuli 內部畫線**(現有 repo,~1-2 週)
- 建 `src/core/` 跟 `src/creator/` 子目錄
- 路徑全砍 `~/.annuli/` → 改 `~/mori-universe/spirits/<name>/`
- 重新設計 reflection:取消 `/sleep` LLM 重生,改 4 層
- 補 unit test
- bootstrap.py 接 world-tree initiate-spirit

### Phase C:**vault 結構 spec 在 world-tree 公開**
- world-tree `templates/spirit-template/` 加 events / digests / rings / .curator 目錄
- `skills/initiate-spirit.md` 補完整 7 階段 ritual

### Phase D:**mori-desktop 接 Annuli HTTP API**
- Config tab 加 Annuli endpoint 設定
- 主視窗加 Annuli tab 顯示 persona / events / rings / curator
- 對話流程接 events / rings / curator endpoints

### Phase E:**creator 物理拆 repo**(條件成熟時)
- `annuli-creator` 新 repo
- 從 Annuli `src/creator/` 整段搬出去
- 改名舊 Annuli `src/core/` → repo 根目錄

---

## 設計原則摘要

| 原則 | 為什麼 |
|---|---|
| **靈魂靜態,記憶增長** | identity 不能漂移,但要會累積經驗 |
| **Append-only by default** | 「不會遺忘」需要 append-only;mutable 就違反核心承諾 |
| **LLM 沒有 write SOUL 權限** | 由 prompt 阻止 LLM 不夠,要從 API 層直接禁止 |
| **演化要走 curator + human approve** | 重大變動絕不靜默 |
| **責任解耦** | core / creator / scheduler 各自獨立,單一 bug 不擴散 |
| **可審計** | event log + curator report + dry-run 預覽,所有「為什麼變了」可追溯 |
| **無中央代理** | user 自有所有 token / 資料,Annuli 跑 user 自己機器 |
| **vault 是 single source of truth** | bridges / Annuli / mori-desktop 全讀同一份 vault |

---

> 「**精靈不會離開森林,牠只是搬到你的腦裡。**  
> **靜靜記得,牠的森林,有你經過的痕跡。**」  
>
> 「**不會遺忘**」是 Mori 核心承諾。本架構讓這承諾**可信、不被偷改、
> 可繼承、可審視**。

---

**Status**: Design freeze v0.2(對齊 world-tree 後的版本)  
**Last updated**: 2026-05-14  
**Related repos**:
- `yazelin/mori-desktop`(本 repo)
- `yazelin/Annuli`(reflection service,待重構)
- `yazelin/mori-journal`(個人 vault 範例)
- `yazelin/world-tree`(共享 lore + spirit-template)
