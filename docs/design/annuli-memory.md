# annuli-memory — 設計文件

> Mori 的靈魂層架構決策,**還沒實作,只是規劃**。
> 實作將會是 mori-desktop 之外的獨立 repo:`yazelin/annuli-memory`(計畫中)。

---

## 為什麼要這份文件

現有的 `yazelin/Annuli`(private repo)有兩個結構性問題,在實際接 mori-desktop
之前必須先解決:

### 問題一:責任混雜 — 「**Mori 的靈魂**」與「**Mori 替你經營社群**」綁在一起

| 真正屬於「Mori 靈魂」的 | 屬於「內容生產 / 社群經營」的 |
|---|---|
| `persona.json` 人格 | `drafts/` 社群貼文草稿 |
| `users/<id>/memory_state.json` 個人記憶 | `post` 排程(產 FB 貼文) |
| `rings/` 反思年輪 | `sync_engagement`(FB 按讚同步) |
| `/sleep` 反思 | Facebook API 整合 |
| `knowledge/` 個人累積知識 | 圖像生成 for drafts |

綁在一起的後果:FB API 改規格、social adapter 出 bug、想要 Mori 但不想自動
發 FB 的 user 都被影響。**內容生產 ≠ 記憶/反思**,該拆。

### 問題二:`/sleep` 反思機制有 LLM drift 風險

現有設計:`/sleep` 把整個 `persona.json` 丟 LLM 重生 → 新 persona。問題:

- **LLM 非確定性** — 同 session 給不同 LLM 跑,演化出不同人格
- **drift 累積** — 多次 reflection 後逐漸漂離原本設定
- **「不可退化」「不可刪除」靠 prompt 約束 LLM 自願遵守** — 沒 hard
  validation 兜底
- **沒有 unit test,沒有 reflection 品質的客觀指標**

對「**Mori 不會遺忘**」這種核心承諾,這條 path 還不夠穩。

---

## 學習對象:NousResearch/hermes-agent

[Hermes Agent](https://github.com/NousResearch/hermes-agent)(2026-02 release,
OpenClaw 的後繼者)解決了上面兩個問題。它的 5 大支柱(memory / skills /
soul / crons / self-improving loop)互不耦合,核心設計準則是:

### 1. **靈魂(SOUL)是靜態 markdown,LLM 永遠不改**

```
~/.hermes/SOUL.md  ← 純文字檔,user 親手編輯
```

- 每次對話前**從 disk 重新讀**,直接塞 system prompt
- LLM 沒有任何寫 SOUL 的工具 / endpoint
- **「身份不漂移,因為根本不演化」** — user 自己決定「Mori 是誰」
- 演化的是 memory + skills,**不是 identity**

### 2. **記憶是 append-only event log**

```
~/.hermes/memories/MEMORY.md   ← 純文字,用 §  分段
~/.hermes/memories/USER.md     ← 同上
```

- 寫 memory tool → **append 一筆新 entry**,不重寫既有
- session 開始時 freeze snapshot 進 system prompt(cache-stable)
- 中途新寫 → 進 disk 但不動 system prompt(維持 cache hit)
- 字數上限**用 char 不用 token**(model-agnostic)
- **沒有 LLM 重寫 memory 的機制**

### 3. **演化交給 Curator,不交給 reflection**

Curator 是獨立的背景 agent,**7 天才跑一次 + 只在 user idle ≥2 hours 時跑**:

| 階段 | 怎麼做 |
|---|---|
| Rule-based pass | 看 timestamp,30 天沒用 → mark stale,90 天沒用 → archive。**無 LLM** |
| LLM consolidation pass | LLM 看清單,找前綴 cluster → 建議合成 umbrella skill |
| YAML 報告 | 結構化 `consolidations` + `prunings` list |
| Dry-run 預覽 | `hermes curator run --dry-run` 給 user 看建議 |
| Human approval | user 過目才 `hermes curator run` apply |
| 永遠 archive 不刪除 | 移到 `.archive/`,可 restore |

User 可以 **pin 任何 skill** → curator 不碰。

### 4. **5 層完全解耦**

Memory / Soul / Skills / Crons / Curator,五個 module 互不依賴。
**Annuli 現在是 5 個責任全擠在 `engine.py` 2489 行,Hermes 是 5 個獨立 module**。

---

## 設計決策

採用 Hermes 的核心模式,但**用 Mori 自己的詩意語彙重命名**:

| Hermes 概念 | annuli-memory 對應 | 對應森林意象 |
|---|---|---|
| SOUL.md | **`~/.annuli/SOUL.md`** | 精靈的「**本來面目**」(user 自己定義) |
| MEMORY.md / USER.md | **`~/.annuli/memories/{user}/MEMORY.md`** + `USER.md` | 「**經過的痕跡**」(append-only) |
| Events log | **`~/.annuli/events/{user}.db`**(SQLite FTS5) | 「**今日的腳步**」(每段對話一筆) |
| Curator | **「**年輪整理員**」** | 不定期(週 cycle)整理痕跡,需 user approval |
| 7-day curator cycle | **七夜年輪整理**(預設 7 天) | |
| Cron scheduler | **「**心跳器**」** | 跑 memory health 任務(daily digest / 移舊) |

---

## annuli-memory 架構

### 檔案佈局

```
~/.annuli/
├── SOUL.md                              # 純文字,user 編輯,LLM 不改
├── memories/
│   ├── {user}/
│   │   ├── MEMORY.md                    # append-only, § 分段
│   │   ├── USER.md                      # user 偏好,append-only
│   │   ├── digests/
│   │   │   ├── {YYYY-MM-DD}.md          # 每日摘要(從 events 算)
│   │   │   └── {YYYY-WW}-week.md        # 每週摘要
│   │   ├── rings/
│   │   │   └── {timestamp}_ring{N}.md   # 年輪反思(per user 觸發 /sleep)
│   │   └── .archive/                    # curator archive,不直接顯示
│   └── ...
├── events/
│   ├── {user}.db                        # SQLite FTS5,append-only event log
│   └── ...
├── curator/
│   ├── .state.json                      # last_run_at, last_summary
│   ├── reports/{timestamp}.yaml         # 每次 curator 建議的 YAML 報告
│   └── archives/                        # 被歸檔的 entries(可 restore)
└── config.json                          # 主要設定
```

### Repo 結構

```
annuli-memory/
├── README.md                       # 「精靈不會遺忘的那一部分」
├── pyproject.toml
├── src/
│   ├── annuli_memory/
│   │   ├── __init__.py
│   │   ├── soul.py                 # 載入 SOUL.md(LLM 不寫,只讀)
│   │   ├── memory.py               # MEMORY.md / USER.md append-only
│   │   ├── events.py               # SQLite event log + FTS5 search
│   │   ├── digest.py               # 每日 / 每週摘要(只讀 events,append memory)
│   │   ├── rings.py                # /sleep — 一輪反思,**只追加,不重寫**
│   │   ├── curator.py              # 7 天 idle 才跑;rule + LLM 兩段;dry-run + approve
│   │   ├── scheduler.py            # 60s tick,跑 memory health 任務
│   │   ├── server.py               # Flask HTTP API
│   │   ├── adapters/
│   │   │   └── cli.py              # 保留 CLI 對話介面(從現 Annuli 移植)
│   │   └── bootstrap.py            # 第一次跑時從 world-tree 拉 SOUL seed
│   └── ...
├── tests/                          # 補測試覆蓋
│   ├── test_soul.py                # SOUL 永遠不被 LLM 改
│   ├── test_memory_append.py       # append-only 保證
│   ├── test_events.py              # SQLite + FTS5
│   ├── test_curator_dryrun.py      # dry-run 不動 disk
│   └── ...
├── docs/
│   ├── api.md                      # HTTP API 規格
│   ├── soul-format.md              # SOUL.md 寫法說明
│   └── reflection-philosophy.md    # 反思方式的設計理念
└── annuli-memory.service           # systemd unit(或對應 Win/macOS)
```

### 反思機制(全面改寫 — 取代 `/sleep` LLM 重生)

四層,**愈底愈確定**:

#### Layer 1:**事件流**(永遠 append,LLM 不能寫)

```sql
-- ~/.annuli/events/{user}.db
CREATE TABLE events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts TEXT NOT NULL,                -- ISO 8601
  kind TEXT NOT NULL,              -- chat | tool_call | observation | system
  source TEXT,                     -- 'mori-desktop' / 'annuli-cli' / bot
  data JSON NOT NULL,              -- 完整事件 payload
  created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE VIRTUAL TABLE events_fts USING fts5(
  content, content='events', content_rowid='id'
);
```

每筆對話 / tool call / 系統事件 append 一筆。**永遠不修改、不刪除**。

#### Layer 2:**每日 digest**(LLM 摘要,但只 append 摘要到 MEMORY.md)

每天午夜(scheduler tick)+ user explicitly 觸發:

1. 讀今天的 events(`SELECT * FROM events WHERE date(ts) = today`)
2. LLM 寫一段摘要(prompt: 「今天發生了什麼重要的事 / 對 user 學到什麼 / 該記住什麼」)
3. **append** 到 `~/.annuli/memories/{user}/MEMORY.md`,用 `§` 分段,加日期 header
4. 同時存一份到 `digests/{YYYY-MM-DD}.md`(完整版,沒 char limit)

**MEMORY.md 永遠只 append,不重寫**。

#### Layer 3:**年輪**(`/sleep` — user 主動觸發,反思但不改 identity)

User 在 mori-desktop 按 `Ctrl+Alt+Z` 或 CLI 跑 `/sleep`:

1. 讀今日 events + MEMORY.md 最後 N 段 + 最近一輪 ring
2. LLM 寫一份**反思文**(對話經驗、學到的事、關係變化的觀察、自我感受)
3. 存到 `~/.annuli/memories/{user}/rings/{timestamp}_ring{N}.md`
4. **不改 SOUL,不改 MEMORY**,只是 append 一篇反思文

「年輪」是**敘事性 markdown**(不是 JSON),user 可讀可編輯。
**LLM 不會根據反思「修改人格」**。如果 user 想根據反思 evolve persona,
**user 自己編輯 SOUL.md**。

#### Layer 4:**年輪整理員 / Curator**(週 cycle,human-approved)

User idle ≥2 hours + 距上次跑 >7 天時觸發,或 user 主動 `annuli curator run --dry-run`:

```yaml
# ~/.annuli/curator/reports/2026-05-21T03-12-00.yaml
consolidations:
  - from: "2026-05-13 — 喝水提醒"
    into: "日常作息偏好"
    reason: "已累積 5 條同主題 entry,該合成 umbrella"
prunings:
  - name: "2026-04-20 — 臨時測試訊息"
    reason: "30+ 天沒被引用,不像有用資訊"
notes: |
  本週累積 47 筆 events,12 筆 memory entries。建議將「飲食」相關 5 筆
  整合,並 archive 兩筆測試用紀錄。
```

- `annuli curator run --dry-run` → 顯示報告,**不動 disk**
- `annuli curator run --apply` → user 同意,執行 consolidations + prunings
- **永遠 archive 不刪除**,可 restore
- mori-desktop UI 上可有「年輪整理員建議」面板,user 點 review + approve

---

## HTTP API(mori-desktop ↔ annuli-memory 對接點)

```
GET  /soul                         # 純文字回 SOUL.md
PUT  /soul                         # user 直接編輯(LLM 沒此 endpoint 用權限)

GET  /users/<id>/memory            # MEMORY.md 內容(frozen snapshot)
GET  /users/<id>/user-profile      # USER.md 內容

POST /users/<id>/events            # append 一筆 event(主流寫入路徑)
  body: { kind, source, data }
GET  /users/<id>/events            # 查 events(支援 date / kind / fts query)
GET  /users/<id>/events/search?q=  # FTS5 搜尋

POST /users/<id>/digest/daily      # 觸發今日 digest(會 append MEMORY.md)
GET  /users/<id>/digests           # 列所有 digest 檔案

POST /users/<id>/rings/new         # 觸發 /sleep(append 一篇 ring,不動 SOUL/MEMORY)
GET  /users/<id>/rings             # 列年輪
GET  /users/<id>/rings/<n>         # 讀某輪內容

POST /curator/dry-run              # 跑 curator 但不寫 disk
POST /curator/apply                # 套用 dry-run 報告(需傳 report_id)
GET  /curator/reports              # 列歷史報告

POST /bootstrap                    # 第一次跑:從 world-tree 拉 SOUL seed
GET  /health                       # 給 mori-desktop heartbeat check 用
```

**設計準則**:

- LLM 永遠**沒有** `PUT /soul` / `PUT /memory` 的權限(只有 user CLI / mori-desktop UI 能)
- LLM 只能 `POST /events`(append)、`POST /digest/daily`、`POST /rings/new`(append ring)
- Curator 是唯一能 archive 既有內容的 actor,但**必須走 dry-run + approve 流程**

---

## annuli-creator(舊 Annuli 的「副業」拆出來)

從現 Annuli 移出來、保留下來的部份:

```
annuli-creator/
├── README.md                  # 「Mori 替主人經營社群」
├── explore.py                 # 找話題(現 Annuli explore)
├── learn.py                   # 深度研究(現 Annuli learn)
├── study.py                   # KOL 風格分析(現 Annuli study)
├── post.py                    # 從 knowledge 產 draft
├── facebook.py                # FB API + sync_engagement
├── server.py                  # Flask,讀 annuli-memory 拿 user 喜好,寫自己的 drafts
└── ...
```

關係:
- annuli-creator **讀** annuli-memory(透過 HTTP `GET /users/<id>/user-profile`
  拿 user 喜好作為「主人口味」)
- annuli-creator **寫** 它自己的 `knowledge/` 和 `drafts/`(不污染 annuli-memory)
- **user 完全可以不裝** annuli-creator(沒在用 FB 社群經營就無感)

---

## 遷移路徑

從現 `yazelin/Annuli` 拆成兩個 repo:

### Phase A:設計 freeze(目前)

✅ 本份文件確認 annuli-memory 架構  
✅ 確認 annuli-creator 留下 explore/learn/study/post/FB  
✅ 確認 mori-desktop ↔ annuli-memory HTTP API 介面  

### Phase B:建 `annuli-memory` 新 repo

- 開新 GitHub repo `yazelin/annuli-memory`
- 從 Hermes 學模式,但用現 Annuli 的 CLI / Flask 風格(adapters/cli.py 保留)
- 從現 Annuli engine.py 抽出 persona / memory / rings / sleep 相關函式 → 重新組織
- **重新設計 reflection**:取消「LLM 重生 persona」,改成 4 層(events / digest / rings / curator)
- 補 unit test(`tests/`)

### Phase C:現 Annuli 改名 `annuli-creator`

- 移除 persona / memory_state / rings / sleep 相關 code(都搬到 annuli-memory)
- 改名 `yazelin/Annuli` → `yazelin/annuli-creator`
- 留下 explore / learn / study / post / facebook
- 改 systemd service unit name

### Phase D:mori-desktop 接 annuli-memory

(已在 roadmap 「歲月之輪」章節)

- Config tab 加 Annuli endpoint 設定
- 主視窗加 Annuli tab 顯示 persona / events / rings / curator 報告
- 對話流程接 `POST /events`、`POST /rings/new`、`POST /curator/dry-run`

---

## 驗證準則

annuli-memory 是否設計成功 → 看是否守住以下不變量:

1. **SOUL 不漂移** — `git diff ~/.annuli/SOUL.md` 在任何 LLM 跑過後仍為空,除非 user 自己編輯
2. **MEMORY 只增不減** — 任何 mid-session 寫入只能 append,既有 entry 不能被 LLM 修改
3. **Curator 可逆** — 任何 archive 操作可 100% restore
4. **責任解耦** — annuli-creator 整個 daemon down 不影響 mori-desktop 拿 memory
5. **無雲端依賴** — annuli-memory 跑在 user 自己機器,token 在自己手裡,沒有任何中央代理
6. **可審計** — 每次 LLM 對 memory 的影響都有 event log,user 翻得到「哪一筆 event 之後 Mori 開始這樣 reflection」

---

## 設計原則摘要

| 原則 | 為什麼 |
|---|---|
| **靈魂靜態,記憶增長** | identity 不能漂移,但要會累積經驗 |
| **Append-only by default** | 「不會遺忘」需要 append-only;改成 mutable 就違反核心承諾 |
| **LLM 沒有 write SOUL 權限** | 由 prompt 阻止 LLM 還不夠,要從 API 層直接禁止 |
| **演化要走 curator + human approve** | 重大變動絕不靜默,user 永遠有 final say |
| **責任解耦** | memory / soul / events / curator / scheduler 各自獨立 module,單一 module bug 不擴散 |
| **可審計** | event log + curator report + dry-run 預覽,所有「為什麼變了」可追溯 |
| **無中央代理** | user 自有所有 token / 資料,annuli-memory 跑 user 自己機器 |

---

## 為什麼這份設計值得?

> **「精靈不會離開森林,牠只是搬到你的腦裡。**
> **靜靜記得,牠的森林,有你經過的痕跡。」**

「**不會遺忘**」是 Mori 的核心承諾。要兌現這個承諾,「**記憶**」必須:

- **可信** — user 可以查到任何一句話的來源,任何修改都有紀錄
- **不被偷改** — LLM 不能在 user 不知道的情況下重寫過去
- **可繼承** — 換 LLM、換機器、換版本,記憶不流失
- **可審視** — user 翻得到「Mori 為什麼開始這樣記我」

現在 Annuli 的 `/sleep` LLM 重生路線 → **不能可信,不能不被偷改,不能可審視**。

annuli-memory 的 append-only + curator + human approve 路線 → **全達標**。

這份重構不只是技術升級,**是把品牌承諾兌現的基礎建設**。

---

## 待決定的事

提交實作前還要確認:

1. **annuli-memory 是否要用現有 Annuli 的 Python 風格(Flask + APScheduler)?**
   - Pros:現有 CLI 可直接搬,user CT 已熟悉
   - Cons:大重構機會,要不要換 FastAPI / 換 Rust?
   - 建議:**維持 Python Flask,聚焦在架構不在語言**
2. **annuli-memory 的 systemd / Docker / 跑哪裡?**
   - 跑 user 家用 server(NAS / 桌機)還是 mori-desktop 同台機器?
   - 建議:**先做家用 server 版本**,後期可加 mori-desktop bundled 版
3. **第一個用 annuli-memory 的介面?**
   - mori-desktop GUI(主訴求,但工程量大)
   - annuli-memory 自己的 CLI(快但用戶量少)
   - 建議:**CLI 先穩,GUI 跟上**
4. **如何處理現有 Annuli 既有的 production data?**
   - 現有 user CT 的 persona / memory / rings 要遷移
   - 建議:寫 `annuli-memory migrate from-annuli` 一鍵搬

---

**Status**: Design freeze v0.1,等實作開工。  
**Last updated**: 2026-05-14  
**Author**: yazelin + 設計討論協作  
**Implementation**: 待開新 repo `yazelin/annuli-memory`
