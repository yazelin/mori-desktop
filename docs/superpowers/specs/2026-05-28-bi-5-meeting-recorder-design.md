# BI-5 Meeting Recorder — Standalone Design

> **Status**: brainstorming-locked 2026-05-28(yazelin 拍板 7 個決策點 + 8 個設計 section)。
> **Goal**: 建立 standalone `mori-meeting-recorder` repo,實現 [meeting-recorder.md](../../meeting-recorder.md) 的 Observer Mode MVP — 雙軌錄音 + 停止後轉錄 + visibility-based 匯出。
> **Plan output**: 本 spec → `writing-plans` → `docs/superpowers/plans/2026-05-28-bi-5-meeting-recorder.md` → 實作。

---

## 0. 為什麼有這份 spec

`meeting-recorder.md`(232 行)是**契約** — 講「資料邊界」「visibility 分流」「audio 多軌不混音」「Presenter Mode 留白」。
本 spec 是**第一刀 MVP 的設計選擇**(Option A — minimum viable),把契約收斂成可寫的 code:選哪個 audio lib、repo 怎麼長、GUI 哪幾個視覺 state、跟 mori-desktop 的耦合面積。

---

## 1. 範圍紀律

### Build now(Option A — minimum viable)

- 新 repo `mori-meeting-recorder`(public, MIT)
- 雙軌 WAV capture(`meeting_system` + `mic_internal`),Linux + Windows MVP
- 停止後轉錄(shell-out whisper.cpp CLI,parallel via `tokio::join!`)
- `meeting.public.md` + `meeting.internal.md` + `timeline.json` 匯出
- Floating capsule UI(AgentPulse 風,單視窗切 size)+ 雙擊展開 3-tab(Record / Sessions / Deps)+ tray icon
- BI-1 自註冊 manifest(`~/.mori/body-parts/mori.meeting-recorder/manifest.json`,kind=StandaloneApp)
- 自己 bundle `scripts/install-whisper-{linux,windows}.{sh,ps1}` + 自己的 DepsTab

### 刻意 defer(Phase 2+)

- Live captions(chunk / VAD / streaming)
- `mix-preview.wav`
- mori-desktop 內 recent-sessions 列表 / handoff 命令(讓 Mori 整理 public.md)
- Permission Broker(BI-2)整合 — Meeting Recorder 是 user-initiated GUI 動作,不需要 broker;之後 mori-desktop 主動讀 recorder output 才需要走 broker
- Presenter Mode(`mic_public`)
- macOS

### 不做(non-goals)

- 指定 Google Meet 視窗 / 自動偵測 mute 狀態
- 把 mic 自動混進客戶版(永不)
- 雲端 relay / SaaS hub / 中央 OAuth(symbol of self-owned data principle)
- 在 mori-desktop 內**直接**完成完整 Meeting Recorder(必須 standalone-first)

---

## 2. 鎖定的設計決策

| # | 決策 | 採用 | 為什麼 |
|---|---|---|---|
| 1 | MVP 範圍 | Option A 最小可行 | 先把 standalone 雙軌 + 停止後轉錄收斂成可 ship 的東西,live captions / handoff 等迭代 |
| 2 | Repo 名 / 可見性 / license | `mori-meeting-recorder`,public,MIT | 對齊 AgentPulse |
| 3 | GUI stack | Tauri 2 + React | family 一致(mori-desktop / AgentPulse 同套);frontend 與 audio runtime 同 binary |
| 4 | Platform scope | Linux + Windows MVP | 平台 native lib(各家最自然);macOS 等 core 穩定再補(屆時加 CoreAudio impl) |
| 5 | Audio lib | **Linux: `libpulse-binding`(PA client API,PipeWire 完全相容)+ hound**;**Windows: `cpal` 0.15 WASAPI loopback + hound**(對齊 mori-desktop) | **Spike(2026-05-28)發現 cpal 0.15 Linux 只走 ALSA host,看不到 PipeWire `.monitor` source**(monitor 是 pulse 抽象層,ALSA 不認識)。OBS 也是平台 native 路線(`linux-pulseaudio` + `linux-pipewire` + `win-wasapi` 各一個 plugin),不走 gstreamer — 零外部 system deps、低延遲、error 可控、長期 backend 切換容易。本 repo 同模式:`audio/{linux,windows}.rs` 分檔(原本就分),只是 Linux impl 用 libpulse 取代 cpal,上層 `audio::open_capture(SourceKind, PathBuf) → CaptureHandle` interface 不變 |
| 6 | STT 整合 | shell-out `whisper.cpp` CLI,各自 bundle install script | standalone-first(model 在 user filesystem `~/.mori/models/`,兩 repo 共享 via convention 不 via IPC);parallel via `tokio::join!` |
| 7 | BI-1 wiring | self-register `~/.mori/body-parts/mori.meeting-recorder/manifest.json`,kind=StandaloneApp,interfaces=[] | 跟 AgentPulse 同模式但沒 HTTP/SSE(MVP 沒對外 endpoint) |
| 8 | GUI 主視覺 | Floating capsule(AgentPulse 風)+ 雙擊展開,**單視窗切 size** | yazelin 想要快速操作 + 收音狀態一目了然;避開「主視窗 vs 膠囊 sync」雙視窗問題 |
| 9 | css token | mori-meeting-recorder 自己一套,**不**沿 mori-desktop var(--c-*) | bundle deps in repo 原則,各 repo 自治 |
| 10 | mori-desktop 整合範圍 | **不加新 tab**;BI-1 BodyTab 已顯示 capabilities / kind / entrypoint / data_policy,等於「詳細設定 + dep visibility」 | 沒 mori-desktop 也要能跑 → 真的安裝 / dep 偵測 / 設定 在 recorder 自己的 DepsTab,mori-desktop 只是 read-only 可見性 |

---

## 3. Repo / Crate 結構

```
mori-meeting-recorder/               (新 public repo,MIT)
├── README.md
├── LICENSE                          (MIT)
├── CLAUDE.md                        (本 repo agent 規則,沿 AgentPulse 風)
├── AGENTS.md                        (Codex Cloud 用,跟 CLAUDE.md 同步)
├── package.json + tauri.conf.json
├── scripts/
│   ├── install-whisper-linux.sh     (curl whisper.cpp release → ~/.mori/bin/whisper-cli)
│   ├── install-whisper-windows.ps1  (同)
│   └── verify.sh                    (cargo test + npm build + cargo check)
├── src-tauri/
│   ├── Cargo.toml                   (single crate workspace)
│   ├── tauri.conf.json              (decorations:false / transparent:true / alwaysOnTop:true / single-instance plugin)
│   └── src/
│       ├── main.rs                  (Tauri entrypoint + commands + invoke_handler + tray)
│       ├── manifest.rs              (BI-1 manifest 寫入 ~/.mori/body-parts/)
│       ├── recorder.rs              (session lifecycle: start/stop/status)
│       ├── audio/
│       │   ├── mod.rs               (AudioCapture trait)
│       │   ├── linux.rs             (libpulse-binding — list pulse sources, record monitor by name)
│       │   ├── windows.rs           (cpal + WASAPI loopback config)
│       │   └── writer.rs            (hound WavWriter,per-track,16kHz mono 16-bit)
│       ├── session_store.rs         (~/.mori/meetings/<id>/ 目錄佈局 + path getter)
│       ├── transcribe.rs            (shell-out whisper.cpp + parse `--output-json-full`)
│       ├── exporter.rs              (segments → .public.md / .internal.md / timeline.json)
│       └── deps.rs                  (detect whisper-cli + ggml-small.bin)
└── src/
    ├── App.tsx                      (state: collapsed | expanded,switch view + window.setSize)
    ├── CapsuleView.tsx              (小膠囊 — primary UI)
    ├── ExpandedView.tsx             (展開 3-tab container)
    ├── tabs/
    │   ├── RecordTab.tsx            (大 Start/Stop + 來源狀態 + 文案 hint)
    │   ├── SessionsTab.tsx          (~/.mori/meetings/ list,open dir via cue_open_path 同 pattern)
    │   └── DepsTab.tsx              (whisper-cli + model 偵測 + install 指令)
    ├── theme.css                    (自己一套 token)
    └── i18n/locales/{en,zh-TW}.json
```

**單一 src-tauri crate**:Option A 範圍小,沒必要拆 core / tauri 兩層。如果未來 audio core 要被別 repo 用,再拆。

---

## 4. 元件責任(每個 file 一個責任)

| 元件 | 責任 | **不**負責 |
|---|---|---|
| `recorder::Recorder` | session lifecycle:`start_session(config)` → `stop_session()` → `status()`。組合 AudioCapture + SessionStore + Transcribe + Exporter | 平台 audio API、檔案 IO 細節 |
| `audio::AudioCapture` trait | 抽象「給我這個來源的 16kHz mono PCM stream」`open(source_kind, device) → Stream + signal_rx`(signal_rx 給 capsule peak RMS) | session 狀態、檔案寫入 |
| `audio::linux` | Linux:cpal `input_devices().filter(|d| d.name().ends_with(".monitor"))` 找 loopback;default_input 找 mic | Windows / macOS impl |
| `audio::windows` | Windows:cpal 取 output device + WASAPI loopback flag(`SupportedStreamConfig` magic);default_input 找 mic | Linux / macOS impl |
| `audio::writer::WavWriter` | hound 包裝:`new(path, spec)` / `push_samples(&[i16])` / `finalize()`。spec = 16kHz mono 16-bit PCM | source picking |
| `session_store::SessionStore` | `new(session_id, base_dir)` 建 `<base>/<id>/{audio,transcript}/` + 每個 file path 的 getter | audio capture / transcribe |
| `transcribe::run` | spawn `whisper-cli -m <model> -f <wav> --output-json-full`,parse stdout JSON,produce `Vec<Segment>` | UI / export format |
| `transcribe::parse_whisper_json` | **pure function**:`&str → Vec<Segment>`,TDD 用 fixture | spawn process / IO |
| `exporter` | **pure function**:`Vec<Segment> + SessionMeta → (public_md, internal_md, timeline_json)`,visibility filter 在這 | audio / transcribe / IO |
| `deps::detect` | check `~/.mori/bin/whisper-cli` 存在 + executable;check `~/.mori/models/ggml-small.bin` 存在 + 大小合理(≥40MB) | 安裝(交給 scripts/) |
| `manifest::write` | `~/.mori/body-parts/mori.meeting-recorder/manifest.json` overwrite;`entrypoints.app` 用 `std::env::current_exe()` | 對外 HTTP server |

---

## 5. Data flow

```
GUI start 按鈕(capsule 上的 ▶ 或 ExpandedView Record tab Start)
  → invoke("recorder_start", { config })
      → Recorder::start_session()
          ├── session_id = format!("meeting-{}", now.format("%Y%m%d-%H%M%S"))
          ├── SessionStore::new(session_id) → mkdir 結構
          ├── AudioCapture::open(MeetingSystem, find_loopback()) → spawn writer thread → audio/system.wav
          └── AudioCapture::open(MicInternal, default_input)     → spawn writer thread → audio/mic-internal.wav

(... user 在會議中 ...)
CapsuleView 每 500ms 輪詢:
  invoke<RecorderStatus>("recorder_status")
    → { state: Recording, elapsed: 754, system_signal: true, mic_signal: false, session_id: Some("meeting-...") }
  pill 顏色 + 計時器更新

GUI stop 按鈕 → invoke("recorder_stop")
  → Recorder::stop_session()
      ├── 兩個 capture 收尾 + WavWriter::finalize() flush
      ├── tokio::join!(
      │     transcribe::run(audio/system.wav, MEETING_SYSTEM, PUBLIC),
      │     transcribe::run(audio/mic-internal.wav, MIC_INTERNAL, INTERNAL),
      │   )
      ├── 寫 transcript/system.segments.jsonl + mic-internal.segments.jsonl
      ├── exporter::export(all_segments, session_meta)
      │     → meeting.public.md    (filter visibility=public)
      │     → meeting.internal.md  (filter visibility=internal)
      │     → timeline.json
      └── return session_id

GUI 顯示「✓ 完成 — session id=...」+「開資料夾」按鈕
```

---

## 6. 檔案格式

### Session 目錄

```
~/.mori/meetings/meeting-20260528-143000/
├── audio/
│   ├── system.wav         (16kHz mono 16-bit PCM)
│   ├── mic-internal.wav   (同)
│   └── mix-preview.wav    ← MVP 不做(phase 2)
├── transcript/
│   ├── system.segments.jsonl
│   └── mic-internal.segments.jsonl
├── meeting.public.md
├── meeting.internal.md
└── timeline.json
```

**為什麼 16kHz mono 16-bit**:whisper.cpp 原生輸入,不用 resample。代價:不能拿來事後做高品質回放(MVP 接受;phase 2 改 48kHz + downsample)。

### Segment JSONL(一行一 segment)

```json
{"id":"seg_001","session_id":"meeting-20260528-143000","track":"system","source_kind":"meeting_system","visibility":"public","start_ms":123000,"end_ms":128500,"text":"我們希望下週三前看到版本。","is_final":true,"confidence":-0.142}
```

- 全部 `is_final: true`(沒 live captions 草稿)
- `confidence` = whisper.cpp `avg_logprob`(`--output-json-full` 提供)

### timeline.json(canonical metadata)

```json
{
  "schema_version": 1,
  "session_id": "meeting-20260528-143000",
  "started_at": "2026-05-28T14:30:00+08:00",
  "stopped_at": "2026-05-28T15:15:00+08:00",
  "duration_secs": 2700,
  "tracks": [
    {
      "name": "system",
      "source_kind": "meeting_system",
      "visibility": "public",
      "audio_path": "audio/system.wav",
      "transcript_path": "transcript/system.segments.jsonl",
      "segment_count": 142
    },
    {
      "name": "mic-internal",
      "source_kind": "mic_internal",
      "visibility": "internal",
      "audio_path": "audio/mic-internal.wav",
      "transcript_path": "transcript/mic-internal.segments.jsonl",
      "segment_count": 87
    }
  ],
  "exports": {
    "public": "meeting.public.md",
    "internal": "meeting.internal.md"
  }
}
```

### `meeting.public.md`(僅 public segments)

```markdown
# Meeting Notes — 2026-05-28 14:30

> Source: meeting_system. Mic-internal not included.

[00:02:03] 我們希望下週三前看到版本。
[00:02:18] 那邊測試環境準備好了嗎?
...
```

### `meeting.internal.md`(僅 internal,可引 public 上下文)

```markdown
# Meeting — 內部備忘 — 2026-05-28 14:30

> 包含 mic-internal segments(本機麥克風)。**內部用途,不對外發。**

[00:02:08] (內部)這個時程可能要先保守回覆。
[00:03:42] (內部)等等先別答應 deliverable list。
...
```

---

## 7. GUI(capsule-first)

### Collapsed state(預設) — 小膠囊

```
┌──────────────────────────────────────────────┐
│  ●  00:12:34   [SYS ●]  [MIC ●]      ▶ / ■  │
└──────────────────────────────────────────────┘
```

- Tauri window: `width: 360, height: 60, decorations: false, transparent: true, alwaysOnTop: true, skipTaskbar: false`
- 中間計時器:idle = 00:00:00,recording = elapsed
- 兩個 pill,500ms tick 從 `recorder_status` 更新:
  - 綠 `●`:過去 500ms 有 sample 且 peak RMS > -40dB(收得到聲)
  - 灰 `●`:device open 但靜音 / 沒訊號
  - 紅 `●`:device open failed(配 tooltip 顯示錯誤)
- 右邊單按鈕:idle ▶(start)/ recording ■(stop)/ transcribing 轉圈
- **雙擊膠囊空白處(避開按鈕 / pill)→ 展開**;按 ▶/■ 是 start/stop,**不**觸發展開。Esc → 收回膠囊
- 可拖移(沿 AgentPulse `real-window-move bounce`)

### Expanded state(雙擊展開)

```
┌──────────────────────────────────────────────────────┐
│  Record   Sessions   Deps               ▴  ✕         │
├──────────────────────────────────────────────────────┤
│                                                      │
│  [大 Start/Stop]                                     │
│                                                      │
│  會議音訊  ● 收音中    /home/.../meet-id-abc.monitor │
│  內部麥克風 ● 收音中   default-input                  │
│                                                      │
│  ⚠ 客戶版只使用會議音訊;內部麥克風不會預設匯出給客戶。 │
│                                                      │
└──────────────────────────────────────────────────────┘
```

- `width: 720, height: 480`,有 `decorations: false` 仍維持(可拖移)
- 3 tab:Record / Sessions / Deps(細節見 §4 / §1 已列)
- ▴ 收回膠囊(window.setSize 回 360×60);✕ 關視窗(回 tray)

### Tray icon
- 左鍵:toggle 顯示 / 隱藏膠囊
- 右鍵選單:「顯示主視窗(展開)」「結束」
- `tauri-plugin-single-instance`:第二次啟動 forward 到第一個 instance(避 ghost tray,AgentPulse 已驗證)

### State machine(App.tsx)

```
collapsed ──[ 雙擊膠囊 ]──→ expanded
expanded  ──[ 點 ▴ ]────→ collapsed
expanded  ──[ 點 ✕ ]────→ window.hide() (留在 tray)
*         ──[ 啟動 ]────→ collapsed (預設)
```

每次切換 `invoke("set_window_mode", { mode: "collapsed" | "expanded" })`,後端用 `window.set_size()` + `window.set_decorations()`。

---

## 8. BI-1 manifest

`~/.mori/body-parts/mori.meeting-recorder/manifest.json`(啟動時 overwrite):

```json
{
  "schema_version": 1,
  "id": "mori.meeting-recorder",
  "name": "Mori Meeting Recorder",
  "kind": "standalone_app",
  "description": "Dual-track meeting recorder (system + mic) with visibility-based export. Observer Mode MVP.",
  "capabilities": [
    "audio.capture.system",
    "audio.capture.mic",
    "transcribe.local"
  ],
  "entrypoints": {
    "app": "/absolute/path/to/mori-meeting-recorder/binary"
  },
  "interfaces": [],
  "permissions": [],
  "data_policy": {
    "owns_raw_data": true,
    "default_ingestion": "off"
  }
}
```

- `interfaces: []` — 沒 HTTP / SSE
- `entrypoints.app` = `std::env::current_exe()` 動態填,user 移 binary 也能正確 register
- `data_policy.owns_raw_data: true` — 對齊 doc L51「Mori Desktop / Annuli / Mori agent 不得自動讀取 raw audio」

mori-desktop 那邊的 `body_registry_list` 自動掃到,BodyTab 顯示 entry。**完成等於 mori-desktop 那邊看得到 recorder 存在**(不需要在 mori-desktop 加任何 code)。

---

## 9. Deps 管理 — bundle in repo

每個 repo 自己一套(對齊 [[feedback_bundle_deps_in_repo]])。

### `scripts/install-whisper-linux.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail
ver="${WHISPER_VERSION:-v1.7.0}"
url="https://github.com/ggerganov/whisper.cpp/releases/download/${ver}/whisper-bin-x64.zip"
mkdir -p ~/.mori/bin ~/.mori/models
[ -f ~/.mori/bin/whisper-cli ] || {
  curl -L -o /tmp/whisper.zip "$url"
  unzip -o /tmp/whisper.zip -d ~/.mori/bin/
  chmod +x ~/.mori/bin/whisper-cli
}
[ -f ~/.mori/models/ggml-small.bin ] || \
  curl -L -o ~/.mori/models/ggml-small.bin \
    https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin
echo "✓ ready: ~/.mori/bin/whisper-cli + ~/.mori/models/ggml-small.bin"
```

### `scripts/install-whisper-windows.ps1`

```powershell
$ErrorActionPreference = "Stop"
$ver = if ($env:WHISPER_VERSION) { $env:WHISPER_VERSION } else { "v1.7.0" }
$bin = "$env:USERPROFILE\.mori\bin"
$models = "$env:USERPROFILE\.mori\models"
New-Item -ItemType Directory -Force -Path $bin, $models | Out-Null
if (-not (Test-Path "$bin\whisper-cli.exe")) {
    $url = "https://github.com/ggerganov/whisper.cpp/releases/download/$ver/whisper-blas-bin-x64.zip"
    Invoke-WebRequest -Uri $url -OutFile "$env:TEMP\whisper.zip"
    Expand-Archive -Force "$env:TEMP\whisper.zip" -DestinationPath $bin
}
if (-not (Test-Path "$models\ggml-small.bin")) {
    Invoke-WebRequest -Uri "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin" `
        -OutFile "$models\ggml-small.bin"
}
Write-Host "✓ ready: $bin\whisper-cli.exe + $models\ggml-small.bin"
```

### DepsTab UI

跟 mori-desktop DepsTab 同模式:detect → 顯示狀態(✓/✗)+ 顯示 install 指令 → user 自己貼 terminal 跑 → 點「重新檢測」。**Recorder 不代執行 install**(避免 sudo prompt 走樣;雖然 whisper 不用 sudo,保持「指令在 UI / user 自己跑」一致性)。

---

## 10. 測試策略

### Unit test(cargo test,純函式)

| 模組 | fixture | 測什麼 |
|---|---|---|
| `audio::writer` | 自己生 10ms 正弦波 samples | open / push / finalize → re-open WAV → samples 跟 spec 一致(channels=1, rate=16000, bits=16) |
| `session_store` | TempDir | `new(id, base)` 建出 `<base>/<id>/{audio,transcript}/`;每個 path getter 回正確絕對路徑 |
| `transcribe::parse_whisper_json` | committed `tests/fixtures/whisper-small.json`(從真 whisper.cpp output 截 30 秒小段) | parse → `Vec<Segment>` 含正確 start_ms / end_ms / text / confidence |
| `exporter::export` | feed mixed public + internal segments | public.md 不含 mic_internal;internal.md 不含 meeting_system 之外的 visibility=public(其實是 internal-only);timeline.json 含 2 個 track + 正確 segment_count |
| `manifest::manifest_json` | with fake binary path | JSON parse → id / kind=standalone_app / entrypoints.app 對 |

### Manual e2e(實機跑一次)

**Linux 先全跑一次,Windows 在 cpal WASAPI loopback glue 寫完後同樣流程跑一次**(用一台 Windows 機,或 VM 加音訊 passthrough)。

1. 開 mori-meeting-recorder
2. 確認膠囊預設 collapsed,雙擊展開
3. Deps tab 看到 whisper-cli + model 都 ✓
4. 回 Record tab,接耳機到 PipeWire,開 Google Meet test call(或 YouTube)
5. 點 ▶ → SYS pill 變綠;對麥克風講話 → MIC pill 變綠
6. 等 2 分鐘 → 點 ■ → 看到「轉錄中…」spinner
7. 完成後 `~/.mori/meetings/meeting-*/` 出現雙軌 WAV + JSONL + public.md + internal.md + timeline.json
8. public.md 不含 mic 內容,internal.md 含
9. mori-desktop(如果跑著)BodyTab 看到 `mori.meeting-recorder` entry

### Spike(實作前先做)

**cpal Linux loopback discovery** — 寫 10 行 Rust:
```rust
let host = cpal::default_host();
for d in host.input_devices()? {
    let name = d.name()?;
    if name.contains(".monitor") || name.contains("Monitor") {
        println!("found loopback: {name}");
    }
}
```
在 yazelin 機器跑一次,confirm 能拿到 `alsa_output.*.monitor` 之類的 device。如果 PipeWire 預設沒 expose,可能要 `pactl load-module module-loopback` 之類 — 那就要把這步加進 install script 或 DepsTab 提示。

---

## 11. 風險與限制

| 風險 | 緩解 |
|---|---|
| libpulse Linux:PipeWire monitor source 命名 / 可見性可能因 distro / 配置不一致 | Spike 已驗證 yazelin 機器有 6 個 monitor source(HDMI / 內建喇叭 / USB mic);若 user 機器沒,UI 提示「跑 `pactl load-module module-loopback`」(後續 follow-up:DepsTab 加自動偵測) |
| Windows cpal WASAPI loopback 在 cpal 0.15 行為未實機驗證 | Task 8 完成後在 Windows 機 e2e;若 cpal `default_output_device + build_input_stream` 不走 loopback,fallback 直接調 `windows` crate WASAPI(spec 視為 follow-up,不擋 MVP Linux ship) |
| Tauri 2 transparent + alwaysOnTop on Wayland 行為 | mori-desktop 有經驗(`feedback_no_force_gdk_x11`);走 Wayland native,X11 fallback |
| whisper.cpp release binary 名稱在不同版本可能不同(`whisper-cli` vs `main`) | install script pin `WHISPER_VERSION`;DepsTab 也 detect 兩個檔名 |
| Windows WASAPI loopback 需要 device 是「正在輸出」才能拿到 samples | doc UI 明示;靜音 / 沒輸出時 SYS pill 顯示灰(不是錯誤) |
| `meeting_system` 軌會錄到別 App 的通知 / 媒體聲 | doc L181 已警告;UI hint 加一條「會議外的系統聲也會被錄」 |
| 喇叭漏進 mic(同 doc L182) | 建議耳機使用,UI 文案提示 |

---

## 12. 跟 BI-N 其他軌的關係

| 軌 | 關係 |
|---|---|
| BI-0 MoriPack | 無關(character pack artifact handoff) |
| BI-1 Body Registry | **Recorder self-register manifest**,mori-desktop BodyTab 顯示 |
| BI-2 Permission Broker | **MVP 不整合**。Recorder 是 user-initiated GUI;之後 mori-desktop 主動讀 recorder output 才走 broker(phase 2) |
| BI-3 AgentPulse | 同 standalone-first repo 套路,但 Recorder MVP 沒 HTTP / SSE 對外(`interfaces: []`),比 AgentPulse 簡 |
| BI-4 Cue Center | 無關(MVP 不發 cue) |
| BI-6 Multi-session / Schedule | 之後可能讓 Recorder 變一個 schedulable session 來源 — 不影響 MVP 設計 |

---

## 13. mori-desktop 整合 — clarify

**MVP 範圍內,mori-desktop 不加任何 code**。

mori-desktop 本來就有的 BI-1 BodyTab 會 auto-discover `mori.meeting-recorder` manifest,顯示:
- 名稱、kind=StandaloneApp、capabilities、entrypoint binary path、data_policy
- 這就是 yazelin 講的「mori-desktop 內的功能 = **詳細設定 + 安裝依賴檢查的可見性**」 — user 可以從 desktop 知道 recorder 存在 / 在哪 / 要哪些權限 / 怎麼處理 data

**真的安裝 / dep 偵測 / 設定**:都在 mori-meeting-recorder 自己的 DepsTab + ExpandedView 處理 → 沒 mori-desktop 也能完整跑 ✓

**Phase 2 才考慮**:
- mori-desktop 加 RecorderTab(顯示 ~/.mori/meetings/* 列表,點開資料夾)
- handoff 命令(把指定 public.md 餵 Mori 整理)
- BI-2 broker:mori-desktop 主動讀 recorder output 前要走 broker `audio.transcript.read` 或類似

---

## 14. 下一步

1. Spec self-review(see writing-plans skill)
2. yazelin 看 spec → approve / 改 → 再 review
3. 此 spec 通過 → invoke `writing-plans` skill → 產 implementation plan(每個 task 細到 bite-sized step + code + verify command)
4. plan 通過 → 開始實作:
   - **第 0 步 spike**:cpal loopback discovery
   - **第 1 步**:`gh repo create yazelin/mori-meeting-recorder --public --license MIT` + scaffold Tauri 2
   - 接著按 plan 走

---

**Last updated**: 2026-05-28
**Owner**: yazelin
**Status**: brainstorming locked, awaiting user spec review
