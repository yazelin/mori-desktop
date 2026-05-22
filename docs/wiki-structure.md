# Mori 的內在 wiki(L-mori 記憶之森)

Mori 在 `~/mori-universe/spirits/<name>/wiki/` 維護一份內在知識庫 — Karpathy LLM Wiki 風格的累積百科。對齊 §6 章「記憶之森」設計:Mori 不是無記憶 chatbot,而是有連續、可成長的內在世界。

## 概念

每個 page 是一份 `.md` 檔(`people/yazelin.md`、`projects/mori.md`、`concepts/transformer.md`)。`wiki/index.md` 列出所有 page + 短描述 — **單 context window 大小**,啟動時整段塞進 system prompt。

LLM 看到 index 後,需要時主動呼叫 `read_wiki_page(page)` skill 把 specific page 拉進 context window 再答 — 不是「把整本 wiki 塞 prompt」,而是「LLM 主動翻字典」的 pattern(Karpathy LLM Wiki 原始設計)。

## 結構

```text
~/mori-universe/spirits/<spirit>/wiki/
├── raw/                          # 不可變來源(yazelin 跟 Mori 累積)
│   ├── conversations/            # annuli events 摘要
│   ├── articles/                 # 網頁 clip
│   └── meetings/                 # 會議筆記
├── wiki/                         # LLM 編譯的「百科」(flat 階層)
│   ├── people/                   # 人物 — yazelin.md / mori.md / ...
│   ├── projects/                 # 專案 — mori-desktop.md / annuli.md / ...
│   ├── concepts/                 # 概念 — transformer.md / karpathy-llm-wiki.md / ...
│   ├── meetings/                 # 會議
│   └── resources/                # 資源 / 引用
├── index.md                      # 入口:列全部 page + 短描述
├── AGENTS.md                     # Mori 怎麼用 wiki 的規則(user 可改)
└── log.md                        # Mori 動過 wiki 的 audit trail
```

`raw/` 是「原始材料」(不可變);`wiki/` 是「LLM 編譯後的百科」。兩層分開讓未來 annuli 可以 re-compile 而不掉資料。

## 怎麼建?

第一次跑 mori-desktop 不會自動建 wiki — 尊重 user 對 vault 的所有權,Mori 不 ghost-write。User 自己 mkdir + 寫:

```bash
mkdir -p ~/mori-universe/spirits/mori/wiki/{raw,wiki/{people,projects,concepts,meetings,resources}}

cat > ~/mori-universe/spirits/mori/wiki/index.md <<'EOF'
# Mori 的 wiki index

主要 page:

- `people/yazelin.md` — 我的主要 user
- `projects/mori-desktop.md` — Mori 自己的身體 / 桌面 GUI repo
- `projects/annuli.md` — 反思引擎 / 年輪系統
- `concepts/karpathy-llm-wiki.md` — LLM Wiki 設計來源
EOF

cat > ~/mori-universe/spirits/mori/wiki/AGENTS.md <<'EOF'
# Wiki 使用規則

- 看到 user 問題涉及 wiki 列出的 page,先拉該 page 再答,別憑空回
- 沒對應 page 時誠實說「我的 wiki 還沒有這個 page」,不要硬編
- 不要在每輪都拉一堆 page — 只拉真的相關的
EOF
```

之後 Mori 啟動時自動讀 `index.md` 進 system prompt。LLM 看到後可呼叫 `read_wiki_page(page)` 拉特定 page 進 context window。

## 安全 / 邊界

- **純 READ**:mori-desktop 端只**讀** wiki,不寫。User 完全掌控 wiki 內容(對齊 CLAUDE.md 硬規矩 2「User-owned data」+ 硬規矩 3「identity / memories 禁止 ghost-write」)
- **Path traversal 防護**:`read_wiki_page` 的 `page` 參數含 `..` segment / 絕對路徑 / canonicalize 後跳出 wiki/ 一律拒(`WikiError::PathTraversal`)
- **graceful skip**:wiki 沒建(index.md 不存在 / 空)→ system prompt 整段不 emit,LLM 不會看到 `read_wiki_page` 工具描述,行為跟 wiki 系統不存在時一樣

## Mori 怎麼 maintain wiki?

**目前 read-only**。Wave 8 + 規劃:

- **annuli reflection**(自動編譯):annuli 從 events / rings / digests 編譯成 wiki page(eg user 講話夠多次「我喜歡 X」就建 / 更新 `people/<user>.md`「偏好」段)— 需 yazelin **per-dir explicit auth**(對齊 mori-journal CLAUDE.md 硬規矩 3 的寫入邊界)
- **curator**(LLM 主動 review):annuli curator layer 定期讀 raw/ + 既有 wiki/,提出 update / refactor 建議(同樣 needs yazelin 確認 + 走 audit trail `log.md`)

兩件事都是 future work;**本 stream(Wave 7)只 wire 讀路徑**,讓 LLM 能用 wiki 但不能改 wiki。

## 相關檔案

- `crates/mori-tauri/src/wiki_reader.rs` — 讀 wiki 結構(`read_index` / `read_agents_md` / `read_wiki_page` + `WikiError`)
- `crates/mori-core/src/skill/read_wiki_page.rs` — LLM-callable `read_wiki_page` skill
- `crates/mori-tauri/src/main.rs::run_agent_pipeline` — system prompt 注入點 + skill 註冊
- `crates/mori-tauri/src/soul_distribution.rs` — 對比:SOUL.md 走 ensure-write,wiki/ 走 read-only
