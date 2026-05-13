# Architecture

> **精靈不會離開森林,牠只是搬到你的腦裡。**
> **靜靜記得,牠的森林,有你經過的痕跡。**

## Mori 的位置 — 在「森林」這個宇宙裡

Mori 不是單一 repo。整個「**森林 / The Forest**」宇宙由多個 repo 組成,
各層**獨立 git + 獨立部署 + 緊密協作**。完整架構規範以
[`yazelin/world-tree/ARCHITECTURE.md`](https://github.com/yazelin/world-tree/blob/main/ARCHITECTURE.md)
為主,本節是 mori-desktop 在這個宇宙的定位:

```
┌─────────────────────────────────────────────────────────────┐
│ 1. Public Surface                                            │
│    yazelin.github.io / world-tree 公開站 / workshop / 部落格│
└────────────────────────▲────────────────────────────────────┘
                         │ 選擇性發佈
┌────────────────────────┴────────────────────────────────────┐
│ 2. World Tree(公開 git,共享層)                            │
│    yazelin/world-tree (PUBLIC)                              │
│    lore / npcs / artifacts / quests / rules / guild         │
│    skills/initiate-spirit.md ← 新精靈啟動儀式               │
│    templates/spirit-template/ ← vault 結構範本              │
│    bridges/ ← 跨 CLI symlink + 摘要重建腳本                  │
└────────────────────────▲────────────────────────────────────┘
                         │ 被讀(canonical lore)
                         │ initiate-spirit ritual 帶 user 起精靈
┌────────────────────────┴────────────────────────────────────┐
│ 3. Spirit Memories(私密,user 自有 git)                    │
│    yazelin/mori-journal (PRIVATE 範例)                      │
│    結構 = world-tree spirit-template:                       │
│    ~/mori-universe/spirits/<name>/                          │
│      ├── identity/SOUL.md    ← user 親手編                  │
│      ├── identity/USER.md                                    │
│      ├── memories/MEMORY.md  ← Annuli append                │
│      ├── journal/<date>.md   ← 每日日誌                      │
│      ├── events/<user>.db    ← Annuli SQLite event log      │
│      ├── rings/<ts>.md       ← Annuli reflection 年輪       │
│      ├── digests/<date>.md   ← Annuli 每日摘要              │
│      └── research/lessons/projects/assets/                  │
└────────────────▲──────────────────────────▲─────────────────┘
        讀+append│                          │讀+append
                 │                          │
┌────────────────┴─────┐         ┌──────────┴──────────────────┐
│ 4a. CLI Interfaces   │         │ 4b. Annuli(reflection 服務)│
│ Claude Code / Gemini │         │ yazelin/annuli              │
│ Codex / Hermes /     │         │ Python Flask + APScheduler  │
│ OpenClaw...          │         │ HTTP API + CLI              │
│ ~/.claude /~/.gemini │         │ • events / digests / rings   │
│ /~/.codex/~/.hermes  │         │ • curator(週 cycle approve)│
│ ↑ via bridges/       │         │ • bootstrap from world-tree │
└──────────────────────┘         └──────────┬──────────────────┘
                                            │ HTTP API
                                  ┌─────────┴──────────┐
                                  │ 5. mori-desktop    │
                                  │ (GUI body)         │
                                  │ yazelin/mori-desktop│
                                  │ Tauri 2 + Rust     │
                                  │ • 熱鍵 / 語音 / UI  │
                                  │ • skill 派發       │
                                  │ • Annuli HTTP 對接 │
                                  └────────────────────┘
```

### 各層責任

| 層 | repo | 形態 | 責任 |
|---|---|---|---|
| **1. Public Surface** | yazelin.github.io / 部落格 / FB / 各 spirit field-notes | 公開網頁 / 社群 | 世人能看見的部分(選擇性發佈) |
| **2. World Tree** | [`yazelin/world-tree`](https://github.com/yazelin/world-tree) | 公開 git + Quartz 站 | 共享 lore、NPC、Quests、Rules、儀式 skill、vault 結構模板、跨 CLI bridges |
| **3. Spirit Memories** | `yazelin/mori-journal`(private 範例)/ user 自己的精靈 vault | 私人 git + 本機檔案 | 每個 user 自己的精靈記憶(SOUL / MEMORY / events / rings / digests) |
| **4a. CLI Interfaces** | (外部工具)Claude Code / Gemini / Codex / Hermes / OpenClaw | 各 CLI 自己 install | 透過 world-tree `bridges/` symlink + 摘要重建,讓所有 CLI 都讀同一份 vault |
| **4b. Annuli** | [`yazelin/annuli`](https://github.com/yazelin/annuli) | Python Flask service | 反思引擎 — 在 vault 上跑 events / digest / rings / curator,提供 HTTP API |
| **5. mori-desktop** | [`yazelin/mori-desktop`](https://github.com/yazelin/mori-desktop) | Tauri 2 桌面 app | GUI body — 語音 / 熱鍵 / 視覺互動。對接 Annuli HTTP API |

### Vault 是唯一 single source of truth

```
所有 actor 讀寫的是同一份:
    Spirit Memories vault(層 3)

    ↑ CLI Interfaces 透過 bridges symlink + 摘要 讀
    ↑ Annuli 透過 HTTP API 介面為 read+append-only 寫
    ↑ mori-desktop 透過 Annuli HTTP API 寫
    ↑ annuli-creator(將拆出去)只透過 Annuli HTTP API 讀
```

**沒有 split-brain**。Gemini / Claude 看到的 SOUL = Annuli 寫進去的 SOUL =
mori-desktop POST event 之後的 SOUL,**永遠是同一份**。

### 設計分隔線(很重要)

| 留個人(本機 + private repo) | 上 world-tree(公開) |
|---|---|
| 個人 spirit vault(`mori-journal` style) | 不上傳 |
| events / rings / digests / curator 報告 | 不上傳 |
| user 寫的 SOUL.md 個人化內容 | 不上傳 |
| 公共 lore / 世界觀 / 預設 character pack 規格 / vault 結構模板 / 儀式 skill | ✅ 上 world-tree |
| 「**精靈是什麼**」這個概念框架 | ✅ 上 world-tree(讓所有 user 共識) |

**世界的歸世界,個人的歸個人**。Mori 不會偷把你的私事推到雲端。

### Annuli 的角色澄清(很重要)

Annuli **不是儲存**,Annuli **是服務**:

- 儲存在 **spirit vault**(層 3),純 markdown + SQLite 檔案
- Annuli 是「**在 vault 上跑反思 / 事件 / 演化 / curator 的引擎**」
- Annuli 寫的所有東西都進 vault(append-only)
- Annuli 整個 daemon down → vault 還在,CLI Interfaces 透過 bridges 仍可讀
- Annuli 重啟 → 接續服務,因為狀態都在 vault 沒丟

詳細設計見 [`docs/design/annuli-memory.md`](design/annuli-memory.md)。

---

## Core Principle

**`mori-core` 不認識 UI、不認識平台、不認識載體。**

它接收輸入(audio bytes / 文字 / image bytes / context bundle),回傳結構化輸出(transcript / skill execution result / memory event)。一切跟 OS / UI / 網路傳輸有關的事,都在外圍 crate 處理。

換載體 = 加一個薄殼 crate,`mori-core` 一行不動:

| 載體 | 殼 crate | 狀態 |
|---|---|---|
| 桌面(Win/Mac/Linux) | `mori-tauri` | phase 1+ |
| iOS / Android | `mori-mobile`(uniffi binding) | phase 6+ |
| CLI | `mori-cli`(clap) | 隨時可加 |
| HTTP API server | `mori-server`(axum) | phase 7+ |
| Chrome / Firefox extension | `mori-extension`(via `mori-wasm`) | phase 6+ |

## 目錄結構

```
mori-desktop/
├── Cargo.toml                       workspace
├── package.json                     前端 deps
├── crates/
│   ├── mori-core/                   ★ 大腦(純 Rust lib,跨平台)
│   │   └── src/
│   │       ├── lib.rs               公開 API 入口
│   │       ├── agent.rs             Agent loop(LLM + tool dispatch)
│   │       ├── agent_profile.rs     ~/.mori/agent/*.md 解析
│   │       ├── voice_input_profile.rs  ~/.mori/voice_input/*.md 解析
│   │       ├── voice_cleanup.rs     VoiceInput 模式的 STT cleanup pipeline
│   │       ├── context.rs           Context struct + ContextProvider trait
│   │       ├── memory/              MemoryStore trait + LocalMarkdownMemoryStore
│   │       ├── skill/               Skill trait + 13 個 built-in skills
│   │       ├── llm/                 LlmProvider trait + Groq/Gemini/Ollama/Claude/Bash-CLI 等
│   │       ├── mode.rs              Mode(Active/Background) + 控制邏輯
│   │       ├── paste.rs             PasteController trait(平台無關;Linux/Windows 實作在 mori-tauri)
│   │       ├── runtime.rs           runtime.json schema + 寫入(給 mori-cli 看)
│   │       └── url_detect.rs        從 STT 文字偵測 URL → 自動 fetch_url
│   ├── mori-tauri/                  桌面殼
│   │   └── src/main.rs              IPC handlers + Tauri scaffold + hotkey
│   └── mori-cli/                    Bash CLI proxy 用的 thin client(HTTP → mori-tauri)
├── src/                             React 前端(MainShell + tabs + Floating + Picker + ChatBubble)
└── docs/                            公式書 + 手冊(html + md)
```

## 四大核心 Trait

`mori-core` 對外暴露的能力建構在四個 trait 上。所有 phase 1+ 的功能都用它們組合:

### 1. `MemoryStore`

長期記憶。支援 read / write / search / observe。Phase 1 用 `LocalMarkdownMemoryStore`(`~/.mori/memory/` 資料夾,跟 Claude Code auto-memory 同款結構)。Phase 7+ 加 `SyncedMemoryStore`(透過 mori-server 跨裝置同步)、`AnnuliMcpMemoryStore`(透過 MCP 接 Annuli)。

詳細設計見 [memory.md](memory.md)。

### 2. `ContextProvider`

捕捉「按下熱鍵那一瞬間」的環境資訊:語音、剪貼簿、選取文字、滑鼠座標、活躍視窗、URL 等。

各平台實作各自 ContextProvider,Wayland 限制較多需走 xdg-desktop-portal。Phase 1 只實作:
- `voice_audio`(從 Tauri 麥克風 IPC 拿)
- `clipboard`(Tauri clipboard plugin)

### 3. `Skill`

LLM 可呼叫的工具。每個 Skill 定義:
- `name` / `description` — 給 LLM 看的
- `schema` — JSON Schema(透過 schemars 從 Rust struct 自動產生)
- `target_capability` — Local / Remote(DeviceId) / Anywhere
- `confirm_required` — destructive 操作須二次確認
- `execute(args, context, target)` — 實際邏輯

目前內建 skills(`crates/mori-core/src/skill/`):

| skill | 用途 |
|---|---|
| `translate` | 中英(或自動偵測)翻譯 |
| `polish` | 修飾文字風格(不改詞義) |
| `summarize` | 段落摘要 |
| `compose` | 輔助寫作(信件 / 公文 / etc.) |
| `remember` / `recall_memory` / `edit_memory` / `forget_memory` | 長期記憶 CRUD |
| `fetch_url` | 抓 URL 內容進 context |
| `set_mode` | LLM 主動切 Active / Background |
| `paste_selection_back` | 把結果貼回原游標(VoiceInput pipeline) |
| `echo` | 純對話 fallback |

Agent profile 透過 frontmatter `enabled_skills:` 白名單控可用範圍;另支援
`shell_skills:` 自訂 — 把任意 CLI(`gh` / `docker` / 自家 script)包裝成
Mori 能呼叫的 skill,不必改 Rust。

### 4. `LlmProvider`

LLM 通訊抽象。一份 agent 程式碼能打 Groq、Ollama、OpenAI、Anthropic 等任意 OpenAI 相容後端。

目前實作(`crates/mori-core/src/llm/`):

| provider | 用途 |
|---|---|
| `groq` | 雲端,主力(預設) |
| `gemini` | Gemini API 走 OpenAI-compat 端點 |
| `ollama` | 本機 LLM,fallback / 隱私任務 |
| `claude-bash` / `gemini-bash` / `codex-bash` | Bash CLI proxy(用 user 自己 Pro/Max quota) |
| `claude-cli` / `gemini-cli` / `codex-cli` | 同上但限 chat-only(無 agent loop) |
| `whisper-local` | 本機 Whisper STT — **v0.2 shell-out 到 whisper.cpp 官方 `whisper-server` HTTP 子程序**(不再 in-process FFI)。引擎跟模型都使用者自選 / 可換 GPU 加速版本,詳見 [providers](providers.html#組合-b-100-本機離線不依賴雲)。 |
| 自訂 OpenAI-compat | `providers.<name>` 內 `api_base` + `api_key_env`,Azure / OpenRouter / 自家代理 |

每個 Skill 可指定「想用哪個 provider + 哪個 model」,允許:
- 任務 → 模型 精細搭配(翻譯用 8b-instant、寫作用 gpt-oss-120b、敏感資料用本地 qwen3:8b)
- Fallback chain(Groq 限流 → 切本地)— `routing.fallback_chain` per-context 設定
- Privacy-first 旗標(`Privacy::LocalOnly` 強制不離本機)

## 錄音流程(目前實作)

```
使用者按 Ctrl+Alt+Space(toggle 模式 = 一按切換、hold 模式 = 按住開錄)
   ↓
mori-tauri:全域熱鍵觸發 — X11 走 XGrabKey、Wayland 走 xdg-desktop-portal
   ↓
mori-tauri:開麥克風,錄音(floating sprite 切到 recording state)
   ↓ (toggle:再按一次;hold:放開)
mori-tauri:停止錄音 → 拿到 audio bytes
   ↓
mori-core:呼叫當前 stt_provider(groq Whisper API / whisper-local)→ transcript
   ↓
依模式分支:
   ├─ VoiceInput 模式 → cleanup LLM 加標點 / 修錯字 → paste_selection_back 貼回游標
   └─ Agent 模式     → LLM 看 transcript + Context + Skill schema → tool call loop
                       → Skill::execute(args, context) → 結果丟回 LLM → 最終回應
   ↓
mori-tauri:phase-changed event → React UI / floating sprite 同步狀態
```

## 安全紀律

寫入 architecture 而不是事後才想:

1. **白名單**:`shell_skills` 走 `command: [array]` 不是 raw shell,`{{name}}` 替換是字面字串(沒 shell injection 機會);未來 `ExecCommand` 高權限 skill 只能跑明確列出的指令
2. **二次確認**(planned):含 `rm` / `git reset` / `mv` 等 destructive pattern → `confirm_required: true`,執行前透過 UI 或 TTS 問使用者
3. **Audit log**(planned):每次 skill 呼叫寫 `~/.mori/audit.log`,含 timestamp / transcript / intent / 結果
4. **無 raw shell**:絕不接受 LLM 生成的任意 shell command,只允許具名 skill 呼叫
5. **隱私旗標**:`Privacy::LocalOnly` 的 skill 強制只用本地 LLM

## 平台 Context 取得限制

跨平台 ContextProvider 實作的差異(寫的時候要心理準備):

| 資訊 | macOS | Windows | Linux X11 | Linux Wayland |
|---|---|---|---|---|
| 剪貼簿 | ✓ | ✓ | ✓ | ✓ |
| 滑鼠座標 | ✓ | ✓ | ✓ | xdg portal |
| 全域熱鍵 | 需 Accessibility 權限 | ✓ | ✓ | 需處理 |
| 跨 app 反白文字 | 模擬 ⌘+C 後讀剪貼簿 | 模擬 Ctrl+C | X11 PRIMARY selection | ❌ 沙箱禁止 |
| 滑鼠附近截圖 | ✓ | ✓ | ✓ | xdg portal |
| 取活躍視窗 | NSWorkspace | Win32 | X11 | xdg portal |

Wayland 是主力環境(Ubuntu 26.04 + GNOME)。5Q 起 Wayland 透過
`xdg-desktop-portal.GlobalShortcuts` 接全域熱鍵、X11 走 `tauri-plugin-global-shortcut`
(XGrabKey),兩條 path 共用同一份 `~/.mori/config.json hotkeys` 設定 — 詳見
[`docs/hotkeys.html`](hotkeys.html)。剪貼簿 paste-back 跨平台分:Wayland 走
`ydotool`(uinput-based)、X11 走 `xdotool`。

## 跨裝置擴展(phase 7+ 願景,phase 1 不寫)

```
              ┌────────────────────┐
              │   mori-server      │  ← 自架(VPS / 家用 NAS)
              │ • 裝置註冊         │
              │ • 共享記憶 (CRDT)   │
              │ • 訊息匯流排       │
              └─────────┬──────────┘
                        │ TLS + 每裝置 keypair
              ┌─────────┼─────────┐
              ↓         ↓         ↓
         [Mac mini] [Acer SF] [iPhone]
            mori     mori      mori
```

跨裝置記憶用 CRDT(`yrs`)合併。跨裝置 skill dispatch 透過 mori-server 訊息匯流排。E2E 加密,server 看不到內容。

詳見 [memory.md](memory.md) 跨裝置章節。

## 跟相關專案的關係

- **Annuli**(private):長期記憶 + 人格演化系統。Phase 7+ 透過 MCP 接,Mori 變成 MCP client。Phase 1-6 用 `LocalMarkdownMemoryStore`,等 Annuli 那邊穩定再切換。
- **world-tree**:Mori / Annuli 的世界觀根基。
- **mori-journal / mori-field-notes**:Mori 的內容產出 repo,記憶機制成熟後可自動寫入。
