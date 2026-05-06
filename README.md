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

## 環境變數

`.env`(別 commit):
```
GROQ_API_KEY=gsk_...    # 用於 Whisper 語音轉文字 + LLM 對話
```

或讀取 `~/.pi/agent/models.json` 裡 `providers.groq.apiKey`(如果你已經設定 Pi)。

## 相關專案

- [`world-tree`](https://github.com/yazelin/world-tree) — Mori 的世界觀
- [`mori-journal`](https://github.com/yazelin/mori-journal) — Mori 的日記
- [`mori-field-notes`](https://github.com/yazelin/mori-field-notes) — Mori 的田野筆記
- `Annuli`(private)— 未來會接的長期記憶 / 人格演化系統

## License

MIT
