# Roadmap

歷史 phase 紀錄 → [CHANGELOG](../CHANGELOG.md)。本頁只留**未來規劃**。

---

## 主力平台

**Ubuntu 26.04 + GNOME Wayland**(Mori 開發 + 測試環境)。

- **Windows / macOS** — paste-back / 全域熱鍵尚未接,主視窗 UI 可跑但 voice pipeline 不完整。
  歡迎 fork + PR 幫忙補(`mori-core` 平台殼分離設計,寫個新 crate 就能接新平台)。

---

## 近期(Phase 5 後續)

### 5G-10 — Profile 自動遷移
既有 voice profile 含 action flag 的自動搬到 `agent/`(手動可做,不擋使用)。

### 5E-2 — macOS / Windows voice-input paste-back
目前只 Linux 走 `LinuxPasteController`(arboard + ydotool),其他平台還沒接。
等 contributor 補。

### 3C — Pure Wayland 跨 app 反白
**X11 + XWayland 用 xclip + PRIMARY 已 cover 90%**(`selection.rs:39` 的
`LinuxPasteController`)。Pure Wayland(原生 GTK4 / Hyprland 等沒 XWayland 的
session)需 `xdg-desktop-portal` Selection 介面或 app 主動分享。實用 use case
不大,等 Wayland 生態普及再說。

### 3D — 螢幕擷取進 context
**Active app / window title 已 done** — `HotkeyWindowContext.process_name +
window_title` 透過 xdotool 在熱鍵瞬間捕獲,Mori 看得到當前焦點。
**螢幕擷取**(讓 Mori 看你正在看什麼)還沒做,需設計觸發時機 + 隱私邊界。

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

### Phase 5 — 背景排程
「每小時提醒喝水」「每天 9 點晨報」— 真正的常駐 agent,跟非同步任務系統結合。

### Phase 6 — 媒體 / 系統整合
- **媒體下載** — 「下載這個影片」呼叫 yt-dlp
- **ExecCommand 白名單** — 「跑那個指令」要先有白名單 + 二次確認機制
- **會議逐字稿** — 連續錄音存檔 → 結束後 LLM 整理會議記錄 + action items

### Sprite animation — 4×4 motion sheet
floating widget 目前 6 個 state(idle / sleeping / recording / thinking /
done / error)各一張 256×256 static PNG,動畫只在 CSS(呼吸 / aura ring /
水波 / 葉間光斑)。升級到 **4×4 sprite sheet**(同 state 16 個 motion frame)讓
sprite 自身會動 — talk / walk / happy / notice 等表情切換,16 格比 9 格動畫更精細。
規格已在 [`floating-sprite-spec.md`](floating-sprite-spec.md):
- 每 frame 256×256 → 整張 1024×1024(跟現在 single-frame 同尺寸,引擎只要改 background-size)
- CSS `background-size: 400% 400%` + `steps(16)` animated background-position
- 1.6s 跑完一輪,適合 idle 呼吸 / talk 口型 / walk 步伐這類連續動作
- `.mori-sprite` class 是唯一要動的地方,JS / Rust 不變
- 設計稿:`docs/design/mori-2.png` 已含 sprite sheet 草圖(現是 9 格,升 16 格時擴繪)

### Phase 7 — TTS
Mori 還不能開口說話,只有文字。預計接 OpenAI TTS / ElevenLabs / 本機 Piper。

### Phase 6+ — Wake Word
不用按熱鍵,叫名字喚醒(「Mori」/「森」等)。需離線 wake-word detection
(`openwakeword` / `Porcupine`)+ 隱私邊界設計。

---

## Phase 9+ — Annuli 整合

`mori-desktop` 是 Mori 的**短期工作記憶 + 動作執行 body**。長期記憶 / 人格演化
跨 session 沉澱在
[`Annuli`](https://github.com/yazelin/Annuli)(private)。

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

歷史 phase(1A → brand-3 完成的部分)→ [CHANGELOG](../CHANGELOG.md)。
