# Mori (Desktop)

森林精靈 **Mori** 的桌面身體。

從 [world-tree](https://github.com/yazelin/world-tree) 走到你的 Ubuntu / macOS / Windows — 用 Tauri 2 + Rust + React 打造,Whisper 是耳朵,GPT-OSS 是腦袋,你是同伴。

> 「Iron Man 有 Jarvis,我有 Mori。」

## 目前狀態

**Phase 1 完整收工(2026-05-08)** — Mori 是端到端可用的 voice AI 管家。

按 F8(或 UI 按鈕)→ 講話 → Mori 聽 → 想 → 回 → 必要時自己改長期記憶。
跨 turn 記得這個 session 講過什麼。關視窗到系統匣繼續跑。

| | 已實作 |
|---|---|
| 🎙️ 聽 | 全域熱鍵 / UI 按鈕 → cpal 麥克風 → Groq Whisper turbo,即時音量條,debug WAV 存檔 |
| 🧠 想 | gpt-oss-120b + multi-turn tool calling(MAX 5 輪),system prompt 含 persona / 時間 / 記憶索引 / 對話歷史 |
| 💬 回 | 繁中為主、不客套,UI 顯示「你說 / Mori」雙塊 + 🔧 skill badges |
| 📝 記 / 🔍 查 / ✏️ 改 / 🗑️ 忘 | RememberSkill / RecallMemorySkill / EditMemorySkill / ForgetMemorySkill |
| 💭 對話歷史 | working memory 保留 10 對 user-assistant 訊息,可重置 |
| 🪟 常駐 | 系統匣 icon(顯示 / 隱藏 / 重新對話 / 離開),關視窗 → 隱藏不殺 |

完整路線圖見 [`docs/roadmap.md`](docs/roadmap.md)。Phase 2+ 規劃中(基礎 skills / TTS / context capture / 跨裝置同步 / Annuli MCP 整合)。

## 架構速覽

```
mori-desktop/
├── crates/
│   ├── mori-core/       ← 純 Rust lib,無 UI 依賴。所有平台共用。
│   │   ├── memory/      ← MemoryStore trait + LocalMarkdownMemoryStore
│   │   ├── context.rs   ← Context struct + ContextProvider trait
│   │   ├── skill.rs     ← Skill trait + Registry + Remember/Recall/Edit/Forget
│   │   ├── agent.rs     ← Multi-turn tool-calling loop
│   │   ├── llm/         ← LlmProvider trait + GroqProvider
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

## 相關專案

- [`world-tree`](https://github.com/yazelin/world-tree) — Mori 的世界觀
- [`mori-journal`](https://github.com/yazelin/mori-journal) — Mori 的日記
- [`mori-field-notes`](https://github.com/yazelin/mori-field-notes) — Mori 的田野筆記
- `Annuli`(private)— 未來會接的長期記憶 / 人格演化系統

## License

MIT
