# Mori (Desktop)

森林精靈 **Mori** 的桌面身體。

從 [world-tree](https://github.com/yazelin/world-tree) 走到你的 Ubuntu / macOS / Windows — 用 Tauri 2 + Rust + React 打造,Whisper 是耳朵,GPT-OSS 是腦袋,你是同伴。

> 「Iron Man 有 Jarvis,我有 Mori。」

## 目前狀態

**Phase 1 — Scaffold(2026-05)**

- [x] Repo 結構 + 文件
- [x] Cargo workspace + 兩個 crate(`mori-core` / `mori-tauri`)
- [x] React 前端骨架(Vite)
- [x] 核心 traits 定義(MemoryStore / Skill / Context / LlmProvider)
- [ ] 實際的全域熱鍵 + 麥克風 + Whisper 整合(下一個 PR)

完整路線圖見 [`docs/roadmap.md`](docs/roadmap.md)。

## 架構速覽

```
mori-desktop/
├── crates/
│   ├── mori-core/       ← 純 Rust lib,無 UI 依賴。所有平台共用。
│   │   ├── memory/      ← MemoryStore trait + LocalMarkdownMemoryStore
│   │   ├── context.rs   ← Context struct + ContextProvider trait
│   │   ├── skill.rs     ← Skill trait + EchoSkill / RememberSkill
│   │   ├── llm/         ← LlmProvider trait + GroqProvider
│   │   └── voice.rs     ← Whisper API client
│   └── mori-tauri/      ← Tauri 2 桌面殼,只做 IPC 跟 OS 整合
├── src/                 ← React 前端
└── docs/                ← architecture / roadmap / memory 設計文件
```

核心紀律:`mori-core` **永不依賴 UI / 平台**。換載體只多寫一個薄殼 crate(mori-mobile / mori-server / mori-extension),`mori-core` 一行不動。詳見 [`docs/architecture.md`](docs/architecture.md)。

## 開發

需求:
- Rust 1.94+
- Node 22+
- (Linux)`libwebkit2gtk-4.1-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`

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
3. `~/.pi/agent/models.json` 的 `providers.groq.apiKey`(legacy fallback,從 Pi 切過來不用搬 key)

## 相關專案

- [`world-tree`](https://github.com/yazelin/world-tree) — Mori 的世界觀
- [`mori-journal`](https://github.com/yazelin/mori-journal) — Mori 的日記
- [`mori-field-notes`](https://github.com/yazelin/mori-field-notes) — Mori 的田野筆記
- `Annuli`(private)— 未來會接的長期記憶 / 人格演化系統

## License

MIT
