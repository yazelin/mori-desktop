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

**Phase 1 + Phase 2 完成(2026-05-08)** — Mori 是端到端可用的 voice + text AI 管家,
有 8 個 skill,但 **還沒實用** — 全域熱鍵被 Wayland 擋、剪貼簿 / URL / 螢幕內容都看不到,
所以日常還是要切到 Mori 視窗才能用。Phase 3+ 在補這些。

按 F8(目前 Wayland 不通,用 UI 按鈕代替)或「貼文字」→ 講話 / 打字 → Mori 聽 → 想 → 回。
跨 session 記得你是誰。同 session 接得上「再說一次」「這個再短點」。

### 能做的事

| | 已實作 |
|---|---|
| 🎙️ 聽 | UI 按鈕 → cpal 麥克風 → Groq Whisper turbo,即時音量條,debug WAV 存檔 |
| ⌨️ 打字 | textarea + Ctrl+Enter,bypass Whisper 直接走 chat(長文 / 程式碼 / 不方便講話時用) |
| 🧠 想 | gpt-oss-120b + multi-turn tool calling(MAX 5 輪),system prompt 含 persona / 時間 / 記憶索引 / 對話歷史 |
| 💬 回 | 繁中為主、不客套,UI 顯示「你說 / Mori」雙塊 + 🔧 skill badges |
| 📝 記 / 🔍 查 / ✏️ 改 / 🗑️ 忘 | RememberSkill / RecallMemorySkill / EditMemorySkill / ForgetMemorySkill |
| 🌐 翻譯 | TranslateSkill — zh-TW 在地化、source/target lang 可指定 |
| ✏️ 潤稿 | PolishSkill — 直接改寫(不給建議),5 種 tone |
| 📋 摘要 | SummarizeSkill — bullet / paragraph / tldr 三種風格 |
| 📨 草擬 | ComposeSkill — email / message / essay / social post,不會捏造署名 |
| 💭 對話歷史 | working memory 保留 10 對 user-assistant 訊息,可重置 |
| 🪟 常駐 | 系統匣 icon(顯示 / 隱藏 / 重新對話 / 離開),關視窗 → 隱藏不殺 |
| ⏱️ 限流自動退避 | 429 → 解析 Groq body「try again in Xs」+ Retry-After header,+1s 緩衝,UI 橘色 banner |

### 還沒做(Phase 3+ 在排)

讓 Mori「**真的能當主力**」需要的功能,目前還缺:

| 缺什麼 | 為什麼重要 | 在哪個 Phase |
|---|---|---|
| ❌ Wayland 全域熱鍵 | 不能從別的 app 喚醒 Mori,要 alt-tab 過去點按鈕 | Phase 4(走 xdg-desktop-portal) |
| ❌ 剪貼簿自動接入 | 「翻譯這個」要手動貼,沒法直接抓當下 clipboard | Phase 3 |
| ❌ URL routing | YouTube 連結 → 自動摘要 / 文章 → fetch + 摘要 | Phase 3 |
| ❌ 媒體下載 | 「下載這個影片」呼叫 yt-dlp | Phase 4 |
| ❌ ExecCommand 白名單 | 「跑那個指令」要先有白名單 + 二次確認機制 | Phase 4 |
| ❌ TTS | Mori 還不能開口說話,只有文字 | Phase 6 |
| ❌ CLI 整合(claude / gemini / codex / copilot) | 語音控制其他 AI agent | Phase 4+ |

完整路線圖見 [`docs/roadmap.md`](docs/roadmap.md)。

## 架構速覽

```
mori-desktop/
├── crates/
│   ├── mori-core/       ← 純 Rust lib,無 UI 依賴。所有平台共用。
│   │   ├── memory/      ← MemoryStore trait + LocalMarkdownMemoryStore
│   │   ├── context.rs   ← Context struct + ContextProvider trait(phase 3 填內容)
│   │   ├── skill/       ← 每 skill 一檔,加新的不撞:
│   │   │                  echo / remember / recall / forget / edit /
│   │   │                  translate / polish / summarize / compose
│   │   ├── agent.rs     ← Multi-turn tool-calling loop(MAX 5 輪)
│   │   ├── llm/         ← LlmProvider trait + GroqProvider(含 429 retry + body 解析)
│   │   └── voice.rs     ← Whisper API client
│   └── mori-tauri/      ← Tauri 2 桌面殼,IPC + 麥克風 + 系統匣 + 熱鍵
├── src/                 ← React 前端
└── docs/                ← architecture / roadmap / memory 設計文件
```

核心紀律:`mori-core` **永不依賴 UI / 平台**。換載體只多寫一個薄殼 crate(mori-mobile / mori-server / mori-extension),`mori-core` 一行不動。詳見 [`docs/architecture.md`](docs/architecture.md)。

## 開發

需求:
- Rust 1.94+
- Node 22+
- (Linux)`libwebkit2gtk-4.1-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev`
  — Ubuntu 26.04 可直接用 [`yazelin/ubuntu-26.04-setup`](https://github.com/yazelin/ubuntu-26.04-setup) 的 `setup-rust.sh` + `setup-tauri-deps.sh` 一條龍裝齊

```bash
git clone https://github.com/yazelin/mori-desktop.git
cd mori-desktop

# 後端 deps + 前端 deps
cargo build
npm install

# 跑 dev 模式
npm run tauri dev
```

## 設定 Groq API key

Mori 啟動時會自動建 `~/.mori/config.json`(第一次跑會看到 stub 內容)。編輯這個檔,把 placeholder 換成你的 Groq key:

```json
{
  "providers": {
    "groq": {
      "api_key": "gsk_...",
      "chat_model": "openai/gpt-oss-120b",
      "transcribe_model": "whisper-large-v3-turbo"
    }
  }
}
```

從 [console.groq.com](https://console.groq.com) → API Keys 拿到 key。Free tier 已涵蓋 Whisper(每天 7,200 秒音訊)+ chat,個人用足夠。

**Key 探測順序**(由前到後):
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

### 全域熱鍵(F8)沒反應

Wayland 為了安全把全域 keylog API 擋住了,`tauri-plugin-global-shortcut` 在 Linux Wayland 下支援不完整。當前繞法:**用 UI 上的「手動觸發」按鈕**。Phase 5+ 會接 xdg-desktop-portal 的 GlobalShortcuts API。

### `cargo build` 失敗:`pkg-config: alsa not found`

cpal 需要 ALSA 開發 headers:
```bash
sudo apt install libasound2-dev
```
(已涵蓋在 [yazelin/ubuntu-26.04-setup 的 setup-tauri-deps.sh](https://github.com/yazelin/ubuntu-26.04-setup/blob/main/scripts/setup-tauri-deps.sh) 裡。)

## License

MIT
