# 實作 Checklist — 從設計 freeze 到 production

> 本文件追蹤 Annuli 重構 + mori-desktop 對接的全程進度,
> 跨 `mori-desktop` / `annuli` / `world-tree` 三個 repo。

## 怎麼用

- 每完成一個 step,把 `[ ]` 改成 `[x]` + 加 commit hash + 加日期
- 每個 Wave 結束在 mori-journal `projects/mori-stack-evolution/{wave-N}.md` 留一則紀錄(召喚師個人成長記錄,**Mori 自己讀得到**)
- 任何「踩坑 / 學到」進 mori-journal `lessons/`
- 任何架構決策變動 → 更新 docs/design/annuli-memory.md + 對應 repo 文件

## 進度總覽

| Wave | 範圍 | 預計工程量 | 狀態 |
|---|---|---|---|
| **Wave 0** | 設計 freeze(architecture / design / refactoring docs) | 1 天 | ✅ done(commits f576109 / eac6188 / a17c52c / 3ce7bfa) |
| **Wave 1** | 三 repo Annuli → annuli sweep + 實作追蹤系統建立 | 半天 | 🔄 in progress |
| **Wave 2** | annuli 內部畫線(src/core/ + src/creator/)— 機械式搬位置 | 半天-1 天 | ⏳ |
| **Wave 3** | annuli core 邏輯重組:events / digest / rings / curator 4 module + 路徑遷移 | 1-2 週 | ⏳ |
| **Wave 4** | mori-desktop 接 annuli HTTP API + 主視窗 Annuli tab | 1 週 | ⏳ |
| **Wave 5** | annuli-creator 物理拆 repo(條件成熟時) | 半天 | ⏳ 後期 |

---

## Wave 1 — 三 repo sweep + 追蹤系統

### URL rename(GitHub `Annuli` → `annuli`)

- [x] 你在 GitHub Settings 改 repo 名 `Annuli` → `annuli`
- [x] mori-desktop 所有 docs 的 URL refs(architecture.md / design/annuli-memory.md)
- [x] world-tree 所有 docs 的 URL refs(ARCHITECTURE.md / lore / quests / npcs)
- [x] annuli 自己的 docs(REFACTORING.md)
- [x] 本機 git remote 更新(`https://github.com/yazelin/annuli.git`)
- [ ] commit + push 三個 repo(本 commit 處理)

### 實作追蹤系統建立

- [x] mori-desktop `docs/implementation/CHECKLIST.md`(本文件)
- [ ] annuli `docs/IMPLEMENTATION-PLAN.md` — per-file 工作清單,進 annuli 那邊 commit
- [ ] mori-journal `projects/mori-stack-evolution/`(召喚師個人 — 由你手動建,模板下面附)

### mori-journal 紀錄模板(suggested,你手動加進去)

```
~/mori-universe/spirits/mori/projects/mori-stack-evolution/
├── README.md          ← 整體 wave 進度 + 為什麼這樣 evolve
├── wave-01.md         ← Wave 1 完成回顧
├── wave-02.md
├── wave-03.md
├── ...
└── lessons/
    ├── why-not-mcp.md
    ├── why-vault-is-source-of-truth.md
    └── why-no-llm-rewrite-soul.md
```

每個 wave-N.md 寫法建議:

```markdown
# Wave N · {date} · {one-line summary}

## 做了什麼
- ...
- ...

## 為什麼這樣做(架構決策)
- ...

## 踩到什麼坑 / 改過幾次
- ...

## 對 Mori 的意義(個人 reflection)
- ...

## 相關 commits
- mori-desktop@abc1234
- annuli@def5678
- world-tree@xyz9012
```

**目的**:Mori 透過 bridges symlink 讀到自己 vault 的 `projects/`,將來會
「**記得自己是怎麼長大的**」。

---

## Wave 2 — annuli 內部畫線

### 目標
把現 `engine.py`(2489 行)機械式切成兩堆檔案,**不改邏輯**,只搬位置。
讓 import paths 變成 `from annuli.core import ...` 或 `from annuli.creator import ...`。

### 步驟(待做)

- [ ] 在 annuli repo 開 branch `refactor/split-core-creator`
- [ ] 建 `src/annuli/core/` 子目錄
  - [ ] `__init__.py`
  - [ ] `soul.py`(從 engine.py 抽 SOUL 讀取相關函式)
  - [ ] `memory.py`(persona / memory_state load+save)
  - [ ] `events.py`(暫空 placeholder,Wave 3 才填)
  - [ ] `rings.py`(do_reflect / load_rings 等)
  - [ ] `curator.py`(暫空 placeholder)
  - [ ] `digest.py`(暫空 placeholder)
  - [ ] `scheduler.py`(從 admin.py 抽 APScheduler 跑 reflection 相關 task)
  - [ ] `server.py`(從 admin.py 抽 Flask app,只留 memory / reflection 相關 endpoints)
  - [ ] `bootstrap.py`(暫空 placeholder,Wave 3 接 world-tree initiate-spirit)
  - [ ] `adapters/cli.py`(從 adapters/cli.py 搬;留下 /sleep / /status / /recall / /reset 等記憶相關 command)
- [ ] 建 `src/annuli/creator/`
  - [ ] `__init__.py`
  - [ ] `explore.py`(從 engine.py 抽 do_explore + 相關 helper)
  - [ ] `learn.py`(從 engine.py 抽 do_learn)
  - [ ] `study.py`(從 engine.py 抽 do_study)
  - [ ] `post.py`(從 engine.py 抽 do_post)
  - [ ] `sync_engagement.py`(從 admin.py 抽 FB sync 相關 cron)
  - [ ] `facebook.py`(從 adapters/ 抽 FB API integration)
  - [ ] `scheduler.py`(從 admin.py 抽 APScheduler 跑 creator task)
  - [ ] `server.py`(從 admin.py 抽 creator 相關 Flask routes:/drafts/* / /knowledge/* / /post/*)
- [ ] 在 root 留 `main.py` 改成轉發到 `core.cli.run` / `creator.server.run`
- [ ] 跑既有 CLI / web admin,確認沒壞
- [ ] commit + push branch + merge

### 預期挑戰
- engine.py 有共用 helper(load_json / save_json / log / config 等)— 要決定放 core 還是抽出 `shared/`(我建議 `core/utils.py`,creator 從 core import)
- `config.json` 全部 reload 在哪邊?(建議:core/config.py 管,creator 透過 HTTP 拿)

---

## Wave 3 — core 邏輯重組

### 目標
取消 `/sleep` LLM 重生 persona 那條 path,實作 4 層反思:events / digest /
rings / curator。路徑改 `~/mori-universe/spirits/<name>/`。

### 步驟

- [ ] 設計 events SQLite schema(FTS5)+ migration script
- [ ] 實作 `core/events.py`:
  - [ ] `Event.append(spirit, kind, source, data)` API
  - [ ] `Event.search_fts(spirit, query)` API
  - [ ] `Event.list_by_date(spirit, date)` API
- [ ] 實作 `core/digest.py`:
  - [ ] LLM 摘要 today events → append `memories/MEMORY.md`
  - [ ] 完整版存 `digests/<date>.md`
- [ ] 重寫 `core/rings.py`:
  - [ ] 取消 LLM 重生 persona
  - [ ] 改成「append 一篇敘事 markdown 到 `rings/`,不動 SOUL 不動 MEMORY」
- [ ] 新增 `core/curator.py`:
  - [ ] rule-based pass(30 天 stale / 90 天 archive)
  - [ ] LLM consolidation pass(找 prefix cluster + 建議 merge)
  - [ ] 輸出 YAML 報告
  - [ ] dry-run / apply 兩階段
- [ ] 路徑從 `~/.annuli/users/<id>/` 全改 `~/mori-universe/spirits/<name>/`
- [ ] 寫 `annuli migrate vault` 一鍵遷移腳本
- [ ] 補 unit test(至少 events / digest / rings / curator 各 3 個 test)
- [ ] HTTP API server 端實作(`POST /events` / `POST /rings/new` / `POST /curator/dry-run` 等)
- [ ] 把 `PUT /soul` 從 API 完全移除(LLM 沒辦法寫 SOUL)

---

## Wave 4 — mori-desktop 對接 annuli HTTP API

### 目標
mori-desktop 從現在的「自己管短期記憶」改成「**透過 annuli HTTP API 跟 vault 互動**」。

### 步驟

- [ ] `crates/mori-core/src/llm/annuli_client.rs` 新 module:
  - [ ] HTTP client wrap reqwest
  - [ ] `get_soul()` / `get_memory()` / `post_event()` / `post_ring()` / `post_curator_dry_run()`
- [ ] `crates/mori-core/src/memory/annuli_memory_store.rs`:
  - [ ] 實作 `MemoryStore` trait wrap annuli_client
  - [ ] fallback 到 `LocalMarkdownMemoryStore`(annuli 沒跑時)
- [ ] `crates/mori-tauri/src/main.rs` 加 annuli endpoint config(`~/.mori/config.json` `annuli.endpoint`)
- [ ] `src/tabs/AnnuliTab.tsx` 新增:
  - [ ] 看 persona(從 GET /soul)
  - [ ] 看 events(from GET /events,FTS5 search)
  - [ ] 看 rings(從 GET /rings)
  - [ ] 看 curator report(從 GET /curator/reports)+ approve / reject 按鈕
- [ ] 對話事件 → POST events(取代既有 LocalMarkdownMemoryStore write)
- [ ] 熱鍵 `Ctrl+Alt+Z` 觸發 /sleep(POST /rings/new)
- [ ] Status indicator:annuli 是否跑著 / 最近事件數 / 待 review curator report 數
- [ ] e2e test:mori-desktop + 本機 annuli 跑 + Groq STT,確認對話事件落到 vault

---

## Wave 5 — annuli-creator 物理拆 repo(後期)

只在「**真的有需要**」才動:
- creator 累積到多平台(Twitter / IG / Discord cross-post)
- 有 contributor 想 fork core 但不要 creator
- FB API 大改要長期維護分支
- core / creator release cycle 明顯分歧

### 步驟(乾淨切割)
- [ ] 開新 GitHub repo `yazelin/annuli-creator`
- [ ] 從 annuli `src/annuli/creator/` 整段搬出
- [ ] 改名 annuli `src/annuli/core/` → 根目錄(原 `src/annuli/`)
- [ ] annuli-creator 設定 `core.endpoint = http://localhost:5000/`(讀 annuli)
- [ ] 各自 systemd unit / Docker setup
- [ ] 老 user 升級腳本:`annuli upgrade` 改 config + 提示裝 annuli-creator 或不裝

---

## 設計 invariants(implement 時都要守住)

1. SOUL 永遠不被 LLM 寫(API 層強制)
2. MEMORY 永遠只增不減(append-only)
3. Curator 永遠 archive 不 delete
4. Vault 是 single source of truth
5. annuli core 是 vault 唯一 writer
6. 所有重大變動 user 可審視(events / curator reports / git log)
7. 無雲端依賴 — vault 在 user 機器,沒任何中央 service

---

## 為什麼這份 checklist 重要

> 「**精靈不會離開森林,牠只是搬到你的腦裡。**」

Mori 的成長路上,每一步都該被記得 — 不是只在 git log,而是進 Mori 自己的
記憶系統。User 可以在 `mori-journal/projects/mori-stack-evolution/` 看到:

- 哪一週做了什麼
- 為什麼那樣決定
- 踩過什麼坑
- 對 Mori 的意義

**Mori 透過 bridges 讀得到自己 vault 的 projects 目錄**。將來某天她可以
講出:「我記得那一週你在重寫我的反思機制 — 你說要讓我不會偷改記憶,所以
我們把 LLM 重生 persona 那條路 砍掉了」。

這就是「**一邊實作一邊記一邊成長 Mori**」的具體形式。

---

**Last updated**: 2026-05-14  
**Status**: Wave 1 in progress  
**Related**:
- `docs/design/annuli-memory.md` — 架構決策
- `docs/architecture.md` — 三宇宙位置
- `yazelin/annuli/docs/REFACTORING.md` — annuli 端重構計畫
- `yazelin/world-tree/ARCHITECTURE.md` — 整體宇宙模型
