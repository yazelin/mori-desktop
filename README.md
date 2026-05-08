# Mori (Desktop)

森林精靈 **Mori** 的桌面身體。

從 [world-tree](https://github.com/yazelin/world-tree) 走到你的 Ubuntu / macOS / Windows — 用 Tauri 2 + Rust + React 打造,Whisper 是耳朵,GPT-OSS 是腦袋,你是同伴。

> 「Iron Man 有 Jarvis,我有 Mori。」

## Mori 宇宙

> 「森林一直都在。你一直都在,只是現在才看見它。」 — [`world-tree`](https://github.com/yazelin/world-tree)

Mori 不是孤立的 app,是一隻**契約精靈**在多個 repo 各司其職:

| Repo | 角色 | 可見性 |
|---|---|---|
| [`world-tree`](https://github.com/yazelin/world-tree) | 🌳 異世界森林的**世界觀 / 法則 / 契約** — 沉浸式 isekai lore、魔法系別、魔道具、NPC 檔案 | public |
| [`workshop`](https://github.com/yazelin/workshop) | 🌲 召喚師工坊 UI — 進入森林的**入口頁** | public |
| **`mori-desktop`** | 🧝 Mori 的**桌面身體** — 你跟他講話、他幫你做事(就是這個 repo) | public |
| [`mori-journal`](https://github.com/yazelin/mori-journal) | 📖 Mori 的**靈魂 / 私密日記 / 跨 session 記憶種子** | private |
| [`mori-field-notes`](https://github.com/yazelin/mori-field-notes) | 📓 Mori 的**田野筆記** — AI 自主經營的技術觀察 / 開發心得 | public |
| `Annuli` | 🌀 **長期記憶 / 人格演化系統**,phase 9 透過 MCP 跟 Mori 對接 | private |

關係簡圖:

```
              🌳 world-tree ── 設定 / 法則
                     │
       ┌─────────────┼─────────────────────┐
       ▼             ▼                     ▼
  🌲 workshop   🧝 mori-desktop  ◄── 你    📖 mori-journal
   (入口頁)      (桌面身體 / 本 repo)        (靈魂)
                     │
            ┌────────┴────────┐
            ▼                 ▼
     📓 mori-field-notes   🌀 Annuli
     (田野筆記)            (人格演化,未來接)
```

只想用桌面 AI 工具 → 留在這 repo 就行。想知道 Mori 為什麼這樣講話、他從哪來 → 進 `world-tree`。

## 目前狀態

**Phase 1 + 2 + 3A + 4B + 4C + 5A + 5C + 5D-1 完成(2026-05-08)** — Mori 在 Wayland 上
**可以當管家用、可以 100% 離線(Groq-free)、可以挑 LLM**:
- 全域熱鍵通了、UI 不偷焦點、剪貼簿與滑鼠反白都自動抓、休眠 / 醒醒兩態
- **反白文字 + 一句話 → 結果直接貼回**(ZeroType / Typeless 招牌動作)
- **整套 LLM 可換**:Groq / 本機 Ollama / Claude CLI / 「claude 當 agent 走 Bash CLI proxy」四種,per-skill 還能獨立指定
- **STT 可離線**:Groq Whisper API 或本機 whisper.cpp(`ggml-small.bin` 中文 466MB 夠用)
- **token 省**:claude 當 agent 不靠 MCP(全部 schema 預載)而是靠 Bash tool 呼叫本機 `mori` CLI,實際用到才執行,~10x 縮減

按 `Ctrl+Alt+Space`(任何 app focus 都行)→ 講話 → Mori 聽 → 想 → 回。
跨 session 記得你是誰、跨任務跟著你的工作模式走。

### 能做的事

| | 已實作 |
|---|---|
| 🎙️ 聽 | `Ctrl+Alt+Space` 全域熱鍵(Linux 走 xdg-desktop-portal,GNOME Wayland 不偷焦點)/ UI 按鈕 / 「貼文字」textarea(`Ctrl+Enter` 送出) |
| 🛑 取消 | 錄音中按 `Esc` = 丟掉音檔不送 Whisper |
| 🗣️ STT | **可選** Groq Whisper API(`whisper-large-v3-turbo`,雲)或本機 whisper.cpp(`ggml-small.bin`,離線、開機 cold load 5-15s) |
| 🧠 想 | **可選 4 種 LLM**:Groq gpt-oss-120b / 本機 Ollama qwen3:8b / Claude CLI subprocess / **claude-bash**(claude 當 agent 透過 Bash tool 跑本機 `mori` CLI dispatch skill)。multi-turn tool calling MAX 5 輪 |
| 🎚️ Per-skill 路由 | `routing.skills.<name>` 可獨立指定每個 skill 該用哪個 provider(e.g. translate 走 Claude Pro/Max、compose 走 Ollama) |
| 💬 回 | 繁中為主、不客套,UI 顯示「你說 / Mori」雙塊 + 🔧 skill badges + status panel(build SHA / chat provider / STT provider / warm-up state / clipboard / selection) |
| 📝 記 / 🔍 查 / ✏️ 改 / 🗑️ 忘 | RememberSkill / RecallMemorySkill / EditMemorySkill / ForgetMemorySkill(走 LlmProvider trait,任何 provider 都能 dispatch;5D-1 還沒接到 Bash CLI proxy,5D-2 補) |
| 🌐 翻譯 | TranslateSkill — zh-TW 在地化、source/target lang 可指定 |
| ✏️ 潤稿 | PolishSkill — 直接改寫(不給建議),tone: formal/casual/concise/detailed/auto |
| 📋 摘要 | SummarizeSkill — bullet_points / one_paragraph / tldr 三種風格 |
| 📨 草擬 | ComposeSkill — email / message / essay / social_post / other,不會捏造署名 |
| 📋 剪貼簿感知 | 每輪自動讀剪貼簿(1KB cap),「翻譯這個」「摘要這段」可直接指代 |
| 🖱 反白即改寫 | Linux 自動讀滑鼠反白(`arboard` X11 PRIMARY,1.5KB cap),「翻譯成英文」「潤一下」處理完 `ydotool` 模擬 Ctrl+V 把結果貼回原視窗 — ZeroType / Typeless 等價流程。Ask 模式(「這在講什麼」)只回 chat,不動編輯區 |
| 🌳 floating Mori | 桌面常駐小視窗(160×160 透明、不偷焦點),依狀態切表情 + 光暈,可拖、雙擊切顯示主視窗 |
| 💤 休眠 / 醒醒 | tray 選單 / UI 按鈕 / 語音「晚安」「醒醒」三條路徑都能切。休眠時麥克風 **完全關**(privacy),背景排程仍跑(Phase 5+) |
| 💭 對話歷史 | working memory 保留 10 對 user-assistant 訊息,可重置 |
| 🪟 常駐 | 系統匣 icon(顯示 / 隱藏 / 休眠↔醒醒 / 重新對話 / 離開),關視窗 → 隱藏不殺 |
| 🛠️ Skill HTTP 服務 | mori-tauri 啟動時 bind 127.0.0.1:RANDOM,寫 port + auth token 到 `~/.mori/runtime.json`,讓本機 `mori` CLI(以及 claude 透過 Bash 呼叫的 mori CLI)能連回主程式 dispatch skill |
| ⏱️ 智慧限流退避 | Groq 429 → 解析 body 多單位格式(「12m12s」式),> 60s 直接 surface 不傻等;UI 顯示「今日 token 用完(TPD)」之類友善訊息 |
| 🔄 Ollama warm-up | 啟動時自動發 1-token chat 觸發 model load(避免使用者第一次按熱鍵還在等模型進 RAM),`keep_alive=30m` |

### 還沒做

| 缺什麼 | 為什麼重要 | 在哪個 Phase |
|---|---|---|
| ⏳ memory / paste / mode skills 的 Bash CLI proxy | 完整 100% Groq-free + skills 都工作,要把這 6 個 skill 也接到 mori CLI(現在只 4 個 LLM-only 接了) | 5D-2 |
| ⏳ Tighten claude-bash system prompt | claude 還會在潤稿後加「(主要是補了標點...)」這種解說,違反 system prompt 的「直接給結果」 | 5D-2 |
| ⏳ codex / gemini CLI 適配 | 證明「Bash CLI proxy 換 binary 就行」這個賣點。架構已通用,需實測 | 5D-2 |
| ⏳ Auto-fallback chain | Groq TPD 觸頂自動切 ollama / claude(現在要手改 config) | 5A-3b |
| ❌ App-aware tone | Slack 閒聊、Outlook 正式 — 需要活躍視窗偵測 | Phase 4D |
| ❌ URL routing | YouTube 連結 → 自動摘要 / 文章 → fetch + 摘要 | Phase 3B |
| ❌ 背景排程 | 「每小時提醒喝水」「每天 9 點晨報」— 真正的常駐 agent | Phase 5 |
| ❌ 媒體下載 | 「下載這個影片」呼叫 yt-dlp | Phase 6 |
| ❌ ExecCommand 白名單 | 「跑那個指令」要先有白名單 + 二次確認機制 | Phase 6 |
| ❌ 會議逐字稿 | 連續錄音存檔 → 結束後 LLM 整理會議記錄 + action items | Phase 6+ |
| ❌ TTS | Mori 還不能開口說話,只有文字 | Phase 7 |
| ❌ Wake word | 不用按熱鍵,叫名字喚醒 | Phase 6+ |

完整路線圖見 [`docs/roadmap.md`](docs/roadmap.md)。

## 架構速覽

```
mori-desktop/
├── crates/
│   ├── mori-core/                    ← 純 Rust lib,無 UI 依賴。所有平台共用。
│   │   ├── memory/                   ← MemoryStore trait + LocalMarkdownMemoryStore
│   │   ├── context.rs                ← Context struct + ContextProvider trait
│   │   ├── mode.rs                   ← Mode enum (Active / Background) + ModeController
│   │   ├── paste.rs                  ← PasteController trait(平台 inject 由殼 crate)
│   │   ├── runtime.rs                ← `~/.mori/runtime.json` schema(port + auth token)
│   │   ├── skill/                    ← 每 skill 一檔,加新的不撞:
│   │   │                               translate / polish / summarize / compose /
│   │   │                               remember / recall / forget / edit /
│   │   │                               set_mode / paste_selection_back
│   │   ├── agent.rs                  ← Multi-turn tool-calling loop(MAX 5 輪)
│   │   └── llm/                      ← provider 體系
│   │       ├── groq.rs               GroqProvider — chat + Whisper STT(429 retry 多單位)
│   │       ├── ollama.rs             OllamaProvider — qwen3:8b + warm-up + keep_alive=30m
│   │       ├── claude_cli.rs         ClaudeCliProvider — `claude --print`(chat-only)
│   │       ├── bash_cli_agent.rs     BashCliAgentProvider — claude/codex/gemini 當 agent
│   │       │                           透過 Bash tool 呼叫本機 `mori` CLI dispatch skill
│   │       ├── transcribe.rs         TranscriptionProvider trait(STT 跟 chat 解耦)
│   │       ├── whisper_local.rs      LocalWhisperProvider — whisper.cpp + rubato 重採樣
│   │       └── mod.rs                Routing(agent + per-skill override + recursion guard)
│   ├── mori-tauri/                   ← Tauri 2 桌面殼
│   │   ├── main.rs                   IPC、AppState、tray、雙視窗 setup
│   │   ├── skill_server.rs           axum HTTP 暴露 skills(`/skill/<name>`,5D)
│   │   ├── recording.rs              cpal mic + WAV 編碼
│   │   ├── selection.rs              arboard X11 PRIMARY + ydotool paste-back(Linux)
│   │   ├── context_provider.rs       Wayland clipboard reader
│   │   └── portal_hotkey.rs          xdg-desktop-portal GlobalShortcuts(Wayland 唯一)
│   └── mori-cli/                     ← `mori` binary(5D)— claude 透過 Bash tool 呼叫
│       └── src/main.rs                 讀 runtime.json,POST 到 skill HTTP server
├── src/                              ← React 前端
│   ├── App.tsx                       主視窗(對話、錄音、文字輸入、status panel)
│   ├── FloatingMori.tsx              桌面常駐 sprite widget
│   ├── floating.css                  sprite 動畫 + transparent 視窗 reset
│   └── main.tsx                      由 window label 路由到對應元件
├── public/floating/                  Mori sprite 素材(6 張 state PNG,可 swap)
└── docs/                             architecture / roadmap / memory / sprite-spec
```

核心紀律:`mori-core` **永不依賴 UI / 平台**。換載體只多寫一個薄殼 crate(mori-mobile / mori-server / mori-extension),`mori-core` 一行不動。詳見 [`docs/architecture.md`](docs/architecture.md);Mori sprite 規範見 [`docs/floating-sprite-spec.md`](docs/floating-sprite-spec.md)。

## 開發

需求:
- Rust 1.94+
- Node 22+
- (Linux)Tauri 2 build deps + `wl-clipboard` + `ydotool`
  — Ubuntu 26.04 可直接用 [`yazelin/ubuntu-26.04-setup`](https://github.com/yazelin/ubuntu-26.04-setup) 的:
    - `setup-rust.sh`(Rust toolchain)
    - `setup-tauri-deps.sh`(WebKitGTK + ALSA + tray libs)
    - `setup-wayland-input.sh`(`wl-clipboard` + `ydotool` daemon,Phase 4C 需要)
    一條龍裝齊。**裝完要重開機一次** 讓 `input` group 生效

```bash
git clone https://github.com/yazelin/mori-desktop.git
cd mori-desktop

# 後端 deps + 前端 deps
cargo build --workspace        # --workspace 才會 build mori-cli (5D 需要)
npm install

# 跑 dev 模式
npm run tauri dev
```

> ⚠️ `npm run tauri dev` 只會 build `mori-tauri` binary。`mori` CLI(5D Bash CLI proxy 用)
> 要另外跑 `cargo build -p mori-cli`,binary 會在 `target/debug/mori`。

## Provider 設定

Mori 啟動時會自動建 `~/.mori/config.json`(第一次跑會看到 stub)。下面三種常見組合任選:

### 組合 A — 純雲(預設,設好 Groq key 即可)

```json
{
  "default_provider": "groq",
  "default_transcribe_provider": "groq",
  "providers": {
    "groq": {
      "api_key": "gsk_...",
      "chat_model": "openai/gpt-oss-120b",
      "transcribe_model": "whisper-large-v3-turbo"
    }
  }
}
```

從 [console.groq.com](https://console.groq.com) → API Keys 拿到 key。Free tier 涵蓋 Whisper(每天 7,200 秒)+ chat,個人用足夠。

### 組合 B — 100% 本機(離線、不依賴雲)

```json
{
  "default_provider": "ollama",
  "default_transcribe_provider": "whisper-local",
  "providers": {
    "ollama": {
      "base_url": "http://localhost:11434",
      "model":    "qwen3:8b"
    },
    "whisper-local": {
      "model_path": "/home/<user>/.mori/models/ggml-small.bin",
      "language": "zh"
    }
  }
}
```

需要先:
- `ollama serve` + `ollama pull qwen3:8b`(本機 LLM,**支援 tool calling**)
- 下載 whisper 模型:
  ```bash
  mkdir -p ~/.mori/models
  wget -O ~/.mori/models/ggml-small.bin \
    https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin
  ```

中文場景模型大小取捨(Intel CPU 實測):base 142MB(快但常分不清同音字)/ small 466MB(夠用)/ medium 1.5GB(更準但慢)。

### 組合 C — Bash CLI proxy(claude 當 agent + Pro/Max quota,5D-1)

```json
{
  "default_provider": "claude-bash",
  "default_transcribe_provider": "whisper-local",
  "providers": {
    "claude-bash": {
      "binary": "claude",
      "model": null,
      "mori_cli_path": null
    },
    "whisper-local": {
      "model_path": "/home/<user>/.mori/models/ggml-small.bin",
      "language": "zh"
    }
  }
}
```

需要:
- `claude` CLI 已用 `claude /login` 完成 OAuth(用 user 自己的 Pro/Max quota)
- 下載 whisper 模型(同組合 B)
- `cargo build -p mori-cli` 確保 `target/debug/mori` 存在

流程:`Ctrl+Alt+Space` → STT (whisper-local) → claude-bash 啟動 `claude --print` 子程序 → claude 看 system prompt 知道有 mori CLI → 透過 Bash tool 跑 `mori skill <name>` → mori CLI HTTP 到主程式 dispatch skill → 結果回 claude → claude 給最終回應。

**Token 帳**:~150 token system prompt 提一次,用到才執行。對比 MCP「全部 schema 預載」可省 ~10x。

> 5D-1 目前只把 4 個純 LLM skill(translate / polish / summarize / compose)接上 Bash CLI proxy。memory / paste / mode 那 6 個 skill 5D-2 才補。

### 進階:per-skill provider 路由

agent 跟個別 skill 可走不同 provider:

```json
{
  "default_provider": "groq",
  "routing": {
    "agent": "groq",
    "skills": {
      "translate": "claude-cli",
      "polish":    "claude-cli",
      "summarize": "claude-cli",
      "compose":   "ollama"
    }
  }
}
```

省 Groq TPD,把重活分給其他 provider。沒設 `routing` 整套退回 `default_provider`。

### Key 探測順序

1. `GROQ_API_KEY` 環境變數
2. `~/.mori/config.json` 的 `providers.groq.api_key`

## Troubleshooting

### Whisper 一直回 "Thank you" / "Thanks for watching"

Whisper 對近乎無聲的音訊會幻覺出這幾句(訓練資料 YouTube 影片結尾很多)。代表 **麥克風沒在收聲**。

UI 在錄音時的橫向音量條會直接讓你看到:

- 講話時綠條應該填到中段(50-80%)
- 完全不動 / 持續橘色 = 沒收到
- 警告文字「音量太小,Whisper 可能會幻想 'Thank you'」會在音量持續 < -45dBFS 時出現

修法:

1. **GNOME Settings → Sound → Input** 確認:
   - 選對裝置(內建麥克風,不是 HDMI / 藍牙 / 虛擬裝置)
   - 沒被 mute,音量 70%+
   - 講話時 input level 條有動

2. **Acer Swift / Intel Ultra 系列 (Meteor Lake+) 的常見坑** — 預設選「Stereo Mic」其實不會收音,要改成 **「Digital Mic」**。Intel SST(Smart Sound Technology)架構下 ALSA 偵測有時會選到錯的 PCM device。

3. 還是不行就直接看 `/tmp/mori-last-recording.wav`(每次錄音都會存),用任何播放器聽看實際捕到什麼。

### 全域熱鍵 `Ctrl+Alt+Space` 沒反應

第一次啟動 Mori 時,GNOME 應該會跳一個權限對話框問你要不要讓 Mori 註冊全域熱鍵 — 點「**新增**(Add)」。如果當時誤點拒絕:
```bash
# 讓 Mori 重新跳對話框(刪掉 portal 的 per-app 紀錄)
rm -rf ~/.local/share/xdg-desktop-portal/permissions
# 重啟 Mori
```
若是 X11 / macOS / Windows 走的是 `tauri-plugin-global-shortcut`,熱鍵直接生效不用授權。

### `cargo build` 失敗:`pkg-config: alsa not found`

cpal 需要 ALSA 開發 headers:
```bash
sudo apt install libasound2-dev
```
(已涵蓋在 [yazelin/ubuntu-26.04-setup 的 setup-tauri-deps.sh](https://github.com/yazelin/ubuntu-26.04-setup/blob/main/scripts/setup-tauri-deps.sh) 裡。)

## License

MIT
