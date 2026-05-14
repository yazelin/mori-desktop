# Wave 4 設計 — mori-desktop ↔ annuli HTTP 接線

> **狀態**:Q1-Q5 已鎖定(2026-05-14 yazelin 套全部建議),實作中。
> **上游**:`yazelin/annuli docs/WAVE-3-DESIGN.md`(完成 + merge `c9179eb`,提供 HTTP API)
> **branch**:`feat/annuli-http-client`(從 main 切出)
> **預估**:1-2 週,含 UI 跟 e2e。

## Multi-spirit 提醒

annuli 程式碼支援多 spirit。**本機 dev 是 Mori,正式機是 Jinn**(內容生產 / FB 社群),兩 vault 完全獨立。Wave 4 接的是本機 mori spirit;`spirit_name` 是 config first-class param,不寫死 `"mori"`。

## Wave 4 目標(三句話)

1. mori-desktop **記憶層**從 `LocalMarkdownMemoryStore`(寫進 `~/.mori/memories/`)切到 `AnnuliMemoryStore`(透過 HTTP 跟 `~/mori-universe/spirits/<name>/` vault 互動)
2. 對話事件、`/sleep` 反思觸發,全部走 annuli HTTP API(`POST /events`、`POST /rings/new`)
3. UI 上加 AnnuliTab 顯示 vault 狀態(SOUL / MEMORY § / events / rings / curator reports)

## Wave 4 不做

- annuli-creator 拆 repo(Wave 5,條件成熟再說)
- 完整 initiate-spirit ritual UI(Wave 4 不寫 onboarding,假設 user 已 manual setup vault)
- 跨機器 vault sync 工具(那是 git push/pull 的事,annuli 不管)

## 上游 API surface(annuli HTTP,已 merge)

從 Wave 3 拿到的 routes:

```
GET    /spirits/<x>/soul                              (純文字 SOUL.md)
PUT    /spirits/<x>/soul                              (X-Soul-Token required)
POST   /spirits/<x>/events                            (body: user_id/kind/source/data)
GET    /spirits/<x>/events?date= | ?q= | ?kind=       (FTS5 trigram 需 ≥3 字)
POST   /spirits/<x>/rings/new                         (body: {user_id})
POST   /spirits/<x>/curator/dry-run
POST   /spirits/<x>/curator/apply                     (body: {report_path})
POST   /spirits/<x>/bootstrap
GET    /health
```

加 basic auth(`ANNULI_ADMIN_USER` / `ANNULI_ADMIN_PASS`)+ optional `X-Soul-Token`(`ANNULI_SOUL_TOKEN`,僅 PUT /soul 需要)。

vault 路徑客戶端不關心(annuli 內部管),mori-desktop 只走 HTTP。

## 設計決定(Q1-Q5,2026-05-14 yazelin 套全部建議)

> **狀態**:freeze。下面每題保留討論脈絡 + 標 **DECISION** 行,寫實作時對照。

### Q1 mori-desktop 既有 `MemoryStore` trait 怎麼對應 annuli vault?

現有 trait(`crates/mori-core/src/memory/mod.rs`):

```rust
async fn read_index() -> Vec<MemoryIndexEntry>
async fn read(id) -> Option<Memory>
async fn write(memory)
async fn search(query, limit) -> Vec<Memory>
async fn delete(id)
fn observe() -> BoxStream<MemoryEvent>
async fn list_by_types(types) -> Vec<Memory>  // 5E-3 給 voice_dict 用
```

`Memory` struct:`{id, name, description, memory_type, created, last_used, body}`。

annuli vault MEMORY.md 的單位是 `## § <header>` section,**不是 per-Memory 檔**。對應方式:

- (a) **MemoryIndexEntry ← § section**:`id = header slug`、`name = header`、`body = section markdown`、`memory_type = Other("vault_section")`(因為 annuli 沒分 type)
- (b) **加 `kind` 欄位在 section header**:`## § 2026-05-14 — preference: 喜歡冷咖啡`,parser 解 header 拿 type
- (c) **完全放棄 MemoryType 對應**:`AnnuliMemoryStore` 不支援 `memory_type` 細分,只認 generic section;`list_by_types` 直接回 [](VoiceDict 5E-3 那條 break — 要 fallback)

**DECISION (2026-05-14,套建議)**:(a) + fallback。`AnnuliMemoryStore.list_by_types(voice_dict)` 走 `read_index` + filter by header convention(例 `## § voice_dict: <X>`),沒匹配回 []。Wave 5+ 加 vault metadata 解這條更完整。

### Q2 `MemoryStore.write` 該寫去哪?

`Memory.write` 在原 model 是「user 說 remember 這件事 → 直接寫 ~/.mori/memories/<id>.md」,**immediate visible**。

annuli vault model:
- MEMORY.md 是 `digest` 自動 append(daily LLM 摘要)
- 直接 append MEMORY.md 違反 append-only-via-digest invariant

三個選擇:

- (a) **轉成 event**:`AnnuliMemoryStore.write(Memory)` → `POST /spirits/<x>/events { kind: "user_remember", data: { name, body, ... } }`。MEMORY 等下次 digest 時才會出現。**eventual consistency**
- (b) **加新 annuli API**:`POST /spirits/<x>/memory/sections`(在 annuli 加 endpoint 直接 append `## § <header>` 到 MEMORY.md)。違反「digest only writes MEMORY」純粹性但 UX 即時
- (c) **混合**:既寫 event(audit trail)又寫直接 § section(立即可見)。實際 2 處寫,維護壓力

**DECISION (2026-05-14,套建議)**:(b) — 加新 annuli endpoint `POST /spirits/<x>/memory/section`,body `{ header, body }`,直接 append § 到 MEMORY.md。標明這是「user-explicit memory write」path,跟 digest 是平行兩條 writer。invariant 不變:LLM 還是不能直接 PUT MEMORY,只能 POST 給專門 endpoint,而**這 endpoint 要 X-Soul-Token**(跟 PUT /soul 一樣等級)。

### Q3 `MemoryStore.delete` 怎麼做?

annuli vault 沒「直接 delete」,只有 curator dry-run + user approve + apply 流程。

選擇:

- (a) **mori-desktop UI 端**:user 按「刪」 → mori-desktop 跑 `POST /curator/dry-run` → 顯示 yaml,user approve → `POST /curator/apply`。**3 步驟,user 至少看 yaml 一眼才動**
- (b) **暫不實作 delete**:`AnnuliMemoryStore.delete(id) -> Err("use curator review")`,mori-desktop ForgetMemorySkill 改成顯示 toast「刪除需透過 Annuli curator,請 /sleep 之後 review」
- (c) **危險:直接刪**:在 annuli 加 `DELETE /spirits/<x>/memory/section?header=`,bypass curator。違反 archive-not-delete invariant

**DECISION (2026-05-14,套建議)**:(b) — 短期 forbidden,長期 (a)。Wave 4 範圍內 ForgetMemorySkill 改 toast,Wave 5+ 才加 UI flow。

### Q4 `AnnuliTab.tsx` UI 範圍?

Wave 4 vs Wave 5+ 切分:

- **Wave 4 MVP**:
  - GET /soul 顯示 SOUL.md(read-only,user vim 編)
  - GET MEMORY.md 顯示 § sections list(read-only)
  - GET /events?date=today 顯示今日對話(read-only)
  - GET rings/ 列年輪(read-only,點開看 markdown)
  - POST /rings/new 按鈕(手動 /sleep)
  - status bar:annuli endpoint up/down + 今日 events 數 + 待 review curator reports 數
- **Wave 5+**:
  - 直接 PUT SOUL.md(in-UI edit,需 X-Soul-Token)
  - curator review/approve UI(取代 vim 改 yaml)
  - 跨 spirit 切換(scribe / herald 加入後)

**DECISION (2026-05-14,套建議)**:照 Wave 4 MVP 範圍。

### Q5 hotkey 整合?

Wave 3 design 提到 `Ctrl+Alt+Z` 觸發 `/sleep`(`POST /rings/new`)。

考慮:
- mori-desktop 既有 hotkey(`Ctrl+Alt+Space` voice input)是 global hotkey,Linux X11 / Windows / Wayland 各有實作
- Wave 4 加 `Ctrl+Alt+Z` 也走同一條 hotkey infra(`crates/mori-tauri/src/hotkey.rs` 或對應檔)

**DECISION (2026-05-14,套建議)**:Wave 4 加 `Ctrl+Alt+Z`,沿用既有 hotkey 模組。Wave 5+ 可考慮 user 自訂(現在沒這 infra)。

## 預計實作順序(Q1-Q5 已鎖定)

| Step | 範圍 | 驗證 | repo |
|---|---|---|---|
| 1 | ✅ 本 design doc freeze + Q1-Q5 鎖定 | yazelin review | mori-desktop |
| 2 | ✅ `crates/mori-core/src/annuli/client.rs` — reqwest HTTP client wrap | 13 unit test | mori-desktop |
| 3 | annuli 加 `POST /spirits/<x>/memory/section` endpoint + X-Soul-Token guard(Q2 (b))| annuli pytest | **annuli** |
| 4 | `~/.mori/config.json` schema 加 `annuli.{endpoint, spirit_name, soul_token, user_id, basic_auth}` | 載入 round-trip | mori-tauri |
| 5 | `crates/mori-core/src/memory/annuli.rs` — `AnnuliMemoryStore` impl `MemoryStore` trait(Q1 (a)+ fallback) | unit test | mori-desktop |
| 6 | `crates/mori-tauri/src/main.rs` 啟動時根據 config 選 store | smoke run | mori-tauri |
| 7 | `crates/mori-tauri/src/hotkey.rs` 加 `Ctrl+Alt+Z` → `POST /rings/new`(Q5)| 手測 | mori-tauri |
| 8 | `agent.rs` 對話完成後 fire-and-forget `POST /events` | 手測 + e2e | mori-core |
| 9 | ForgetMemorySkill 改 toast「請走 curator review」(Q3 (b))| 手測 | mori-core |
| 10 | `src/tabs/AnnuliTab.tsx` MVP UI(Q4 — 唯讀 + /rings/new)| 手測 | mori-tauri/src/ |
| 11 | status bar:`/health` polling + 事件 / curator 數 | 手測 | mori-tauri/src/ |
| 12 | e2e:annuli + mori-desktop 整套跑通 | manual | both |

順序變動:原 step 5(annuli endpoint)前移到 step 3,因為它是 AnnuliMemoryStore.write 的 blocker(沒 endpoint 就沒法 impl write)。

## 驗證策略

### Unit test

```
crates/mori-core/tests/test_annuli_client.rs   # reqwest + wiremock mock annuli
crates/mori-core/tests/test_annuli_memory.rs   # AnnuliMemoryStore impl 對 MemoryStore trait
```

### E2E

需要本機跑 annuli HTTP server:
```bash
# Terminal 1
cd ~/mori-universe/annuli
ANNULI_SOUL_TOKEN=test-token ANNULI_ADMIN_PASS= .venv/bin/python main.py admin --port 5000

# Terminal 2
cd ~/mori-universe/mori-desktop
npm run tauri dev
# 跑 mori-desktop,對話一輪,看 vault events.md 多了 lines
```

### Invariant test(跨 wave 守住)

- mori-desktop POST /events 後 vault `events/<date>.md` 真的多了 line
- /rings/new 後 vault `rings/<date>_ring<N>.md` 真的出現
- mori-desktop **沒有任何 path** 能 PUT /soul 不帶 token(unit test 驗 client request 一定加 header,但 token 沒設時 403)
- mori-desktop fallback:annuli 沒跑時(`/health` 連不到)mori-desktop 該不該 fallback `LocalMarkdownMemoryStore`?**設計題**(Q6 候選,但 Wave 4 可以「不 fallback,直接顯示『annuli not connected』」)

## Wave 5 預告(不在本 PR)

- `annuli-creator` 物理拆 repo(條件:多平台 cross-post / 有 contributor / FB API 大改長期維護分支)
- mori-desktop UI 加 curator review / SOUL edit / spirit switcher

---

**Last updated**: 2026-05-14  
**Status**: draft,等 yazelin 回 Q1-Q5  
**Related**:
- 上游:[annuli WAVE-3-DESIGN.md](https://github.com/yazelin/annuli/blob/main/docs/WAVE-3-DESIGN.md)
- mori-desktop existing memory:`crates/mori-core/src/memory/mod.rs`(`MemoryStore` trait)
- 既有 LocalMarkdownMemoryStore:`crates/mori-core/src/memory/markdown.rs`
