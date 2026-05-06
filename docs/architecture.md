# Architecture

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
├── Cargo.toml                  workspace
├── package.json                前端 deps
├── crates/
│   ├── mori-core/              ★ 大腦
│   │   └── src/
│   │       ├── lib.rs          公開 API 入口
│   │       ├── memory/         MemoryStore trait + 實作
│   │       ├── context.rs      Context struct + ContextProvider trait
│   │       ├── skill.rs        Skill trait + 內建 skills
│   │       ├── llm/            LlmProvider trait + GroqProvider
│   │       └── voice.rs        Whisper 客戶端
│   └── mori-tauri/             桌面殼
│       └── src/main.rs         IPC handlers + Tauri scaffold
├── src/                        React 前端
└── docs/
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

Phase 1 內建 skills:
- `EchoSkill` — 回應使用者(基本對話)
- `RememberSkill` — 把當前 context 寫入 memory

Phase 2+ 加:`Translate` / `Polish` / `Summarize` / `Compose` ...

### 4. `LlmProvider`

LLM 通訊抽象。一份 agent 程式碼能打 Groq、Ollama、OpenAI、Anthropic 等任意 OpenAI 相容後端。

Phase 1 實作:
- `GroqProvider`(雲端,主力)
- `OllamaProvider`(本地,fallback / 隱私任務)

每個 Skill 可指定「想用哪個 provider + 哪個 model」,允許:
- 任務 → 模型 精細搭配(翻譯用 8b-instant、寫作用 gpt-oss-120b、敏感資料用本地 qwen3:8b)
- Fallback chain(Groq 限流 → 切本地)
- Privacy-first 旗標(`Privacy::LocalOnly` 強制不離本機)

## 啟動流程(Phase 1 簡化版)

```
使用者按 Ctrl+Alt+M
   ↓
mori-tauri:全域熱鍵觸發
   ↓
mori-tauri:開麥克風,錄音
   ↓ (使用者再按一次熱鍵)
mori-tauri:停止錄音 → 拿到 audio bytes
   ↓
mori-tauri 呼叫 mori-core::voice::transcribe(audio)
   ↓
mori-core:呼叫 GroqProvider Whisper API → 拿到 transcript
   ↓
mori-tauri:emit("transcript", text) 給前端
   ↓
React UI:顯示 transcript
```

Phase 2+ 把這條流程接上 Skill dispatch:

```
... transcript 拿到後 ...
   ↓
mori-core:LLM 看 transcript + Context + 所有 Skill schema
   ↓ tool call decision
mori-core:Skill::execute(args, context)
   ↓
回傳結果 → 前端顯示 / TTS 念出
```

## 安全紀律

寫入 architecture 而不是 phase 5 才想:

1. **白名單**:`ExecCommand` 等高權限 skill 只能跑 `~/.mori/skills.toml` 列出的指令
2. **二次確認**:含 `rm` / `git reset` / `mv` 等 destructive pattern → `confirm_required: true`,執行前透過 UI 或 TTS 問使用者
3. **Audit log**:每次 skill 呼叫寫 `~/.mori/audit.log`,含 timestamp / transcript / intent / 結果
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

Wayland 是目前環境(Ubuntu 26.04),最多受限。Phase 4+ 處理 portal 整合。

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
