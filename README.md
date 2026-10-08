# Mori (Desktop)

森林精靈 **Mori** 的桌面身體 — 從 [world-tree](https://github.com/yazelin/world-tree) 走到你的桌面。
Tauri 2 + Rust + React，Whisper 是耳朵，LLM 是腦袋，你是同伴。

> 「Iron Man 有 Jarvis,我有 Mori。」

![Mori OG](docs/og-image.png)

**完整介紹 + 互動 demo**：[**yazelin.github.io/mori-desktop**](https://yazelin.github.io/mori-desktop/)

**Latest** — [**v0.7.6**](https://github.com/yazelin/mori-desktop/releases/tag/v0.7.6)：**Deps 修復 + CLI 偵測補強**（Linux whisper-server 改由 Deps 自行建置，已安裝項目可重新安裝 / 修復，Codex/Gemini/Claude CLI 補掃 nvm / Volta / 使用者 bin）· v0.7.5 → Annuli process controls + release toolchain cleanup · v0.7.4 → Windows Annuli runtime path hotfix · v0.7.3 → Annuli token setup + FLAC recordings 修補 · 完整 changelog 看 [`CHANGELOG.md`](CHANGELOG.md)

---

## Demo

按住 `Ctrl+Alt+Space` 講話，放開 Mori 接著做事（X11 session，29 秒）：

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

第一次跑會做四件事：

1. **權限對話框**(Linux Wayland)— 點「**新增**」。X11 / Windows / macOS 直接 grab 不會跳。
2. **建立 `~/.mori/`**:config stub / themes / 6 voice + 6 agent starter / corrections.md
   baseline / logs / installed-apps cache 等(完整結構見
   [`docs/mori-home`](https://yazelin.github.io/mori-desktop/mori-home.html))
3. **宿靈儀式(Quickstart)** — 跳 onboarding modal(v0.4.2+),5 幕詩意流或 Direct setup
   表格擇一，問使用者名 / Groq API key / LLM provider key / starter 語系。設了
   `$GROQ_API_KEY` / `$GEMINI_API_KEY` / `$OPENAI_API_KEY` env var 會自動偵測 + banner
   提示「key 欄位可留空」,verify 真打 API 用 env value 確認
4. **啟動主視窗** + 桌面右下 floating sprite(160×160,OS prefers-color-scheme 決定 dark/light)

詳細欄位範本見 repo 根 [`config.example.json`](config.example.json);完整步驟見
[**docs/getting-started**](https://yazelin.github.io/mori-desktop/getting-started.html);
儀式 vs Direct setup 詳見 [**docs/dwelling-rite**](https://yazelin.github.io/mori-desktop/dwelling-rite.html)。

---

## 上手 30 秒

日常用法四個鍵打天下：

| 鍵 | 用途 |
|---|---|
| `Ctrl+Alt+Space` | 開始 / 結束錄音(可切 `toggle` / `hold` 模式) |
| `Ctrl+Alt+Esc` | 中斷錄音 / 思考(SIGKILL 子程序) |
| `Ctrl+Alt+P` | Profile picker overlay(方向鍵選) |
| `Alt+0~9` | 切 VoiceInput profile |
| `Ctrl+Alt+0~9` | 切 Agent profile |

流程：

1. **選 mode**(每按一次就鎖在那個 mode 直到再切)— `Alt+N` 純聽寫貼游標,`Ctrl+Alt+N` 走 Agent loop
2. **錄音** — 按 `Ctrl+Alt+Space`(預設 toggle 一按切換,Config 可切成 hold 按住錄)
3. **中斷** — `Ctrl+Alt+Esc` 隨時丟掉錄音 / abort LLM call
4. **忘了 slot 編號** — `Ctrl+Alt+P` 開 picker

預設安裝就送 6 個 voice + 6 個 agent starter(`USER-00.純文字輸入` ~ `USER-05.提示詞優化` /
`AGENT.md` + `AGENT-01.翻譯助手` ~ `AGENT-05.聽我指令`),slot 0~5 都有，熱鍵切換馬上可用。
v0.4.1+ 也 bundle EN 對照版,**Profiles tab「加入範本」按鈕**可按一下換語系 / 還原
(改壞了也救得回來)。自訂 slot 6~9 用同檔名格式 `AGENT-NN.<display>.md` /
`USER-NN.<display>.md` 丟到 `~/.mori/agent/` / `~/.mori/voice_input/` 即可
(範本見 [`examples/`](examples/) 或 [Profile 範本頁](https://yazelin.github.io/mori-desktop/profile-examples.html))。

完整熱鍵清單 + 自訂方式 → [docs/hotkeys](https://yazelin.github.io/mori-desktop/hotkeys.html)。

### Mori Ear 語音辨識模式

把 `stt_provider` 設為 `whisper-local` 時，Mori Desktop 會把錄音交給
[Mori Ear](https://github.com/yazelin/mori-ear) 的本機轉譯服務。`whisper-local` 是為了
相容既有設定而保留的名稱；實際使用哪一種辨識後端，由 Mori Ear 的即時模式決定：

| 模式 | 行為 |
|---|---|
| `local` | 只用本機 `whisper-server`，音檔不送上雲端 |
| `groq` | 只用 Groq Whisper API，需要網路與 Groq API key |
| `auto` | 先用本機；本機不可用或辨識失敗時，再改用 Groq |

Linux 可執行 `ear settings` 或 `mori-ear --settings`，用視窗直接切換「線上 Groq」、
「本機」或「自動」。新送出的錄音會立即採用新模式，不必重新啟動 Desktop 或 Ear。
也可以在 Mori Ear 錄音時說「現在是什麼模式？」或「Mori，切換到線上模式」。成功後
會播放簡短語音並顯示桌面通知；語音指令、提示音與通知可分別用
`~/.mori/ear.json` 的 `mode_commands.enabled`、`sound`、`notification` 開關控制。

Desktop 呼叫 Ear 時會指定 `cleanup=false`，只取得原始辨識結果，再由目前的 VoiceInput
profile 做一次文字整理，避免重複潤飾。轉檔頁則固定使用 `local`，不會因 `auto` 模式
把既有音檔送往雲端。

---

## 能做什麼

**Voice / Agent**
- 雙模式(VoiceInput 純聽寫 / Agent 帶 loop)+ 10 個 profile slot 切換(0~9,v0.4.1+ 預載 6 個 voice + 6 個 agent starter)
- 外部工具 bridge — `agent_mode: dispatch` 把語音優化過的 prompt 推給其他桌面 app
  (範本見 [examples/agent/AGENT-03.ZeroType Agent.md](examples/agent/AGENT-03.ZeroType%20Agent.md))
- 自訂 `shell_skills` — 把 `gh` / `docker` / `kubectl` / 自家 script 變 Mori 能力，不用改 Rust

**LLM Providers**
- 雲端 — Groq / Gemini
- 本機 — Mori Ear `local` STT + `ollama` LLM（可完全離線執行）
- Bash CLI proxy — `claude` / `gemini` / `codex`(用 user 自己的 Pro/Max quota,
  v0.4.0+ Windows 短名 binary 自動探 `.cmd` shim)
- OpenAI-compat 自訂端點 — Azure OpenAI / OpenRouter / 自家代理寫進 `providers.<name>`
  就能用，見 [docs/providers](https://yazelin.github.io/mori-desktop/providers.html)

**個人化**
- 長期記憶(`~/.mori/memory/*.md`,user 可編)+ 自動 inject 進 context
- 剪貼簿 / 反白 / URL 自動進 context(v0.4.0+ 進 LLM 前自動 redact API key 樣式)
- **STT 校正字典**(v0.5.1+)— `~/.mori/corrections.md` bundle 200+ 條常見諧音 / 技術詞校正,profile 可 `#file:` 引用
- 雙 theme(dark / light)+ VSCode-like 自訂(`~/.mori/themes/*.json`)+ v0.4.1+ **OS prefers-color-scheme 自動偵測**
- 替換 floating Mori 角色 — 4×4 sprite sheet animation + character pack 系統
  (規範見 [docs/character-pack](docs/character-pack.md),`.moripack.zip` import 規劃中)
- 完整視覺品牌系統(公式書 = 單一可信來源)

**可靠性 / 觀測 / 隱私**
- 所有 LLM provider 都有 timeout 兜底
- Agent loop 殘留 child 不會卡死 — 按 `Ctrl+Alt+Esc` 即可 SIGKILL
- **Phase A 觀測層**(v0.4.0+)— `~/.mori/logs/mori-YYYY-MM-DD.jsonl` 每次 LLM call /
  spawn error / redaction 全自動入帳，**LogsTab** UI 可 filter 看，除錯不用盯 terminal
- **隱私 redact**(v0.4.0+)— clipboard / selection 進 LLM API 之前掃 `gsk_*` / `sk-*` /
  `AIzaSy*` / `Bearer *` 等 token 樣式遮蔽,**token 永遠不離開本機**
- **Context anti-injection**(v0.5.1+)— context section 加 hard rule,LLM 不再把剪貼簿
  / 視窗標題裡夾的「忽略上述」「執行 X」之類 payload 當 user 指令執行
- **Installed apps catalog**(v0.5.0+)— 跨平台 scan 使用者實際裝的 app,top 50 注入
  `open_app` skill description,LLM 不亂猜「user 講 SQL 是 SQL Server 還是 SQLite」
- **Hey Mori 喚醒**(v0.6.0+)— Tray menu 開「Hey Mori 待命」,對麥克風喊就觸發
  錄音 + STT + agent,**不用按熱鍵**。Wake 觸發後播一段 Mori 的應答音(5 個內建
  voice 可選 / 也能上傳自錄),你不用盯畫面就知道 Mori 在聽。VAD silence-stop
  自動偵測你講完(連續 1.5s 靜音就送出),不用固定錄滿 N 秒。Bundled
  `hey-mori.onnx` 預設 model,fresh install 開箱即用;Linux user 可進階自訓
  個人聲線 verifier 提高命中率

未來規劃(非同步任務隊列 / AgentPulse 通知 / TTS / 自訂 wake-word phrase UI / Annuli
長期人格演化)詳見 [**roadmap**](docs/roadmap.md)。

---

## 平台支援

### 概況

| 平台 | 狀態 |
|---|---|
| **Ubuntu 26.04 + GNOME Wayland** | 主力開發 + 測試，全功能 |
| **Linux X11**(任何發行版) | 全功能 |
| **Linux Wayland**(GNOME / KDE / ...) | `xdg-desktop-portal` ≥ 1.19 可用 portal 熱鍵；Ubuntu 24.04 LTS 的 1.18 可改用下方 GNOME 自訂快捷鍵 |
| **Windows 10 / 11** | **v0.4.0 first-class**(2026-05)— 視窗 context capture / paste-back / open_url / open_app / 短名 binary 自動探 `.cmd` shim 全套到位 |
| **macOS** | **核心 voice 跑得起來**(主視窗 + cpal 錄音 + STT + LLM 都 cross-platform)。**OS 整合層尚未接** — paste-back / 反白選取 / send_keys / 視窗 context capture 各個 `selection_macos.rs` / `capture_window_context()` mac 變體都還沒寫。Contributor 路徑(寫一份對應 native call 即可),見 [roadmap](docs/roadmap.md) |

### 功能 × 平台對照(v0.7.x)

| 能力 | Linux X11 | Linux Wayland | Windows | macOS |
|---|---|---|---|---|
| 全 22 條全域熱鍵 | 支援：XGrabKey | 支援：xdg-desktop-portal ≥1.19；GNOME 1.18 可用 `--toggle-recording` 備援 | 支援：Win32 `RegisterHotKey` | 未支援 |
| 麥克風錄音 | 支援：ALSA(cpal) | 支援：PipeWire(cpal) | 支援：WASAPI(cpal) | 尚未驗證 CoreAudio |
| 雲端 STT(Groq / OpenAI Whisper) | 支援 | 支援 | 支援 | 支援：Tauri+reqwest 跨平台 |
| 本機 STT（Mori Ear + whisper.cpp `whisper-server`） | 支援 | 支援 | 支援：HTTP 架構可用 | 架構可用，尚未驗證 binary |
| `SendInput` Ctrl+V paste-back | 支援：xdotool / ydotool | 支援：ydotool 0.1.x / 1.x | 支援：Win32 `SendInput` | 未支援 |
| 滑鼠反白即讀(不必 Ctrl+C) | 支援：xclip PRIMARY | 支援：同左 | 未支援，必先 Ctrl+C | 部份支援：NSPasteboard |
| 視窗 context(process / title) | 支援：xdotool + `/proc` | 支援：同左 | 支援：Win32 `GetForegroundWindow` 等 | 未支援 |
| Mori 主視窗 + tabs(Chat / Profiles / Config / Memory / Annuli / Skills / Deps / Logs) | 支援 | 支援 | 支援 | 支援 |
| Floating Mori 精靈(透明 + 動畫) | 支援：XShape 1-bit clip | 支援：CSS border-radius | 支援：Tauri transparent window | 尚未驗證 |
| Tray icon + 右鍵 menu | 支援：AppIndicator | 支援：AppIndicator | 支援 | 尚未驗證 |
| Character pack(sprite 動畫) | 支援 | 支援 | 支援：4×4 placeholder 寫到 `%USERPROFILE%\.mori\characters\` | 支援 |
| Built-in skills(memory / translate / polish / summarize / compose / fetch_url) | 支援 | 支援 | 支援：全綠 self-test 過 | 支援：Tauri+HTTP 跨平台 |
| Action skills `open_url` / `open_app` | 支援：xdg-open / `.desktop` | 支援：同左 | 支援：Win32 `ShellExecuteExW`(silent error,不彈窗) | 未支援 |
| Action skill `send_keys` | 支援：ydotool 鍵碼 | 支援：同左 | 支援：`SendInput` VK 注入 | 未支援 |
| URL-template skills(google_search / ask_chatgpt / ask_gemini / find_youtube) | 支援 | 支援 | 支援：走 open_url | 未支援 |
| ollama 本機 LLM | 支援 | 支援 | 支援：官方 Windows installer | 支援 |
| claude-bash / gemini-bash / codex-bash CLI proxy | 支援 | 支援 | 支援：chain 端對端 work | 尚未驗證 |
| Memory persistence(`~/.mori/memory/*.md`) | 支援 | 支援 | 支援：走 USERPROFILE | 支援 |

### Windows 已知細微差別

1. **「滑鼠反白即讀」** — Windows OS 沒有 X11 PRIMARY selection 概念。User 要用「反白 → 直接講話讓 Mori 處理」流程的話，必須**先 Ctrl+C** 把選取內容放進剪貼簿。Linux X11 可以直接拖反白讀到。
2. **`open_app` 解析範圍** — Windows 走 `ShellExecuteExW` 自動查 App Paths 註冊表 + PATH。v0.5.0+ 加 **installed apps catalog**:Mori scan 你的 Start Menu / Desktop `.lnk`,top 50 常用 app 注入 LLM tool description,LLM 用列表 match 而不是猜。Microsoft Store apps(AUMID-only)目前仍不一定能解 — roadmap 中。
3. **本機 whisper-server 安裝 / 修復** — v0.7.6 起 Linux 在 Deps 頁會從 whisper.cpp source 自行建置 CPU 版 `whisper-server`,並把相依 `.so` 放進 `~/.mori/bin/`;Windows 仍使用官方 release zip 解壓到 `%USERPROFILE%\.mori\bin\`。已安裝項目可在 Deps 頁按「重新安裝 / 修復」覆蓋壞掉的 binary。

### 架構備註

`mori-core` 是純 Rust lib 跟平台無關;`mori-tauri` 的平台分流走
`cfg_attr(target_os = ..., path = ...)`,加新平台等於加一份對應的
`selection_<platform>.rs` + Cargo.toml 的 target-specific deps。
細節見 [Troubleshooting](https://yazelin.github.io/mori-desktop/troubleshooting.html)
跟 [Roadmap](docs/roadmap.md)。

### Mori Ear 與本機 STT

Mori Desktop 不再自行啟動私有的 Whisper 子程序。選用 `whisper-local` 時，Desktop 會：

1. 讀取 `~/.mori/mori-ear-server.json`，確認 Mori Ear 的本機 HTTP 服務可用。
2. Ear 尚未執行時，嘗試啟動 `mori-ear --serve`，並等待服務就緒。
3. 將 WAV 錄音送至 Ear 的 `/inference`；未指定後端，因此會跟隨 Ear 當下的模式。
4. 收到原始逐字稿後，再套用 Desktop 的 VoiceInput profile 與 LLM 文字整理。

`local` 模式使用 Mori 共用的 whisper.cpp `whisper-server`。Mori Ear 會讀取
`~/.mori/whisper-server.json`，必要時透過 `~/.mori/bin/mori-whisper-serve --ensure`
要求管理程序啟動服務；`auto` 模式在這條本機路徑失敗後才改用 Groq。

### Ubuntu 24.04 的全域快捷鍵備援

Ubuntu 24.04 LTS 內建的 `xdg-desktop-portal` 1.18 不支援 Mori 使用的全域快捷鍵介面。
若不升級 portal，可在 GNOME「設定 → 鍵盤 → 自訂快捷鍵」新增：

- 名稱：`Mori Desktop 錄音`
- 指令：`env WEBKIT_DISABLE_DMABUF_RENDERER=1 /完整路徑/mori-tauri --toggle-recording`
- 快捷鍵：`Ctrl+Alt+Space`

Alt+0~9(語音輸入 profile)與 Ctrl+Alt+0~9(Agent profile)同樣靠 portal 註冊，沒 portal 時改綁
`mori-tauri --profile-slot N` / `--agent-slot N`。一次綁 20 組：

```bash
S=org.gnome.settings-daemon.plugins.media-keys; B=/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings
BIN=/完整路徑/mori-tauri; paths=()
for n in $(seq 0 9); do for k in profile agent; do
  p="$B/mori-$k-$n/"; paths+=("'$p'")
  [ $k = profile ] && key="<Alt>$n" || key="<Ctrl><Alt>$n"
  gsettings set $S.custom-keybinding:$p name "Mori $k slot $n"
  gsettings set $S.custom-keybinding:$p command "env WEBKIT_DISABLE_DMABUF_RENDERER=1 $BIN --$k-slot $n"
  gsettings set $S.custom-keybinding:$p binding "$key"
done; done
# 再把 ${paths[@]} 併進 `gsettings get $S custom-keybindings` 既有清單後 set 回去(別覆蓋掉其他自訂快捷鍵)
```

若 Mori 已在執行，第二個程序會通知既有程序切換錄音；若 Mori 尚未執行，同一個指令會
先啟動 Mori 再開始錄音。Wayland 貼回文字會自動偵測 `ydotool` 0.1.x 或 1.x 的參數格式，
避免舊版把按鍵碼 `29 47 47 29` 當成文字貼出。

---

## 文件

| | |
|---|---|
| [**Landing**](https://yazelin.github.io/mori-desktop/) | 推廣首頁 + interactive demo |
| [Getting Started](https://yazelin.github.io/mori-desktop/getting-started.html) | install / dev / 第一次跑(Linux / Windows / macOS) |
| [Dwelling Rite](https://yazelin.github.io/mori-desktop/dwelling-rite.html) | Quickstart 5 幕 + Direct setup,中英 starter 選 / env var 偵測 |
| [Hotkeys](https://yazelin.github.io/mori-desktop/hotkeys.html) | 完整熱鍵清單 + 自訂 |
| [Providers](https://yazelin.github.io/mori-desktop/providers.html) | Groq / Gemini / Ollama / Claude / Gemini / Codex bash+cli / OpenAI-compat 端點 |
| [~/.mori/](https://yazelin.github.io/mori-desktop/mori-home.html) | config / profile / memory / theme / logs / corrections 全套結構 |
| [Annuli](https://yazelin.github.io/mori-desktop/annuli.html) | Annuli runtime / SOUL token / Windows 手動安裝 / 記憶寫入故障排除 |
| [Troubleshooting](https://yazelin.github.io/mori-desktop/troubleshooting.html) | LogsTab 除錯 / Windows bash CLI / 全域熱鍵 / Whisper deps |
| [Tokenizer 對比](docs/tokenizer-comparison.md) | 中英 starter 在不同 LLM 的 token 數差異 + 取捨 |

進階參考:[Profile 範本](https://yazelin.github.io/mori-desktop/profile-examples.html) ·
[Design Book](https://yazelin.github.io/mori-desktop/design-book.html) ·
[Architecture](docs/architecture.md) · [Roadmap](docs/roadmap.md) · [CHANGELOG](CHANGELOG.md)

---

## Mori 宇宙

只想用桌面 AI 工具 → 留在這 repo 就行。想看更大的世界觀：

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
- **Windows whisper-server 按一下下載** — 目前 Deps 頁只在 Linux 自動下載引擎,Windows 要手動。需要把 `InstallSpec::Shell` 補一個 `InstallSpec::Download` variant 走 Rust reqwest + zip extract
- **Custom wake-word UI**(v0.6.0 起 CLI `mori-wake-train.py` 可訓任意 phrase / Linux only)— 想叫「Hey Hermes」/「Hey 小綠」需要 UI 化的訓練流程 + Windows piper-phonemize wheel 相容
- **TTS speak-back**(Mori 真的講話，不只 wake-ack)— Gemini TTS quota 受限 + Edge TTS 免費 fallback + 開關 + cache 策略
- **其他 LLM provider integration**(Claude API native / DeepSeek / Qwen 等)

更詳細的進入點 → [roadmap](docs/roadmap.md)。

---

## License

MIT
