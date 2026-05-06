# Roadmap

從 voice memo MVP 走到全方位 AI 管家。每一階段都不重寫核心,只加 module / 加 skill / 加平台殼。

---

## Phase 1 — Voice MVP(2026-05,進行中)

**目標**:按 Ctrl+Alt+M → 錄音 → Whisper → 螢幕顯示 transcript。

- [x] Repo + Cargo workspace + Tauri scaffold + React 前端
- [x] `mori-core` traits 定義(MemoryStore / Skill / Context / LlmProvider)
- [x] `LocalMarkdownMemoryStore` 最簡實作
- [ ] `GroqProvider`:Whisper transcription + chat completion
- [ ] `mori-tauri`:全域熱鍵(Ctrl+Alt+M)
- [ ] `mori-tauri`:麥克風錄音(cpal)
- [ ] React UI:錄音狀態 + transcript 顯示
- [ ] `EchoSkill`(LLM 看 transcript 後回個自然語言確認)
- [ ] `RememberSkill`(把當前 transcript 寫入 memory)

## Phase 2 — 基礎 Skills(2026-06)

純文字操作類,不依賴系統整合。

- [ ] `TranslateSkill` — 翻譯
- [ ] `PolishSkill` — 修詞 / 改錯字
- [ ] `SummarizeSkill` — 摘要(輸入剪貼簿或 URL 內容)
- [ ] `ComposeSkill` — 創作短文
- [ ] Session log:每次互動寫入 `~/.mori/sessions/<timestamp>/`
- [ ] LLM tool calling 真正接上(Groq function calling)
- [ ] 多 provider 支援:`OllamaProvider`(隱私任務 fallback)

## Phase 3 — Context Capture / 剪貼簿 / URL Routing(2026-07)

按熱鍵時自動抓「現場資訊」,LLM 根據當下 context 決定該做什麼。

- [ ] `ContextProvider` 各平台實作骨架(只先做剪貼簿、URL 偵測)
- [ ] URL routing:剪貼簿是 YouTube / 文章 → 摘要 / 下載 quick-action
- [ ] 「按熱鍵 + 反白文字」場景處理(macOS / Windows 用模擬 Ctrl+C)
- [ ] Session 自動摘要 → 寫入 archival memory
- [ ] `RecallSkill`:LLM 主動搜尋過去記憶

## Phase 4 — 系統整合 + ExecCommand(2026-08-09)

進入「真的能控制電腦」階段。要先把安全機制做對。

- [ ] 截圖滑鼠附近 + OCR
- [ ] 活躍視窗 / app 偵測(macOS NSWorkspace、Win32、X11、xdg portal)
- [ ] `~/.mori/skills.toml` 白名單機制
- [ ] `ExecCommandSkill`(只允許白名單指令)
- [ ] Destructive 操作二次確認 UI
- [ ] Audit log 寫入 `~/.mori/audit.log`
- [ ] `DownloadMediaSkill`(yt-dlp wrapper)

## Phase 5 — 記憶加速 + UI 美感(2026-10-11)

- [ ] `MemoryStore` 加 `sqlite-vec` 加速層(向量搜尋)
- [ ] Reranker(Cohere / 本地 sentence-transformers)
- [ ] Mori UI 美感打磨:浮動 HUD overlay、glassmorphism、framer-motion 動畫
- [ ] 系統 tray icon + 狀態指示
- [ ] Push-to-talk 模式(toggle 之外的選項)

## Phase 6 — 分身 / 行動版 / Chrome extension(2026-12 ~ 2027-01)

`mori-core` 編譯成 WASM + 各平台殼。

- [ ] `crates/mori-wasm` — WASM bindings
- [ ] `crates/mori-extension` — Chrome / Firefox / Edge(Manifest V3)
- [ ] `crates/mori-mobile`(uniffi 包成 iOS / Android binding)
- [ ] TTS 整合(piper 本地 / ElevenLabs / OpenAI tts-1)
- [ ] Wake word(openWakeWord 或 Picovoice Porcupine)

## Phase 7 — 跨裝置同步(2027-02-04)

- [ ] `crates/mori-server` — 自架伺服器(axum + WebSocket)
- [ ] 裝置配對流程(QR code + 公私鑰)
- [ ] `SyncedMemoryStore`(CRDT via yrs)
- [ ] E2E 加密(server 看不到內容)
- [ ] Tailscale 整合選項(免自架 TLS)

## Phase 8 — 跨裝置 Skill Dispatch(2027-04)

- [ ] `ExecutionTarget::Remote(DeviceId)` 真正能用
- [ ] 訊息匯流排(裝置 A 說「在 B 上跑 X」→ B 執行 → 結果回 A)
- [ ] 離線 queue(B 不在線 → 等 B 上線再執行)

## Phase 9 — 接上 Annuli + 被動學習(2027-06+)

- [ ] `AnnuliMcpMemoryStore`(透過 MCP 接 Annuli 的記憶 / 人格)
- [ ] Mori 變成 Annuli 的「桌面身體 + 服務代理」
- [ ] 共享 persona / 年輪 / 知識庫
- [ ] 被動學習:Mori 觀察使用者操作模式,自動寫入 archival
- [ ] 反思排程:夜間 batch 把 working memory 整理進 archival

## 開放問題(未排程)

- 多語言介面(目前只繁中)
- 多使用者(共用一台機器)
- Plugin 機制(讓使用者寫自己的 Skill 不用改 core)
- 收費 / 開源策略(若考慮做 hosted 版)

---

## 紀律

每個 phase 結束時問三件事:

1. 上一階段做的東西**真的有人在用嗎**?(我自己用了至少兩週)
2. 有沒有 phase 1 的東西被**砍掉重練**?(若有,前面架構錯了)
3. 下一階段的 trait / interface **有預留位置嗎**?(若沒有,先補完再開新功能)
