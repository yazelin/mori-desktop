# Roadmap

歷史 phase 紀錄 → [CHANGELOG](../CHANGELOG.md)。本頁只留**未來規劃**。

平台支援現況見 [README → 平台支援](../README.md#平台支援)。**v0.2 之後 Linux X11 +
Linux Wayland + Windows 10/11 都是全功能**;macOS 殼還沒接。

---

## 近期 — macOS 平台殼

Windows v0.2 已上線(`selection_windows.rs` + Win32 SendInput / GetForegroundWindow
+ Win RegisterHotKey),設計上 mori-core 跟其他平台完全共用,加 macOS 同樣 pattern
寫一份就能用。三塊要補:

| 子項 | Linux 現況 | Windows 現況 | Mac 要寫的 |
|---|---|---|---|
| **selection capture**(滑鼠反白 → context) | `xclip -selection primary` | 回 None(OS 沒這概念) | `NSPasteboard` + accessibility(可選) |
| **window context**(focus app / title 進 LLM context) | `xdotool getactivewindow` + `/proc/<pid>/comm` | `GetForegroundWindow` + `QueryFullProcessImageNameW` | `NSWorkspace.frontmostApplication` + accessibility(視窗標題要 accessibility 權限) |
| **paste-back**(模擬 Ctrl+V 貼回游標) | xclip + xdotool/ydotool | `SetClipboardData` + `SendInput` Ctrl+V | `NSPasteboard.writeObjects` + `CGEventCreateKeyboardEvent` Cmd+V + accessibility permission UX |

**架構**:寫一份 `selection_macos.rs`,在 `mori-tauri/src/main.rs` 加一行
`#[cfg_attr(target_os = "macos", path = "selection_macos.rs")]`,定義
`PlatformPasteController` type alias 指 Mac 版,call sites 0 改。`Cargo.toml`
target-specific deps 區塊加 `objc2` / `core-graphics` 之類。

**為什麼是 contributor 路徑**:yazelin 主力 Ubuntu + Windows,沒有 Mac 日常
驗證環境。寫了測不到 = 不知對錯。歡迎 fork + PR — 參考 `selection_windows.rs`
寫成同樣公開 API(`read_primary_selection` / `PlatformPasteController` /
`send_enter` / `warn_if_setup_missing`)就直接接得上。

---

## 近期 — Windows whisper-server 一鍵下載

`crates/mori-tauri/src/deps.rs` 的 `InstallSpec::Shell` 只在 Linux work(用
`sh -c`),Windows user 目前要手動從 whisper.cpp release 下載
`whisper-server.exe`。要補一個 `InstallSpec::Download { url_template, dest_template,
extract_member }` variant,走 Rust reqwest + zip extract,跨平台一致。

Linux 端的 whisper-server 一鍵下載已經 work(v0.2),做法可參考 `deps.rs`
registry 的 whisper-server entry。

---

## 中期 — 非同步任務系統

> 「同步回應 → 任務隊列」現階段 Mori 都是 user 講話 → block 等 Mori 回應。
> 對長時間任務(會議逐字稿、媒體下載、深度研究)需要背景跑 + 完成通知。

### Task Queue
- 「立即執行」vs「排入隊列」開關:user 講完話可選擇同步等 / 丟進隊列
- 隊列長度上限可設(避免 LLM token 暴衝)
- 隊列狀態顯示在主視窗 sidebar(新增 Tasks tab)+ floating sprite chip
- 個別任務可取消 / 重排優先級

### AgentPulse 整合
- Task 完成後透過
  [`AgentPulse`](https://github.com/yazelin/agent-pulse)
  推播通知(desktop notification / Slack DM / email 等)
- User 可設「哪類任務完成才通知」(短任務不打擾、長任務一定通知)
- Mori 主動報結果:「你 30 分鐘前交代的會議摘要好了 →」

### 同步 / 非同步開關
- Config:`async_tasks.enabled` / `max_queue_length` / `notify_threshold_seconds`
- Per-profile 可覆蓋(對話 profile 預設同步、研究 profile 預設非同步)

---

## 長期 — 進階能力

### 背景排程
「每小時提醒喝水」「每天 9 點晨報」— 真正的常駐 agent,跟非同步任務系統結合。

### 媒體 / 系統整合
- **媒體下載** — 「下載這個影片」呼叫 yt-dlp
- **ExecCommand 白名單** — 「跑那個指令」要先有白名單 + 二次確認機制
- **會議逐字稿** — 連續錄音存檔 → 結束後 LLM 整理會議記錄 + action items

### TTS
Mori 還不能開口說話,只有文字。預計接 OpenAI TTS / ElevenLabs / 本機 Piper。

### Wake Word
不用按熱鍵,叫名字喚醒(「Mori」/「森」等)。需離線 wake-word detection
(`openwakeword` / `Porcupine`)+ 隱私邊界設計。

---

## 更遠 — Annuli 整合

`mori-desktop` 是 Mori 的**短期工作記憶 + 動作執行 body**。長期記憶 / 人格演化
跨 session 沉澱在 [`Annuli`](https://github.com/yazelin/Annuli)(private)。

未來 Mori 透過 MCP 跟 Annuli 對接:
- 重要對話沉澱進 Annuli 的長期 memory pool
- 跨 device sync(在 phone / 另一台電腦上的 Mori 共用同一份人格)
- 人格演化(season-based reflection,Mori 自己讀 Annuli 寫 patches)

---

## 設計原則(不變)

- `mori-core` 永不依賴 UI / 平台 — 換載體只多寫一個薄殼 crate
- 公式書(`docs/*.html` + `_book.css`)是視覺單一可信來源
- Theme(`~/.mori/themes/*.json`)/ profile(`~/.mori/voice_input/` `agent/`)/
  memory(`~/.mori/memory/`)都是 plain text user 可編輯
- LLM 沒拿到 shell 直接 access — `shell_skills` 走 `command: [array]`,
  `{{name}}` 替換是字面字串

---

已 ship 的 phase(1A 起一路到目前)→ [CHANGELOG](../CHANGELOG.md)。
