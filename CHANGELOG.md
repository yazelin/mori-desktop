# Changelog

依 phase 逆序(最新在上)。完整 commit log 用 `git log`。

未來規劃見 [`docs/roadmap.md`](docs/roadmap.md)。

---

## 5E-3 — VoiceInput 可選載入相關記憶 / voice_dict 校正詞庫(2026-05-12)

VoiceInput cleanup pipeline 過去純單輪 LLM 轉換,完全不參與 memory(`remember`
/ `recall_memory` 只在 Agent 模式)。痛點:Whisper 一直把「Annuli」翻成
「安奴利」/「安列利」、人名 / 公司名常被翻錯,user 沒有地方放「校正詞庫」。

5E-3 開:**VoiceInput read-only inject memory by type**(寫入仍只走 Agent)。

- **新 `MemoryType::VoiceDict` variant**:校正詞庫專用,跟 user_identity /
  preference / project / reference / skill_outcome 並列為 first-class type。
  `MemoryType::as_str()` + `MemoryType::parse()` 集中 stringify / parse,
  DRY 掉 markdown.rs + main.rs 兩處重複(`5E-3a`)
- **`MemoryStore::list_by_types(&[MemoryType])`** trait default impl:
  逐檔 frontmatter 過濾(memory 通常 <50 篇,IO 量小不需 cache)
- **Voice profile frontmatter 新鍵 `inject_memory_types: [voice_dict]`**:
  inline array 寫法。`None`(沒寫)→ 走 config 全域 fallback;`Some(vec![])`
  → 強制不 inject(即使 config 全域有設)。新增 hand-rolled
  `parse_inline_string_array` helper(`5E-3b`)
- **全域 `config.json` `voice_input.inject_memory_types`** 作為 profile 沒設時
  的 default。`resolve_inject_memory_types(profile)` 統一 fallback 鏈,各 string
  經 `MemoryType::parse` 轉
- **Voice pipeline 注入點**(`crates/mori-tauri/src/main.rs:1449` 附近):
  只在 `cleanup_level: smart` fetch memory(minimal/none 跳 LLM 不需要);
  新 `build_voice_dict_section()` helper 拼成「校正參考」段落,提示 LLM
  「這是參考詞表,**不是** user 想說的話,不要照搬輸出」。失敗 fallback 空
  字串,不擋 cleanup pipeline(`5E-3c`)
- **UI**:`MemoryTab` `TYPE_OPTIONS` 加 `voice_dict`(user 能建這型 memory);
  `ProfileEditor` 加 `MemoryTypeChipsEditor` 多選 chips,讓 voice profile
  勾選要 inject 哪些 type;`ConfigTab` voice_input section 加全域 fallback
  chips(`5E-3d`)
- **7 unit tests**:list_by_types filter / MemoryType parse + roundtrip /
  inline array parser / profile inject_memory_types Some/None/empty / resolve
  fallback chain
- **docs**:`docs/memory.md` 加完整 type 對照表 + `voice_dict` 範例 +
  Agent remember → VoiceInput inject 串接流程

## 5N — voice profile 鍵大小寫整合 + 自訂 OpenAI-compat 端點(2026-05-12)

把過去散在每張 voice profile 的 `ZEROTYPE_AIPROMPT_*` frontmatter(端點 + key
+ model 三件套)收編進 `~/.mori/config.json` 的 `providers.<name>` 機制 — Azure
OpenAI / OpenRouter / 任何 OpenAI-compat 端點都當「具名 provider」用,profile
只負責寫 `provider: <name>`。順便把 voice profile parser 統一改 case-insensitive,
修一個潛藏 bug。

- **自訂 provider** — `build_named_provider` 在 5 個 hard-coded 名字未命中時,
  改去查 `/providers/<name>/api_base`,有就視為 OpenAI-compat;讀
  `api_key_env`(指 OS env 或 `api_keys.<name>`)+ `model` 組
  `GenericOpenAiProvider`。`build_chat_provider` 同步放寬 allowlist,失敗才
  fallback 到 groq + warn(`e6ec534`)
- **Parser case-insensitive** — voice profile frontmatter 鍵全部 case-insensitive,
  canonical 寫法是 lowercase snake_case;SCREAMING_SNAKE 形式偵測到 warn 一次
  +列出遷移路徑(`872e12d`)
- **修隱性 bug**:過去所有 `USER-XX` profile 的 `enable_read: true` 因 parser 只
  認 SCREAMING_SNAKE 都被 silently 忽略,`#file:` 預處理一直沒生效。5N 起會
  真的走 ReadSkill 路徑
- **ProfileEditor 收尾** — 移除 ZEROTYPE_AIPROMPT_* details 區塊(改在
  config.json 編);`enable_auto_enter` 用 lowercase canonical patch key;provider
  Select 動態補 option 讓自訂 provider 名(如 `azure-gpt41`)顯示得出來
- **6 unit tests**:custom OpenAI-compat happy path、missing api_base、missing
  api_key、lowercase enable / mixed-case / lowercase zerotype alias
- **docs**:`providers.html` 加「自訂 OpenAI-compat 端點」段落,範例貼 Azure +
  OpenRouter 兩種寫法
- **舊鍵保留**:`ZEROTYPE_AIPROMPT_*` 跟 SCREAMING `ENABLE_*` 仍作 deprecated
  alias 解析,下個版本移除

## agent reliability / ZeroType bridge(2026-05-12)

ZeroType Agent profile 卡 `Phase::Responding` 永遠不退的 bug,沿線挖出多個
agent loop / shell_skill / LLM provider 的 reliability 問題,一併補:

- `shell_skill` 加 `kill_on_drop(true)` — Ctrl+Alt+Esc 中斷後子程序(xclip /
  ydotool)連帶 SIGKILL,避免殘留 process 影響後續 trigger(`5e8af7d`)
- `groq.rs` reqwest client 加 90s timeout — LLM call hang(stream 不結束 /
  API glitch)不再讓 agent loop 永遠卡(`68c381e`)
- 新 `AgentMode::Dispatch` profile flag — bridge profile 第一個 tool_call
  execute 後直接 Done,不再 round 第二輪 LLM(`bb831ca`)
- `generic_openai`(gemini)+ `claude_cli` + `bash_cli_agent` 同樣補 timeout
  (90s / 120s / 180s)(`17f6933`)
- 新 unit test:Dispatch / MultiTurn 路徑分別驗證,加 `AgentMode::from_str_or_default`
  parse cases(`71d3ac2`)
- `examples/agent/AGENT-03.ZeroType Agent.md` + `examples/scripts/mori-trigger-zerotype.sh`
  bridge pattern 範本入 repo;docs/profile-examples.html 加進這個 pattern 給其他
  ZeroType 學員 reference(`6f4703f`)

## brand-3 follow-ups(2026-05-12)

brand-3 後續的瑣碎收尾跟 fix:

- Custom Select 取代 native `<select>`(Linux webkit2gtk 的 GTK dropdown 配色
  被 system GTK theme 鎖死;自繪 dropdown 跨 theme 同步)(`84fad44`)
- 公式書 tagline 改「**靜靜記得 · 你經過的痕跡**」(從 SOUL.md 引言截取);
  OG image 重 render(`3dd0999`)
- Nav 統一兩套各自規範:docs nav 7 link(含 ~/.mori/ + Troubleshooting),
  design book nav 6 link(`dd18add`)
- `docs/profile-examples.html` Profile 範本頁 + `examples/agent/` + `examples/voice_input/`
  starter pack(`fb7a79b`)
- `docs/roadmap.md` 5E-3 VoiceInput 可選載入相關記憶(roadmap-only)(`667242c`)
- `docs/desktop-ui.html` Phase Status 對齊 brand-2 / brand-3 完成項;拿掉舊
  emoji 描述改 SVG `IconXxx` 名稱(`dd70dd4`, `21f703d`)
- Picker / chat_bubble hide 時加 `setSize(1, 1)` — 雙保險避免 transparent
  alwaysOnTop 視窗擋下面 app click hit-test(`81846d4`)
- `scripts/restart-dev.sh` + `npm run dev:restart` 一鍵 kill + 重啟 dev(`5498abe`)
- 12 張主畫面截圖 + main-dark.png caption 精準化(STT 收音不佳時 Mori 自我察覺,
  不是 perfect 對話)(`b37504f`, `0482cbc`)
- `docs/mori-home.html` ~/.mori/ 結構說明頁 + nav link

## brand-3 — 雙 theme + VSCode-like 自訂 theme(2026-05-12)

- CSS variables 重構:`shell.css` / `chat-panel.css` / `picker.css` / `chat-bubble.css` / `floating.css` 全部色值改用 `var(--c-*)`,`:root` 內置 Mori Dark fallback
- Theme 檔在 `~/.mori/themes/*.json` — 啟動時內建 `dark.json` / `light.json` 自動寫入,user 可複製改名做自訂
- 跨視窗同步:任一 window 切 theme 後 `emit("theme-changed")`,picker / chat_bubble / floating 全部跟著切
- UI:左下角 sun/moon quick toggle + Config tab Theme picker(列所有 themes + Reload + 顯示資料夾 path)
- Light theme 對比 override:skill kind badge / memory type chip / dep ok status / 解鎖 label / kbd 等 light 下 forest-deep 暗字
- 公式書 `brand.html` 增 forest-night `#1f3329` / forest-shadow `#172620` 兩階深綠 + Dual Theme System 整段

## brand-2 — line-art SVG icon + 全套 brand palette(2026-05-12)

- Sidebar 6 個 tab 從 emoji(💬📋⚙️📓🛠️📦)→ inline line-art SVG icon(stroke=currentColor)
- 整個專案 emoji 大掃除:ChatPanel / Picker / Profiles / Memory / Deps / Skills tab 的 emoji button / chip / section header 全換 SVG
- 主視窗從「tech dark + 天空藍」改成 forest palette:main #1f3329 / sidebar #172620 / modal #243a31 + forest active / cream text
- Chat bubble:user → sand 暖色 / assistant → forest-soft 葉脈綠
- `mori-sleeping` / `mori-error` / `mori-recording` PNG 上緣 ~y20 的橫線 artifact 抹掉(原稿 frame 殘留,sleeping 無 aura ring 蓋住才裸露)
- commits: `0b51788`, `3516131`

## brand-1 — 徽章 logo + 公式書(2026-05-12)

- Logo 徽章式設計:深綠 disc(#182622,從 logo 內部 wreath 採樣)+ 米色 stroke 縮 83% + 右移 20px 補同心
- 同一張 PNG 用到底:`public/logo.png` / `crates/mori-tauri/icons/{16~256}.png` / `docs/sprites/logo.png`
- 公式書 `docs/index.html` + brand / character / desktop-ui / tray 四頁 + `_book.css` 共用 CSS variables
- `.desktop` Icon 用 absolute path + StartupWMClass=Mori-tauri 修 dock 4 icon 堆疊
- commits: `4910510`, `afefbc0`, `033ee09`, `1369cd6`, `15d99e2`

## Phase 5O — Dependencies tab(2026-05-12)

- `~/.mori` 第 6 個 tab,偵測 + 安裝 optional 工具(yt-dlp / ydotool / xdotool / xclip / whisper-local / ollama)
- 區分 needs-sudo(顯示指令給 user 自己跑)vs 無 sudo(代執行 + 顯示 stdout/stderr)
- `crates/mori-tauri/src/deps.rs` registry,UI 可一鍵 install + reload check
- commit: `8ff7142`

## Phase 5N — Chat panel 重設計(2026-05-12)

- top bar(mode chip + provider · model + status 按鈕)/ scrollable thread / progress chip / bottom input bar 四層 vertical flex
- user 訊息靠右 / Mori 靠左,bubble + tool chip
- 從 fixed-position 改 inline thread,符合常見 chat app 操作習慣
- commit: `842ba80`

## Phase 5M — 主視窗 sidebar 架構(2026-05-12)

- App 從單一 chat 視窗變 6 個 tab(Chat / Profiles / Config / Memory / Skills / Deps)
- 左 196px sidebar + 右 main area,每個 tab 一個 React component(只 mount 當前 tab,省 IPC)
- commit: `f5be5a5`

## Phase 5L — Config / Memory / Skills UI(2026-05-12)

- **5L-1**:textarea raw editor(快速版)
- **5L-2**:`ConfigTab` typed form — 常用欄位 dropdown / input,Raw JSON 模式給 routing.skills 進階(`415617e`)
- **5L-3**:`ProfileEditor` typed form + `shell_skills` 表格 + frontmatter ↔ raw 雙向 sync(`a328213`)
- **5L-4**:`MemoryTab` browser/editor + `SkillsTab` skills inspector(列當前 profile 啟用的 builtin + shell skills)(`178b935`)
- **5L-5**:command ↔ parameters placeholder 一致性檢查 + memory 全文搜尋(rust backend) + config form 細項(`7eb4e94`)

## Phase 5K — Profile Picker + Tray submenu(2026-05-12)

- **5K-1**:Ctrl+Alt+P 開 picker overlay,3-item carousel(prev / current / next),↑↓ 切 + Tab 換組 + Enter 確認(`5edb3d7`, `40b51fc`)
- **5K-2**:tray menu 列出全部 voice / agent profile(超過 Alt+0~9 / Ctrl+Alt+0~9 上限的也能點)(`718dcc4`)
- Wayland focus quirk fix:picker / chat_bubble 第二次 hide/show 拿不到 focus → 改 setPosition 進出畫面(`527faeb`, `13f943f`)

## Phase 5J — 單層 profile + 統一 context + Gemini(2026-05-12)

- Profile 結構從「voice / agent 兩種 schema」統一為單層 frontmatter:`provider` / `stt_provider` / `enabled_skills` / `shell_skills` / `cleanup_level` 一套吃
- `build_context_section()` 在 Rust 端統一注入:timestamp / clipboard / selection / `urls_detected`,所有 provider 看到相同 context
- 新 named provider `gemini`(走 OpenAI-compat 端點)
- `chat_bubble` 獨立視窗(Wayland 單窗 setSize + transparent 不穩)
- `tauri-plugin-single-instance` 防 dev 重啟 orphan 並存
- commits: `42b41c3`, `927bb65`, `ed5a74a`, `2b2f52f`

## Phase 5I — skill_server 動態 registry(2026-05-12)

- `skill_server`(claude-bash / gemini-bash / codex-bash 共用 HTTP 入口)從 hardcoded 8 skill 改成每次 request 即時讀 active agent profile build 完整 registry
- `mori skill call <name> --args '{...}'` 通用 dispatch 子命令
- bash-CLI 系列現在跟 OpenAI tool-calling 系列(groq / ollama / claude-cli)看到一致的 action_skills + shell_skills
- commit: `efc4b99`

## Phase 5H — 使用者自訂 shell skill(2026-05-12)

- Agent profile frontmatter `shell_skills: [...]` 直接定義 CLI 包裝,不用改 Rust
- `command: ["gh", "pr", "list", ...]` 是 array(不走 shell parsing),`{{name}}` 替換是字面字串 — LLM 無法 escape
- `gh` / `docker` / `kubectl` / `yt-dlp` / 自家 script 任何 PATH 內 CLI 都能變 Mori 能力
- commit: `0149f67`

## Phase 5G — 雙模式架構(VoiceInput + Agent)(2026-05-12)

- 熱鍵 + profile 二維:`Alt+0~9` = VoiceInput / `Ctrl+Alt+0~9` = Agent
- VoiceInput:STT → 單輪 LLM cleanup → 貼游標(只做「字」)
- Agent:STT → multi-turn agent loop → chat 回應 / 動作執行(`open_url` / `send_keys` / 查資料)
- Action skills 搬到 mori-core Skill trait(原 `voice_input_tools` 刪除)
- 動態 `SkillRegistry`,profile 決定該載哪些 skill
- `#file:` 前置處理,profile body 可內嵌參考檔
- 共用 `corrections.md`,新增 default `AGENT.md`
- commits: `4aecf2a`, `a7552f5`, `939f9f5`, `546fc61`, `4bd01ed`, `6299e5f`

## Phase 5F — Voice input profile(2026-05-11)

- `Alt+1~9` 切 voice input profile(ZeroType 相容格式)
- Floating widget 重設計:音量光環 / ripple / 詳細狀態 chip(轉錄中 / 處理中 / profile 名稱)
- Voice input agent loop:9 個工具(translate / polish / 標點 / etc.)+ smart_paste
- `profile.stt_provider` 可覆蓋全域 STT 設定
- `xclip` 取代 `arboard` 消除 wl-clipboard portal 對話框
- Terminal paste-back 用 `Ctrl+Shift+V`
- `config.api_keys` 支援 `ZEROTYPE_AIPROMPT_API_KEY_ENV` fallback
- commits: `da61073`, `7632c81`, `3735c7f`, `3a43621`, `67d6b86`, `c94519c`, `dee74c9`, `92c2396`, `a6c0998`, `5570745`, `95005e6`

## Phase 5E — Voice-input mode(2026-05-08)

- Mori 變 LLM-powered dictation:STT → LLM cleanup(加標點 / 修幻聽 / 切段,prompt 鎖死不准改詞)→ 貼回游標
- 三級 cleanup:`smart`(LLM)/ `minimal`(純程式 post-process)/ `none`(raw whisper)
- commit: `117221a`

## Phase 5D — Bash CLI proxy + chat-only variants(2026-05-08~09)

- **5D-1**:`claude-bash` provider — claude CLI 當 agent,透過 Bash tool 呼叫本機 `mori` CLI dispatch skill(walk Pro/Max quota)(`f06ca0f`)
- **5D-2**:`gemini-bash` / `codex-bash` agent providers + memory skills 接上 Bash CLI proxy + tighten system prompt(`38e41ff`, `b95ed53`, `d48788a`)
- **5D-3**:`gemini-cli` / `codex-cli` chat-only 變體(no agent flag)— 給 per-skill routing 用(`eed104d`)

## Phase 5C — Local STT(2026-05-08)

- whisper.cpp Rust binding (`whisper-rs`)+ rubato 重採樣,100% 離線
- `stt_provider: "whisper-local"` + `model_path` 指向 GGML 模型(small 466MB 夠用)
- commit: `cd56c9e`

## Phase 5A — Multi-provider(2026-05-08)

- **5A-1**:`OllamaProvider` — 本機 LLM(qwen3:8b)+ tool calling + warm-up + `keep_alive=30m`(`c5a6289`, `2ff9e77`)
- **5A-2**:`ClaudeCliProvider` — `claude --print` chat-only subprocess(`3f5d948`)
- **5A-3a**:per-skill provider routing(agent + 個別 skill 走不同 provider)(`74222b9`)

## Phase 4B + 4C — 系統整合(2026-05-07~08)

- **4B-1**:Wayland-compatible global hotkey via `xdg-desktop-portal` GlobalShortcuts(GNOME 唯一可用)(`ced5c9e`)
- **4B-2**:Active / Background mode + tray toggle + `set_mode` skill(`afb7d20`)
- **4B-3**:Floating Mori widget + GNOME Wayland always-on-top fix(`44580c6`)
- **4C**:Primary-selection 讀取 + `ydotool` paste-back(Linux)(`45244b6`)

## Phase 3 — Context Capture(2026-05-07 / 2026-05-12)

- **3A**:剪貼簿自動進 context + system prompt(`874cdac`)
- **3B**:URL detection(純 Rust scanner)+ `fetch_url` skill(reqwest + HTML strip,8KB cap),trigger 鎖在「這個 / 這篇」指示詞(`d414186`, `268def8`)

## Phase 2 — 基礎 text skills(2026-05-07)

- `translate` / `polish` / `summarize` / `compose` 四個純 LLM skill
- commit: `0185cc8`

## Phase 1 — Voice MVP(2026-05-07)

- **1A**:Workspace + traits + docs scaffold(`a6d191e`)
- **1B**:Voice pipeline 端到端 — hotkey + mic + Whisper(`209ff6d`)
- **1C**:Chat-back pipeline + `LocalMarkdownMemoryStore` I/O(`d9d7455`)
- **1D + 1E**:`SkillRegistry` + `RememberSkill` + multi-turn + `RecallMemorySkill`(`45dfcc0`)
- **1F**:Conversation history + tray icon + forget/edit skills(`e72d37a`)
