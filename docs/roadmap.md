# Roadmap

> **精靈不會離開森林,牠只是搬到你的腦裡。**
> **靜靜記得,牠的森林,有你經過的痕跡。**

歷史 phase 紀錄 → [CHANGELOG](../CHANGELOG.md)。本頁只留**未來規劃**。

Mori 是三層宇宙(`mori-desktop` / `Annuli` / `world-tree`),各層分工見
[architecture.md](architecture.md)。本 roadmap 主要排 `mori-desktop` 端的能力,
但會明確標出哪些工作是「**接上 Annuli 已有的服務**」而不是 mori-desktop 自己重做。

平台支援現況見 [README → 平台支援](../README.md#平台支援)。v0.2 之後 Linux X11 +
Linux Wayland + Windows 10/11 都是全功能;macOS 殼還沒接。

---

## 近期

### macOS 平台殼

Windows v0.2 已上線,設計上 mori-core 跟其他平台完全共用,加 macOS 同樣 pattern
寫一份就能用。三塊要補:

| 子項 | Linux 現況 | Windows 現況 | Mac 要寫的 |
|---|---|---|---|
| **selection capture**(滑鼠反白 → context) | `xclip -selection primary` | 回 None(OS 沒這概念) | `NSPasteboard` + accessibility(可選) |
| **window context**(focus app / title 進 LLM context) | `xdotool getactivewindow` + `/proc/<pid>/comm` | `GetForegroundWindow` + `QueryFullProcessImageNameW` | `NSWorkspace.frontmostApplication` + accessibility(視窗標題要 accessibility 權限) |
| **paste-back**(模擬 Ctrl+V 貼回游標) | xclip + xdotool/ydotool | `SetClipboardData` + `SendInput` Ctrl+V | `NSPasteboard.writeObjects` + `CGEventCreateKeyboardEvent` Cmd+V + accessibility permission UX |

**為什麼是 contributor 路徑**:yazelin 主力 Ubuntu + Windows,沒有 Mac 日常
驗證環境。歡迎 fork + PR — 參考 `selection_windows.rs` 寫成同樣公開 API
就直接接得上。

---

## 中期 — 三條主線

### 記憶之森 — 本機記憶架構升級

短期 session 記憶留在 `~/.mori/memory/`(現有 plain markdown),但要把它變得
**真的好用**:

- **樹狀結構**:仿 Obsidian vault — `notes/`、`projects/`、`people/`、
  `daily/` 等資料夾,user 打開檔案總管就看得到結構
- **SQLite index 層**(`~/.mori/memory/.index.db`)— 不存內容,只索 metadata
  + full-text search + 主題關鍵字。內容仍 plain markdown,user 可直接編輯
- **確定性 chunking**:長 memory 自動切 ≤3K token chunks,鎖在內容語意邊界
  (段落 / 子標題),**不靠 vector embedding 那條 fuzzy path**
- **每日 digest**:Mori 每天結束自己生 `~/.mori/memory/daily/{YYYY-MM-DD}.md`,
  總結今天記了什麼。對應 Annuli 的 `/sleep` ring,但 daily 留本機,sleep 才推 Annuli
- **三層 scope**:per-source(這個 user 在某 app 跟 Mori 聊的事)/ per-topic
  (某主題的所有紀錄)/ global(今天 / 這週 digest)— 對應森林意象的「**樹 /
  樹根 / 林冠**」

設計上仍然「**user 打開記事本就能看 / 改**」— 跟現有 `~/.mori/agent/`
`~/.mori/voice_input/` plain-text 哲學一致。

### 林間心跳 — 接 Annuli scheduler

Annuli 那邊**已經有完整的背景循環系統**(APScheduler 跑 4 條任務:
explore / learn / study / post)。mori-desktop 不重做,改成**前端 + Annuli 控制台**:

- mori-desktop 主視窗加「**Annuli**」tab — 顯示 Annuli 跑的 task 狀態、
  最近 ring、knowledge 庫
- 提供 GUI 切換 task on/off(POST `http://localhost:5000/schedule/<id>/toggle`)
- 提供「**立即觸發**」按鈕(POST `/schedule/<id>/run`)
- Annuli 完成長任務 → 推 `AgentPulse` 通知(desktop notification / Slack DM
  / email),Mori sprite 主動講話「你 30 分鐘前要的 X 好了」
- 桌面快捷:`Ctrl+Alt+Z` 觸發 Annuli `/sleep`(化今天為一輪年輪)

**配置**:
- `~/.mori/config.json` 多 `annuli.endpoint`(預設 `http://localhost:5000`)
- `annuli.enable_sync` boolean(預設 off,user 自己 enable,主權在 user)
- Sleep mode 暫停整個對 Annuli 的對接

### 跨界之手 — 服務整合 framework

愈來愈多服務出 CLI(`gh` / `gemini-cli` / `claude` / `notion-cli` / `yt-dlp` /
`spotify-cli` / `obsidian-cli` / ...),這條 path 比 OAuth REST wrapper 更短:

- **`shell_skills` 仍是 backbone**:user 寫 yaml 把任意 CLI 包成 skill,LLM
  看 schema 就會用。**比 MCP 省 token context 很多**(MCP 把所有 server 的
  tool descriptions 都塞 system prompt;shell_skills 只塞 user 啟用的那幾條)
- **CLI 版本優先**:服務有 CLI 就 wrap CLI,沒 CLI 才走 OAuth REST
- **OAuth 補位**:對於沒 CLI 的服務,做一個 `~/.mori/oauth/` token store
  (OS keyring 走 Tauri plugin)+ 每個 service 一個
  `crates/mori-core/src/integrations/<name>.rs`(Gmail / Calendar / Notion API 等)
- **使用者自選 + 自運行**:每把 token 在自己機器、Mori 不集中代理任何認證、
  不引入任何「中央授權代理」服務

**設計原則**:user 看得到、改得到每把 token,所有 service 整合 spec 公開
寫進 `docs/integrations/<service>.md`。

---

## 長期

### 觀之眼 — 視覺能力

Mori 目前只「聽」(語音)+「讀」(剪貼簿文字),不會「看」。要補:

- Clipboard 圖片偵測 → 進 context
- 截圖(Win+Shift+S / `gnome-screenshot -a` 等)→ vision LLM
- 拖檔到 chat → vision LLM
- 「**截一下這個畫面問 Mori**」快捷鍵
- vision LLM 由 provider routing 切(Groq Llama 3.2 Vision / Gemini Vision /
  Claude Vision)

**不做 OS-level screen polling**(隱私底線)— **user 主動觸發才看**。

### 唇與聲 — TTS + 角色化

Mori 還不能開口說話。要補:

- TTS(OpenAI / ElevenLabs / 本機 Piper),per-profile 設不同聲線
- floating sprite **嘴型同步**:viseme map(音訊 → 嘴型)動畫
- 角色化語氣:每個 agent / voice profile 自己的聲音 + 嘴型 + 個性
- 接 Annuli 的 persona — Mori 講話時 LLM 在 system prompt 看得到當前 persona

### 林之耳 — Wake Word

不用按熱鍵,叫名字喚醒(「Mori」/「森」等)。需離線 wake-word detection
(`openwakeword` / `Porcupine`)+ 隱私邊界設計(預設 off,要 user 自己 enable
+ 視覺指示 sprite 在「聆聽」狀態)。

### 多界橋樑 — IM Bot 整合(Telegram / Discord / LINE / Slack)

讓使用者**透過手機** IM app 找 Mori(翻譯 / 摘要 / 記憶查詢 / 排程提醒等),
Mori 不只是桌面熱鍵 → 變成隨時可達的個人 agent。

**通訊架構**:

| 平台 | 收訊息機制 | 內網 work? |
|---|---|---|
| Telegram | Long polling **或** webhook | ✅ Polling = outbound |
| Discord | Gateway WebSocket(永遠 outbound) | ✅ WebSocket = outbound |
| Slack | Socket Mode(WebSocket)或 webhook | ✅ Socket Mode = outbound |
| LINE | 只接 webhook | ❌ 需要 public HTTPS |
| MS Teams | 多數 webhook | ❌ 同 LINE |

**NAT / webhook 解法**(對 LINE / Teams 而言):**走 Cloudflare Tunnel
(個人 free)**,Mori 內建 setup wizard 自動拉 `cloudflared` daemon →
產 free trycloudflare.com subdomain → user 貼到 LINE Developer Console
即可。**不自架中央 relay server** — 違反 Mori 自有資料 ethos + 無法承擔
infrastructure ops + bot token / 訊息 leakage 責任。

**工程順序**:

- **v0.5 MVP**(~3 週):內建 Telegram + Discord(都是 outbound,內網 work)
  - 新增 `crates/mori-bot/` 或 `mori-tauri/src/bot/`
  - Config tab 加 Bot tab,貼 token → save → 立即 online
  - Bot 訊息走進現有 agent loop(skill / memory 共用)
  - User ID whitelist(防 LLM quota DoS)
  - Bot token 走 OS keyring(Tauri plugin)而非 config.json
- **v0.6**(~4 週):LINE + CF Tunnel 整合 wizard
- **v0.7**:Slack Socket Mode + per-bot agent profile 綁定

**架構面要注意**:bot token phishing target → OS keyring 存;LLM quota
rate-limit + user whitelist;群組裡只在 `@mori` mention 才回應;webhook
signature 驗證(LINE HMAC、Discord Ed25519);bot 訊息轉成 Mori 內部統一
input 格式跟 voice transcript 對接。

### 媒體 / 系統整合

- **媒體下載** — 「下載這個影片」呼叫 yt-dlp(shell_skill 包)
- **ExecCommand 白名單** — 「跑那個指令」要先有白名單 + 二次確認機制
- **會議逐字稿** — 連續錄音存檔 → Whisper streaming → LLM 整理會議記錄 +
  action items,結果丟 Annuli 的 `knowledge/` 永久保存

---

## 更遠

### 歲月之輪 — mori-desktop ↔ Annuli 完整對接

Annuli 已經是 production-ready 的 Flask service(`main.py admin --port 5000`),
有完整的 persona / users / rings / knowledge / drafts / schedule 系統。
mori-desktop 跟它對接走純 HTTP REST,**不需要 MCP**(Annuli 已有 40+ Flask 路由,
直接 call 即可):

| Mori 動作 | 對應 Annuli endpoint |
|---|---|
| Session 開始 — 拉這個 user 的長期記憶 | `GET /users/<platform>_<user_id>` |
| 對話中重要事件 — 餵進長期 | `POST /knowledge/learn`(或新增 `/users/<id>/event` endpoint) |
| user 觸發 `/sleep`(`Ctrl+Alt+Z`)— 化今天為年輪 | 直接 call `engine.do_reflect()` 或新 endpoint `POST /users/<id>/sleep` |
| 看 persona / rings / knowledge | `GET /persona` / `GET /users/<id>` / `GET /knowledge` |
| 觸發 Annuli 排程任務 | `POST /schedule/<id>/run` |

**主視窗加 Annuli tab**:類似現有 Memory tab,但內容是從 HTTP 拉 Annuli 那邊
的 persona / rings / knowledge / schedule。**user 不用切到 Annuli web UI 就能管
所有長期記憶**。

**跨 device 同一份精靈**:同一個 user 在多台機器上跑 mori-desktop,都接同一個
Annuli(可能跑在 user 家用 server / NAS),於是不管 user 在哪台機器上 talk,
Mori 都是同一個 — 同樣的 persona、同樣的 rings、同樣記得過去的事。**「精靈
搬到你的腦裡」 — 牠在哪台機器上都是同一個牠**。

### 世界樹之線 — Mori 連到 world-tree

[`world-tree`](https://github.com/yazelin/world-tree) 目前是 lore 概念 repo,
未來會建成**公開 read-only 服務**(per yazelin 規劃),Mori 從那邊拉:

- **公共 lore / 世界觀**:精靈森林的設定、Mori 的核心身份、共識個性 base
- **預設 character pack 規格**:sprite 動畫規格、預設 personality seed
- **共享預設 memory seeds**:某些對所有 Mori 一致的「**精靈與生俱來知道的事**」
- 連線形式:HTTP GET(public),snapshot 緩存本機,定期 refresh

**個人留個人,世界的歸世界**:
- mori-desktop / Annuli 上的個人記憶 → **不上傳** world-tree
- world-tree 的公共 lore → 拉下來進每個 Mori 的初始狀態
- **大群 Mori 共識**:多台 user 機器上的 Mori 都讀同一份 world-tree,所以
  行為 / 個性 / 預設世界觀一致 — **「同一個精靈,分身在你我家中」**

---

## 設計原則(不變)

- `mori-core` 永不依賴 UI / 平台 — 換載體只多寫一個薄殼 crate
- 公式書(`docs/*.html` + `_book.css`)是視覺單一可信來源
- Theme(`~/.mori/themes/*.json`)/ profile(`~/.mori/voice_input/` `agent/`)/
  memory(`~/.mori/memory/`)都是 plain text user 可編輯
- LLM 沒拿到 shell 直接 access — `shell_skills` 走 `command: [array]`,
  `{{name}}` 替換是字面字串
- **三層分隔線**:個人留個人(本機 + Annuli)、世界的歸世界(world-tree),
  Mori 不會偷把私事推到雲端
- **無中央代理**:user 自有所有 token / 資料 / memory,Mori-desktop 不集中
  代理任何認證、不引入「需 Mori 服務商」的依賴

---

已 ship 的 phase(1A 起一路到目前)→ [CHANGELOG](../CHANGELOG.md)。
