# Mori (Desktop)

森林精靈 **Mori** 的桌面身體 — 從 [world-tree](https://github.com/yazelin/world-tree) 走到你的桌面。
Tauri 2 + Rust + React,Whisper 是耳朵,LLM 是腦袋,你是同伴。

> 「Iron Man 有 Jarvis,我有 Mori。」

![Mori OG](docs/og-image.png)

📖 **完整介紹 + 互動 demo**:[**yazelin.github.io/mori-desktop**](https://yazelin.github.io/mori-desktop/)

🌲 **Latest** — [**v0.4.1**](https://github.com/yazelin/mori-desktop/releases/tag/v0.4.1):EN starter set(12 個英文範本進 binary)· Profiles tab「加入範本」UI(zh / en × 22 份範本一鍵裝)· [tokenizer 對比 docs](docs/tokenizer-comparison.md)(EN 在 gpt-oss-120b 省 26% token)· OS prefers-color-scheme 自動偵測 · v0.4.0 → Windows OOB + 觀測層 + 隱私 redact · 完整 changelog 看 [`CHANGELOG.md`](CHANGELOG.md)

---

## Demo

按住 `Ctrl+Alt+Space` 講話,放開 Mori 接著做事(X11 session,29 秒):

<video src="docs/demos/hotkey-hold-x11.mp4" controls width="640" muted></video>

---

## Quick Start

```bash
git clone https://github.com/yazelin/mori-desktop.git
cd mori-desktop

# Linux 第一次:裝 system deps(GTK / WebKit / ALSA / libssl / 等)
# repo 自帶 script,跟 CI 跑同一份,版本跟 git 同步
sudo bash scripts/install-linux-deps.sh
# Windows / macOS:跳這步,Tauri prereqs 見官方文件

npm install
npm run tauri dev          # 會自動 build mori-cli + frontend dist + mori-tauri
```

> Build chain:`tauri dev` → `npm run dev` → 觸發 `predev` script → `cargo build -p mori-cli`
> → 接著 vite dev server 起來。`npm run tauri dev` 自己又會 `cargo run --bin mori-tauri`。
> 全部 zero config,user 啥都不用手動跑。

第一次跑會做三件事:

1. 跳全域熱鍵權限對話框 — 點「**新增**」(Wayland)。X11 直接 grab 不會跳。
2. 建立 `~/.mori/`(config stub / themes / starter profile / agent.md)
3. 啟動主視窗 + 桌面右下 floating sprite

裝完還沒設 LLM provider 的話,第一次按 `Ctrl+Alt+Space` 會抱怨 — 去 Config tab 填 Groq key
或選離線組合(`whisper-local` STT + `ollama` LLM)。完整欄位範本見 repo 根目錄
[`config.example.json`](config.example.json)(複製到 `~/.mori/config.json` 改值即可)。詳細步驟見
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

### 概況

| 平台 | 狀態 |
|---|---|
| **Ubuntu 26.04 + GNOME Wayland** | 主力開發 + 測試 |
| **Linux X11**(任何發行版) | 全功能 |
| **Linux Wayland**(GNOME / KDE / ...) | 需要 `xdg-desktop-portal` ≥ 1.19 — 24.04 LTS 自帶 1.18 不夠新,熱鍵會掛(改 portal 即可) |
| **Windows 10 / 11** | **v0.2 全功能上線**(2026-05) — 主力 dev 機之一 |
| **macOS** | 主視窗 UI 可跑,voice pipeline 三塊還沒接 — contributor 路徑,見 [roadmap](docs/roadmap.md) |

### 功能 × 平台對照(v0.2)

| 能力 | Linux X11 | Linux Wayland | Windows | macOS |
|---|---|---|---|---|
| 全 22 條全域熱鍵 | ✅ XGrabKey | ✅ xdg-desktop-portal ≥1.19 | ✅ Win32 `RegisterHotKey` | ❌ 沒寫 |
| 麥克風錄音 | ✅ ALSA(cpal) | ✅ PipeWire(cpal) | ✅ WASAPI(cpal) | ⚠️ CoreAudio 沒測 |
| 雲端 STT(Groq / OpenAI Whisper) | ✅ | ✅ | ✅ | ✅ Tauri+reqwest 跨平台 |
| 本機 STT(whisper.cpp `whisper-server` 子程序) | ✅ | ✅ | ✅ shell-out + HTTP 架構 work | ⚠️ 沒寫(架構通,binary 沒驗) |
| `SendInput` Ctrl+V paste-back | ✅ xdotool / ydotool | ✅ ydotool | ✅ Win32 `SendInput` | ❌ 沒寫 |
| 滑鼠反白即讀(不必 Ctrl+C) | ✅ xclip PRIMARY | ✅ 同上 | ❌ **OS 沒這 primitive**(必先 Ctrl+C) | 部份 NSPasteboard |
| 視窗 context(process / title) | ✅ xdotool + `/proc` | ✅ 同上 | ✅ Win32 `GetForegroundWindow` 等 | ❌ 沒寫 |
| Mori 主視窗 + tabs(Chat / Config / Profile / Memory / Skills / Deps) | ✅ | ✅ | ✅ | ✅ |
| Floating Mori 精靈(透明 + 動畫) | ✅ XShape 1-bit clip | ✅ CSS border-radius | ✅ Tauri transparent window | ⚠️ 沒測 |
| Tray icon + 右鍵 menu | ✅ AppIndicator | ✅ AppIndicator | ✅ | ⚠️ 沒測 |
| Character pack(sprite 動畫) | ✅ | ✅ | ✅ 4×4 placeholder 寫到 `%USERPROFILE%\.mori\characters\` | ✅ |
| Built-in skills(memory / translate / polish / summarize / compose / fetch_url) | ✅ | ✅ | ✅ 全綠 self-test 過 | ✅ Tauri+HTTP 跨平台 |
| Action skills `open_url` / `open_app` | ✅ xdg-open / `.desktop` | ✅ 同上 | ✅ Win32 `ShellExecuteExW`(silent error,不彈窗) | ❌ 沒寫 |
| Action skill `send_keys` | ✅ ydotool 鍵碼 | ✅ 同上 | ✅ `SendInput` VK 注入 | ❌ 沒寫 |
| URL-template skills(google_search / ask_chatgpt / ask_gemini / find_youtube) | ✅ | ✅ | ✅ 走 open_url | ❌ 沒寫 |
| ollama 本機 LLM | ✅ | ✅ | ✅ 官方 Windows installer | ✅ |
| claude-bash / gemini-bash / codex-bash CLI proxy | ✅ | ✅ | ✅ chain 端對端 work | ⚠️ 沒測 |
| Memory persistence(`~/.mori/memory/*.md`) | ✅ | ✅ | ✅ 走 USERPROFILE | ✅ |

### Windows 已知細微差別

1. **「滑鼠反白即讀」** — Windows OS 沒有 X11 PRIMARY selection 概念。User 要用「反白 → 直接講話讓 Mori 處理」流程的話,必須**先 Ctrl+C** 把選取內容放進剪貼簿。Linux X11 可以直接拖反白讀到。
2. **`open_app` 解析範圍** — Windows 走 `ShellExecuteExW` 自動查 App Paths 註冊表(chrome / code / firefox / msedge / notepad / winword 等預設 app)+ PATH。Start Menu 釘選的 `.lnk` shortcut 跟 Microsoft Store apps(AUMID)目前不一定能解 — roadmap 中。
3. **本機 whisper-server 一鍵下載** — Linux 在 Deps 頁可一鍵裝;Windows 目前要從 [whisper.cpp releases](https://github.com/ggml-org/whisper.cpp/releases) 手動下載 `whisper-bin-x64.zip` → 解壓 → 整套(.exe + .dll)放到 `%USERPROFILE%\.mori\bin\`。Deps 頁有 Manual 指令引導。

### 架構備註

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
