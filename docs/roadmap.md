# Roadmap

從 voice memo MVP 走到全方位 AI 管家。每一階段都不重寫核心,只加 module / 加 skill / 加平台殼。

---

## Phase 1 — Voice MVP(2026-05,進行中)

**目標**:按 Ctrl+Alt+M → 錄音 → Whisper → 螢幕顯示 transcript。

### Phase 1A — Scaffold(完成 2026-05-07)
- [x] Repo + Cargo workspace + Tauri scaffold + React 前端
- [x] `mori-core` traits 定義(MemoryStore / Skill / Context / LlmProvider)
- [x] `LocalMarkdownMemoryStore` trait 骨架(read/write 留到 phase 1C)

### Phase 1B — Voice pipeline 端到端(2026-05-07)
- [x] `GroqProvider::chat`:OpenAI 相容 chat completion(含 tool calling 解析)
- [x] `GroqProvider::transcribe`:Whisper turbo multipart upload
- [x] `mori-tauri`:全域熱鍵 Ctrl+Alt+M
- [x] `mori-tauri`:麥克風錄音(cpal,跨平台)+ WAV 編碼(hound)
- [x] State machine:Idle → Recording → Transcribing → Done / Error
- [x] React UI:phase-aware hero panel,錄音狀態動畫 + transcript 顯示

### Phase 1C — Chat-back + 真實記憶 I/O(2026-05-07)
- [x] `LocalMarkdownMemoryStore` 真實 read / write / search / delete
- [x] Pipeline 加入 LLM chat 階段:transcript → gpt-oss-120b → response
- [x] `Phase::Responding` / `Phase::Done { transcript, response }` 雙塊顯示
- [x] System prompt 整合 Mori persona + core memory + 當前時間
- [x] 單元測試:write/read roundtrip + search

### Phase 1D — Skill dispatch + RememberSkill(2026-05-08)
- [x] `SkillRegistry`:註冊、列舉、dispatch
- [x] `Agent::respond`:LLM tool calling 接 SkillRegistry
- [x] `RememberSkill`:LLM 自己判斷該不該寫記憶 → 直接寫入 markdown store
- [x] System prompt 加入 tool 使用守則(不要硬叫、寫完要確認)
- [x] 替代原本 hardcode 的 provider.chat(),全走 Agent 路徑

### Phase 1E — Multi-turn tools + Index-only context(2026-05-08)
- [x] `ChatMessage` 擴展支援 OpenAI tool-calling 多輪協定
      (`assistant.tool_calls` echo + `tool` role with `tool_call_id`)
- [x] `Agent::respond` 改成多輪迴圈(MAX_ROUNDS=5,LLM 看 tool 結果再答)
- [x] `RecallMemorySkill`:LLM 按需拉單筆記憶 body
- [x] `read_index_as_context` 取代 `read_all_as_context`:system prompt 只送
      索引(name + description + id),body 透過 recall_memory 拉
- [x] System prompt 重寫:教 LLM 何時用 recall vs remember、整合 vs 新增

### Phase 1F — Conversation history + Tray + Forget/Edit(2026-05-08)
- [x] **對話歷史**:`AppState.conversation: Vec<ChatMessage>`,
      Agent::respond 取 `&[ChatMessage]` history,trim 到 MAX_HISTORY_PAIRS=10
- [x] `reset_conversation` IPC + UI 按鈕 + 系統匣選單
- [x] **系統 tray icon**:顯示 / 隱藏 / 重新對話 / 離開 選單,
      關視窗 → 隱藏不殺 app(像 Slack)
- [x] **ForgetMemorySkill / EditMemorySkill**:LLM 能語音刪 / 改記憶
- [x] **Skill 呼叫透明化**:`Phase::Done` 帶 `skill_calls`,UI 在 Mori 回覆下
      列 🔧 badge,失敗顯示 ⚠️
- [x] System prompt 加 forget / edit 規則(destructive 要謹慎、明確 id)

**Phase 1 完整收工:Mori 已是端到端可用的 voice AI 管家。**

## Phase 2 — 基礎 Skills(2026-05-08)

純文字操作類,不依賴系統整合。

- [x] `TranslateSkill` — 翻譯(target_lang 含 zh-TW 在地化)
- [x] `PolishSkill` — 潤稿改錯,可指定 tone(formal/casual/concise/detailed/auto)
- [x] `SummarizeSkill` — 摘要,可指定 style(bullet_points / one_paragraph / tldr)
- [x] `ComposeSkill` — 草擬 email / message / essay / social_post
- [x] System prompt 加 4 個 text skills 的觸發守則
- [x] skill.rs 拆 module(skill/{echo,remember,recall,forget,edit,translate,polish,summarize,compose}.rs)

未排程:
- [ ] Session log:每次互動寫入 `~/.mori/sessions/<timestamp>/`
- [ ] 多 provider 支援:`OllamaProvider`(隱私任務 fallback)

## Phase 3 — Context Capture / 剪貼簿 / URL Routing

按熱鍵時自動抓「現場資訊」,LLM 根據當下 context 決定該做什麼。

### Phase 3A — 剪貼簿(2026-05-08,完成)
- [x] `TauriContextProvider` 實作 `ContextProvider` trait,讀剪貼簿文字
- [x] `run_chat_pipeline` 每輪開始抓 context
- [x] System prompt 注入 clipboard 內容(若有,4KB cap)
- [x] LLM 看到 clipboard 後,使用者說「這個 / 這段」可指代
- [x] UI 狀態列「📋 N 字 / —」+ tooltip 顯示完整內容
- [x] `context-captured` event emit 給前端

### Phase 3B — URL routing(下個 PR)
- [ ] 剪貼簿 / 輸入裡偵測 URL,填 `ctx.urls_detected`
- [ ] LLM 看到 YouTube URL → 自動建議 / 觸發 summarize
- [ ] LLM 看到一般文章 URL → fetch 內容後 summarize / extract

### Phase 3C — 跨 app 反白文字
- [ ] macOS / Windows / X11:模擬 Ctrl+C 抓 selection
- [ ] Wayland:走 xdg-desktop-portal(較難,工程量大)

### Phase 3D — 其他
- [ ] Session 自動摘要 → 寫入 archival memory

## Phase 4 — 系統整合 + ExecCommand(2026-08-09)

進入「真的能控制電腦」階段。要先把安全機制做對。

- [ ] 截圖滑鼠附近 + OCR
- [ ] 活躍視窗 / app 偵測(macOS NSWorkspace、Win32、X11、xdg portal)
- [ ] `~/.mori/skills.toml` 白名單機制
- [ ] `ExecCommandSkill`(只允許白名單指令)
- [ ] Destructive 操作二次確認 UI
- [ ] Audit log 寫入 `~/.mori/audit.log`
- [ ] `DownloadMediaSkill`(yt-dlp wrapper)

## Phase 5H — 使用者自訂 shell skill (2026-05-12,完成)

「mori 變成 LLM 加持的個人 CLI dispatcher」— profile 裡定義一段 shell_skills，
裝在系統 PATH 的任何 CLI（gh / docker / kubectl / yt-dlp / 自家 script）
都能變成 Mori 可呼叫的工具，不用改 Rust 程式碼。

- [x] `serde_yml` workspace 依賴，用真正 YAML parser 解析 frontmatter
- [x] `ShellSkillDef` / `ParamDef` 加進 AgentFrontmatter
- [x] `mori-tauri/src/shell_skill.rs` — ShellSkill 實作 mori_core Skill trait
- [x] `Box::leak` workaround for `&'static str` name/description（profile 切換頻率低）
- [x] `{{param}}` 替換邏輯（純字串，無 shell parsing → 無 injection）
- [x] timeout（預設 30 秒，可設）+ stdout/stderr 截斷（4KB / 1KB）
- [x] working_dir + success_message 模板
- [x] 範例 `AGENT-01.工作流範例.md`：gh_pr_list / docker_ps_filter / open_blog 等
- [x] 9 個 unit tests（含 shell injection 測試確認 LLM 無法 escape）
- [x] README + roadmap 更新

## Phase 5G — 雙模式架構(VoiceInput + Agent)(2026-05-12,完成)

5F 把 voice input 同時做 dictation + agent 結果走太多坑（Gemini 3 thought_signature、
Groq parse_failed、Chrome extension 焦點時序、5G 一系列 retroactive 改造）。
**5G 把責任邊界畫乾淨**：

```
Alt + 0~9         → VoiceInput profile（dictation，永遠單輪 LLM）
Ctrl+Alt + 0~9    → Agent profile（multi-turn agent，可呼叫 skill 做動作）
Ctrl+Alt + Space  → toggle 錄音（兩 mode 共用）
```

VoiceInput 只做「字」（文字轉換 + 貼游標），Agent 做「事」（chat + 動作）。

### 5G-1 — Revert voice agent loop
- [x] `run_voice_input_pipeline` 移除 agent loop / has_type_b_flags 分支
- [x] VoiceInput 永遠單輪 LLM cleanup → paste

### 5G-2 — `Mode::Active` → `Mode::Agent` 改名
- [x] mori-core/src/mode.rs enum + as_str
- [x] mori-tauri main.rs 所有 Mode::Active 引用
- [x] mori-core/src/skill/set_mode.rs（接受 "active" 為 alias，向下相容）
- [x] frontend src/FloatingMori.tsx + src/App.tsx Mode 型別

### 5G-3 — Agent profile parser
- [ ] `~/.mori/agent/AGENT-XX.主題.md` 目錄結構
- [ ] frontmatter 共用 voice_input_profile parser
- [ ] 新增 `enabled_skills: []` 鍵控制 SkillRegistry

### 5G-4 — Ctrl+Alt+0~9 熱鍵註冊
- [ ] `portal_hotkey.rs` 加 `agent-slot-0` ~ `agent-slot-9`
- [ ] emit `portal-agent-slot` 事件（payload: u8）

### 5G-5 — `handle_agent_profile_slot(N)`
- [ ] 切到 `Mode::Agent` + 套對應 profile
- [ ] floating widget 顯示「Agent · 程式助理」

### 5G-6 — 動作工具搬 mori-core/skill
- [ ] OpenUrlSkill / OpenAppSkill / SendKeysSkill / GoogleSearchSkill /
      AskChatGptSkill / AskGeminiSkill / FindYouTubeSkill
- [ ] RunShellSkill 含 `run_shell_whitelist`
- [ ] 從 `mori-tauri/src/voice_input_tools.rs` 搬實作邏輯

### 5G-7 — Profile 動態 SkillRegistry
- [ ] Agent profile 載入時依 `enabled_skills` 篩選 skill
- [ ] Routing per-skill provider 跟 profile 結合

### 5G-8 — `#file:` 預處理
- [ ] profile body 掃 `#file:path` → 讀檔 inline
- [ ] 路徑必須在 `$HOME` 子樹（防 traversal）
- [ ] 單檔 / 全部加總大小上限
- [ ] frontmatter `enable_read: true` opt-in

### 5G-9 — Floating widget 顯示模式
- [ ] 切換 profile 時 label 顯示「VoiceInput · 朋友閒聊」/「Agent · 程式助理」
- [ ] 配色區分兩種模式

### 5G-10 — 自動遷移
- [ ] 偵測 voice_input/ 內含 type-B flag 的 profile
- [ ] 啟動時搬到 agent/，通知使用者

### 5G-11 — README + roadmap 更新
- [x] README 5G 雙模式架構說明
- [x] roadmap 5G 章節

## Phase 5F — ZeroType 相容語音輸入 Profile 系統(2026-05,完成)

### Phase 5F-1 — Profile 系統核心 + Context 注入
- [ ] `~/.mori/voice_input/` 目錄 + 首次啟動自動生成預設檔案
- [ ] `SYSTEM.md` 模板引擎：`{{CONTEXT.*}}` 佔位符替換
- [ ] `USER-*.md` 載入 + YAML frontmatter 解析（`cleanup_level` / `provider` / `ZEROTYPE_AIPROMPT_*` / `ENABLE_*`）
- [ ] `active` 檔案追蹤當前 profile
- [ ] 熱鍵按下瞬間抓 context：`PROCESS_NAME` / `WINDOW_TITLE` / `ACTIVE_APP`（xdotool → `/proc/<pid>/comm`）
- [ ] `SELECTED_TEXT` 注入（`selection.rs` 已有，補接 voice pipeline）
- [ ] `CLIPBOARD` / `CURRENT_TIME` / `TODAY_DATE` / `OS` 注入
- [ ] `ZEROTYPE_AIPROMPT_*` frontmatter → openai-compatible 臨時 provider
- [ ] `provider:` mori 具名 provider 快捷方式
- [ ] `ENABLE_SMART_PASTE` / `ENABLE_AUTO_ENTER` 類型 A flag 支援
- [ ] profile 的 `cleanup_level` 覆蓋全域設定
- [ ] 預設附上課程 prompt 大全（USER-01 ~ USER-06 等）
- [ ] xdg-portal 的 active window 抓取在 Wayland 上的三層 fallback

### Phase 5F-2 — Alt+1~9 全域切換熱鍵
- [ ] `portal_hotkey.rs` 擴充支援多個快捷鍵（目前只有 Ctrl+Alt+Space）
- [ ] 向 xdg-desktop-portal 註冊 `Alt+1` ~ `Alt+9`
- [ ] 收到 `Alt+N` → 掃描 `USER-0N.*` → 寫 `active` → emit IPC 事件

### Phase 5F-3 — Floating Widget 強化
- [ ] 錄音中音量紅光：後端每 80ms emit dBFS，aura scale 跟著跳，靜音縮小不消失
- [ ] STT 完成後原文泡泡：sprite 下方顯示 ~3 秒
- [ ] Alt+N 切換時顯示 profile 檔名：sprite 下方 1.5 秒

### Phase 5F-4 — ENABLE flags 類型 B（voice input agent loop）
- [ ] Profile 有 type-B ENABLE flag → voice input 走 agent loop（無則走現有簡單路徑）
- [ ] `ENABLE_SEND_KEYS` → `ydotool key <keys>` 工具
- [ ] `ENABLE_OPEN_URL` → `xdg-open <url>` 工具
- [ ] `ENABLE_GOOGLE_SEARCH` / `ENABLE_ASK_CHATGPT` / `ENABLE_ASK_GEMINI` / `ENABLE_FIND_YOUTUBE` → URL 組合 + xdg-open
- [ ] `ENABLE_OPEN_APP` → 搜尋 `~/.local/share/applications/*.desktop` + `gtk-launch`
- [ ] `ENABLE_READ` → 讀本機檔案注入 context（`#file:` 語法）
- [ ] `ENABLE_RUN_SHELL` → shell 執行，需 `run_shell_whitelist` 白名單

## Phase 5D-3 — gemini-cli + codex-cli chat-only(2026-05-09,完成)

- [x] `CliProtocol::GeminiChat` / `CodexChat`:省略 agent 旗標,non-TTY 下 tool 執行無法被核准 → chat-only
- [x] `BashCliAgentProvider::new_with_protocol()`:顯式指定 protocol,不靠 binary 名稱偵測
- [x] `supports_tool_calling() = false` → 可安全用於 `routing.skills`,不觸發 anti-recursion guard
- [x] `build_named_provider` 新增 `"gemini-cli"` / `"codex-cli"` provider key
- [x] README 補 config 範例

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
