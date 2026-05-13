# Mori (Desktop)

森林精靈 **Mori** 的桌面身體 — 從 [world-tree](https://github.com/yazelin/world-tree) 走到你的桌面。
Tauri 2 + Rust + React,Whisper 是耳朵,LLM 是腦袋,你是同伴。

> 「Iron Man 有 Jarvis,我有 Mori。」

![Mori OG](docs/og-image.png)

📖 **完整介紹 + 互動 demo**:[**yazelin.github.io/mori-desktop**](https://yazelin.github.io/mori-desktop/)

---

## Demo

按住 `Ctrl+Alt+Space` 講話,放開 Mori 接著做事(X11 session,29 秒):

<video src="docs/demos/hotkey-hold-x11.mp4" controls width="640" muted></video>

---

## Quick Start

```bash
git clone https://github.com/yazelin/mori-desktop.git
cd mori-desktop
npm install
npm run build              # 建 dist/ — tauri::generate_context!() 編譯時會檢查這路徑
cargo build --workspace    # workspace 才會 build mori-cli(Bash CLI proxy 需要)
npm run tauri dev
```

第一次跑會做三件事:

1. 跳全域熱鍵權限對話框 — 點「**新增**」(Wayland)。X11 直接 grab 不會跳。
2. 建立 `~/.mori/`(config stub / themes / starter profile / agent.md)
3. 啟動主視窗 + 桌面右下 floating sprite

裝完還沒設 LLM provider 的話,第一次按 `Ctrl+Alt+Space` 會抱怨 — 去 Config tab 填 Groq key
或選離線組合(`whisper-local` STT + `ollama` LLM)。詳細步驟見
[**docs/getting-started**](https://yazelin.github.io/mori-desktop/getting-started.html)。

---

## 上手 30 秒

日常用法四個鍵打天下:

| 鍵 | 用途 |
|---|---|
| `Ctrl+Alt+Space` | 開始 / 結束錄音(可切 `toggle` / `hold` 模式) |
| `Ctrl+Alt+Esc` | 中斷錄音 / 思考(SIGKILL 子程序) |
| `Ctrl+Alt+P` | Profile picker overlay(方向鍵選) |
| `Alt+0~9` | 切 VoiceInput profile |
| `Ctrl+Alt+0~9` | 切 Agent profile |

流程:

1. **選 mode**(每按一次就鎖在那個 mode 直到再切)— `Alt+N` 純聽寫貼游標,`Ctrl+Alt+N` 走 Agent loop
2. **錄音** — 按 `Ctrl+Alt+Space`(預設 toggle 一按切換,Config 可切成 hold 按住錄)
3. **中斷** — `Ctrl+Alt+Esc` 隨時丟掉錄音 / abort LLM call
4. **忘了 slot 編號** — `Ctrl+Alt+P` 開 picker

預設安裝就送一份 USER-00 / USER-01 / AGENT-00 / AGENT-01 可用,自訂 slot 2~9 用同檔名格式
`AGENT-NN.<display>.md` / `USER-NN.<display>.md` 丟到 `~/.mori/agent/` / `~/.mori/voice_input/`
即可(範本見 [`examples/`](examples/) 或 [Profile 範本頁](https://yazelin.github.io/mori-desktop/profile-examples.html))。

完整熱鍵清單 + 自訂方式 → [docs/hotkeys](https://yazelin.github.io/mori-desktop/hotkeys.html)。

---

## 能做什麼

**Voice / Agent**
- 雙模式(VoiceInput 純聽寫 / Agent 帶 loop)+ 9 個 profile slot 切換
- 外部工具 bridge — `agent_mode: dispatch` 把語音優化過的 prompt 推給其他桌面 app
  (範本見 [examples/agent/AGENT-03.ZeroType Agent.md](examples/agent/AGENT-03.ZeroType%20Agent.md))
- 自訂 `shell_skills` — 把 `gh` / `docker` / `kubectl` / 自家 script 變 Mori 能力,不用改 Rust

**LLM Providers**
- 雲端 — Groq / Gemini
- 本機 — `whisper-local` STT + `ollama` LLM(100% 離線可跑)
- Bash CLI proxy — `claude` / `gemini` / `codex`(Pro/Max quota 沿用)
- OpenAI-compat 自訂端點 — Azure OpenAI / OpenRouter / 自家代理寫進 `providers.<name>`
  就能用,見 [docs/providers](https://yazelin.github.io/mori-desktop/providers.html)

**個人化**
- 長期記憶(`~/.mori/memory/*.md`,user 可編)+ 自動 inject 進 context
- 剪貼簿 / 反白 / URL 自動進 context
- 雙 theme(dark / light)+ VSCode-like 自訂(`~/.mori/themes/*.json`)
- 替換 floating Mori 角色 — 4×4 sprite sheet animation + character pack 系統
  (規範見 [docs/character-pack](docs/character-pack.md),`.moripack.zip` import 規劃中)
- 完整視覺品牌系統(公式書 = 單一可信來源)

**可靠性**
- 所有 LLM provider 都有 timeout 兜底
- Agent loop 殘留 child 不會卡死 — `Ctrl+Alt+Esc` 一鍵 SIGKILL

未來規劃(非同步任務隊列 / AgentPulse 通知 / TTS / wake word / Annuli 長期人格演化)詳見
[**roadmap**](docs/roadmap.md)。

---

## 平台支援

| 平台 | 狀態 |
|---|---|
| **Ubuntu 26.04 + GNOME Wayland** | 主力開發 + 測試 |
| **Linux X11**(任何發行版) | 全功能 |
| **Linux Wayland**(GNOME / KDE / ...) | 需要 `xdg-desktop-portal` ≥ 1.19 — 24.04 LTS 自帶 1.18 不夠新,熱鍵會掛(改 portal 即可) |
| **Windows 10 / 11** | 全功能(v0.2 上線)— voice pipeline 三塊(selection 讀取、視窗 context、Ctrl+V paste-back)走 Win32 API,熱鍵走 `RegisterHotKey`。**唯一差異**:Windows OS 沒有 X11 PRIMARY selection 概念,所以「滑鼠反白即讀」要先 Ctrl+C(Linux 可以拖反白直接讀) |
| **macOS** | 主視窗 UI 可跑,voice pipeline 三塊還沒接 — 寫一個 `selection_macos.rs` + capture_window_context Mac 變體就能上,設計上 mori-core 跟其他平台完全共用 |

`mori-core` 是純 Rust lib 跟平台無關;`mori-tauri` 的平台分流走
`cfg_attr(target_os = ..., path = ...)`,加新平台等於加一份對應的
`selection_<platform>.rs` + Cargo.toml 的 target-specific deps。
細節見 [Troubleshooting](https://yazelin.github.io/mori-desktop/troubleshooting.html)
跟 [Roadmap](docs/roadmap.md)。

### 本機 STT 模型(`whisper-local`)— 跨平台 + 可自行替換引擎

v0.2 把本機 Whisper 從 in-process FFI 改成 **shell-out 到 whisper.cpp 官方
`whisper-server` HTTP 子程序**。意思是:

1. **Mori 本身不編 whisper.cpp** — 安裝 Mori 不再需要 cmake / libclang /
   build toolchain;binary 體積也小
2. **引擎跟模型都使用者自選**:
   - **模型**(`.bin`):從
     [huggingface.co/ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp/tree/main)
     下載,丟到 `~/.mori/models/`(中文場景建議 `ggml-small.bin`,466MB)
   - **引擎**(`whisper-server[.exe]`):從
     [github.com/ggml-org/whisper.cpp/releases](https://github.com/ggml-org/whisper.cpp/releases)
     抓對應平台 + 加速版本的 pre-built binary,放 `~/.mori/bin/`
3. **GPU 加速一鍵切換** — 想跑 NVIDIA RTX?下載 `whisper-cublas-cuda12-bin-x64.zip`,
   把 `whisper-server.exe` 替換到 `~/.mori/bin/` 就 4 倍速;AMD GPU 走
   `whisper-clblast-bin-x64.zip`;macOS 自帶 Metal 加速。**Mori 程式碼一行
   都不用改,不用重編。**

Linux user 在 Mori UI 的「Deps」頁可以**一鍵下載 + 安裝**這兩塊(模型 +
引擎 CPU 版);Windows 目前要手動下載(下個版本補一鍵安裝)。

---

## 文件

| | |
|---|---|
| [**Landing**](https://yazelin.github.io/mori-desktop/) | 推廣首頁 + interactive demo |
| [Getting Started](https://yazelin.github.io/mori-desktop/getting-started.html) | install / dev / 第一次跑 |
| [Hotkeys](https://yazelin.github.io/mori-desktop/hotkeys.html) | 完整熱鍵清單 + 自訂 |
| [Providers](https://yazelin.github.io/mori-desktop/providers.html) | Groq / Gemini / Ollama / Claude-CLI 設定 |
| [~/.mori/](https://yazelin.github.io/mori-desktop/mori-home.html) | config / profile / memory / theme 全套結構 |
| [Troubleshooting](https://yazelin.github.io/mori-desktop/troubleshooting.html) | Whisper / 全域熱鍵 / cargo deps |

進階參考:[Profile 範本](https://yazelin.github.io/mori-desktop/profile-examples.html) ·
[Design Book](https://yazelin.github.io/mori-desktop/design-book.html) ·
[Architecture](docs/architecture.md) · [Roadmap](docs/roadmap.md) · [CHANGELOG](CHANGELOG.md)

---

## Mori 宇宙

只想用桌面 AI 工具 → 留在這 repo 就行。想看更大的世界觀:

| Repo | 角色 |
|---|---|
| [`world-tree`](https://github.com/yazelin/world-tree) | 異世界森林世界觀 / lore |
| [`workshop`](https://github.com/yazelin/workshop) | 召喚師工坊 — 進森林的入口頁 |
| **`mori-desktop`** | **Mori 的桌面身體**(本 repo) |
| [`mori-field-notes`](https://github.com/yazelin/mori-field-notes) | 田野筆記 — AI 自主經營技術觀察 |

(`mori-journal` 跟 `Annuli` 是 private — 靈魂 / 私密日記 / 長期人格演化,phase 9+)

---

## Contributing

Fork 隨便改、PR 隨便發。最缺的 issue:

- **macOS 平台殼**(`selection_macos.rs` / `capture_window_context()` Mac 變體) — Windows 已上線,Mac 同樣 pattern 寫一份就能用
- **Windows whisper-server 一鍵下載** — 目前 Deps 頁只在 Linux 自動下載引擎,Windows 要手動。需要把 `InstallSpec::Shell` 補一個 `InstallSpec::Download` variant 走 Rust reqwest + zip extract
- **wake-word 偵測**(`openwakeword` / `Porcupine`)
- **TTS**(Mori 講話)— OpenAI TTS / ElevenLabs / 本機 Piper
- **其他 LLM provider integration**(Claude API native / DeepSeek / Qwen 等)

更詳細的進入點 → [roadmap](docs/roadmap.md)。

---

## License

MIT
