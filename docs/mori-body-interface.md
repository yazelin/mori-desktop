# Mori Body Interface 決議

> 狀態:architecture decision,尚未完整實作。
> 目的:定義 Mori universe 裡各個「身體部件」如何獨立運行、如何接回 Mori
> Desktop、如何共享設定、如何溝通、如何被權限控管,以及 Mori 未來如何演進成
> multi-session / schedule / cue center。

## 背景

Mori Desktop 目前容易走向「一個功能就是一個 tab / 一張表格」。這種做法短期快,
但長期會讓功能、設定、資料、通知、session 狀態全部塞進同一個 app。當功能開始
包含 Mori Ear、Mori Meeting Recorder、Agent Plus、Annuli、排程、coding agent
session 監控時,單純的 tab 列表會變成高耦合 monolith。

新的方向是:每個重要能力都應該可以是 Mori universe 的一個 **body part**。

```text
Mori Desktop = desktop shell / 設定中心 / Cue center / Permission broker

Mori Ear               = 聽覺部件
Mori Meeting Recorder  = 會議記錄部件
Agent Plus             = Agent / CLI session pulse 部件
Annuli                 = 反思部件
Timebird / Scheduler   = 時間與提醒部件
Mori Bot               = IM / mobile bridge 部件
```

body part 的核心要求:

1. 可獨立執行。
2. 可獨立測試。
3. 可獨立 release。
4. 有明確資料邊界。
5. 透過穩定介面接回 Mori Desktop。
6. 不把 internal/private data 自動交給 Mori agent / Annuli / memory。

Mori Desktop 是 Mori Instance 在桌面作業系統上的 shell / UI / 互動方式。它不是
Mori Instance 的唯一外殼;未來 Mori Mobile / Mori Phone、robot screen、web console、
headless service 都可以是不同 shell。Body Interface 必須服務 Mori Instance,不能只
服務 Mori Desktop。

## 核心決議

Mori universe 採用 **standalone-first, integration-second** 架構。

- 新的大型能力優先做成獨立 repo / 獨立 service / 獨立 CLI。
- Mori Desktop 不直接擁有每個能力的核心 runtime。
- Mori Desktop 只負責 discovery、設定、狀態、cue、啟停、授權、handoff。
- 每個 body part 透過 manifest + local API / CLI / event stream 接入。
- Mori agent 只在使用者明確要求時讀取某個 body part 的輸出。

這不是「把所有東西外包出去」。這是把 Mori 拆成多個低耦合、可獨立進化的身體部位。

本文件描述 body part 接入方式。更高層的 Mori Instance / Mori Hub / World Tree
關係,以及 Mori 從桌面軟體走向具身代理人的方向,見
[Mori Instance Direction 決議](mori-instance-direction.md)。

## 名詞

| 名詞 | 定義 |
|---|---|
| Body Part | 一個可獨立執行、可被 Mori Desktop 管理的能力單元 |
| Body Registry | Mori Desktop 掃描/登錄 body parts 的 registry |
| Capability | body part 對外提供的能力,例如 `audio.capture`, `agent.session.observe` |
| Session | 一條正在進行或可恢復的工作線 |
| Run | 一次具體執行,可能屬於某個 session |
| Cue | 需要使用者或 Mori 注意的事件,例如 done / waiting_input / failed |
| Schedule | 未來某時間或週期性觸發的任務 |
| Handoff | body part 明確把某個 artifact 交給 Mori agent / Annuli / UI |
| Permission Broker | Mori Desktop 統一決定是否允許高風險操作 |

## 不再只是功能表格

Mori Desktop 的「功能表格」如果存在,它必須是 body registry 的動態投影,不是靜態功能清單。

每個 body part 不是一列功能,而是一個帶狀態的節點:

```text
Body Part
  identity
  install status
  runtime status
  capabilities
  settings schema
  permission policy
  health checks
  event stream
  sessions
  artifacts
```

UI 不應只顯示「功能 A / 功能 B」。它應該回答:

- 這個 body part 有沒有安裝?
- 能不能啟動?
- 缺什麼 dependency?
- 目前有幾個 session?
- 有沒有 cue?
- 它會讀哪些資料?
- 它會寫哪些資料?
- 它能不能把資料交給 Mori?
- 哪些權限還沒批准?

## Body Part Manifest

每個 body part 應提供一份 manifest。可以放在 repo root、安裝目錄,或由 local API
回傳。

```json
{
  "schema_version": 1,
  "id": "mori.meeting-recorder",
  "name": "Mori Meeting Recorder",
  "kind": "standalone_app",
  "version": "0.1.0",
  "owner": "mori-universe",
  "description": "Multi-track meeting recorder with public/internal exports.",
  "entrypoints": {
    "app": "mori-meeting-recorder",
    "cli": "mori-meeting-recorder",
    "local_api": "http://127.0.0.1:48731"
  },
  "interfaces": [
    {
      "name": "control",
      "transport": "http",
      "base_url": "http://127.0.0.1:48731"
    },
    {
      "name": "events",
      "transport": "sse",
      "url": "http://127.0.0.1:48731/events"
    }
  ],
  "capabilities": [
    "audio.capture.system",
    "audio.capture.microphone",
    "caption.realtime",
    "meeting.export.public",
    "meeting.export.internal"
  ],
  "settings_schema": "settings.schema.json",
  "permissions": [
    "audio.microphone",
    "audio.system_output",
    "filesystem.write.meetings"
  ],
  "data_policy": {
    "owns_raw_data": true,
    "default_ingestion": "metadata",
    "allow_user_ingestion_policy": true
  }
}
```

### Required manifest fields

| Field | Purpose |
|---|---|
| `id` | Stable integration id,例如 `mori.agent-plus` |
| `kind` | `standalone_app`, `local_service`, `cli`, `crate`, `plugin` |
| `entrypoints` | Mori Desktop 可如何啟動或連線 |
| `interfaces` | transport binding 清單,例如 HTTP/SSE/CLI/WebSocket/Zenoh/DDS/ROS2 |
| `capabilities` | UI / agent / scheduler 能理解的能力 |
| `settings_schema` | 動態設定 UI 的來源 |
| `permissions` | 需要 Permission Broker 管的能力 |
| `data_policy` | 是否擁有 raw data,是否允許自動 ingestion |

## Runtime Topologies

Mori body part 可以有多種 runtime 形態,不強迫全部 Tauri tab 化。

| Topology | 適用 | Mori Desktop 整合方式 |
|---|---|---|
| In-process crate | 小型核心能力,低平台耦合 | Rust API / Tauri command |
| Sidecar local service | 長時間跑、需 event stream | local HTTP + SSE/WebSocket |
| CLI tool | 可批次執行、可被其他 agent 呼叫 | command + JSON stdout |
| Standalone app | 有完整 UI / platform permission | launch + local API / file handoff |
| External app adapter | 第三方 agent / tool | wrapper + event parser + permission gateway |

預設偏好:

1. 大型平台能力:standalone app / sidecar。
2. Agent/session 觀察:sidecar local service。
3. 短任務與自動化:CLI。
4. Mori Desktop 內建必要能力:crate。

## Packaging / Provisioning Layers

Body parts 不只是在 runtime 動態掃描。Mori 應支援四層 provisioning,讓第一方功能可
一鍵安裝,第三方功能也可獨立接入。

```text
1. Compile-time core
   Mori Desktop shell、body registry、permission broker、基本 settings UI。

2. Build-time bundled sidecars
   隨 Mori Desktop installer 一起打包的官方 body part binary / CLI / service。

3. Install-time body packs
   安裝時依 bundle manifest 一次安裝 body parts 與 dependencies。

4. Runtime discovery
   啟動後掃描 ~/.mori/body-parts 或使用者手動加入的外部 body part。
```

這四層可以同時存在。Bundled 不代表 tightly coupled;即使隨 installer 打包,body part
仍應透過 Body Interface 和 Mori Instance 溝通。

### Compile-time core

只放 Mori shell 必備能力:

- Body registry reader。
- Settings shell。
- Cue center shell。
- Permission broker shell。
- Manifest / schema validator。
- Local launcher / health checker。

這一層不應包含所有功能 runtime。Mori Desktop 會逐步從「功能全塞進 app」收斂成
「Mori Instance 的 desktop shell」。

### Build-time bundled sidecars

適合第一方、常用、穩定的 body parts:

- Mori Ear basic runtime。
- Agent Plus basic observer。
- Mori Meeting Recorder launcher。
- TTS / wake-word helper。
- 官方 connector launcher。

打包形態偏好 binary / CLI / sidecar service,而不是 Rust dynamic library plugin。
跨平台 Rust/Tauri 動態 plugin ABI、簽章、更新、安全邊界都比較複雜;sidecar 仍可
獨立更新與獨立 crash。

### Install-time body packs

Body pack 是一組預先定義好的 body parts 與 dependencies。它讓使用者安裝時一次完成
所需組件,不用進 Desktop tab 後逐項裝 dependency。

```json
{
  "bundle_id": "mori.body-pack.meeting",
  "name": "Mori Meeting Pack",
  "includes": [
    "mori.meeting-recorder",
    "mori.whisper-local",
    "mori.ffmpeg"
  ],
  "dependencies": [
    {
      "id": "ffmpeg",
      "install": "system_or_managed"
    },
    {
      "id": "whisper-server",
      "install": "managed",
      "platform": ["windows", "linux"]
    }
  ],
  "register_manifests": true,
  "post_install_health_check": true
}
```

可能的 packs:

- `mori.body-pack.minimal`
- `mori.body-pack.personal`
- `mori.body-pack.meeting`
- `mori.body-pack.agent-ops`
- `mori.body-pack.streaming`
- `mori.body-pack.robot-ros2`

### Runtime discovery

外部開發者最主要靠這一層接入:

```text
~/.mori/body-parts/*/manifest.json
~/.mori/body-parts/*.json
<repo>/.mori-body/manifest.json
manual local API endpoint
```

第三方 body part 只要提供 manifest、settings schema、health、interfaces、
permissions、data policy,就能被 Mori Desktop 發現與接入。

## Extraction Strategy

目前 mori-desktop 已經包含許多功能。遷移策略不是一次拆光,而是先建立中間介面,再
逐步抽出。

```text
Current mori-desktop
  many built-in features
        ↓
Define Body Interface / provisioning / permission / ingestion policy
        ↓
Wrap existing built-in feature behind internal manifest
        ↓
Move implementation to sidecar / standalone repo when stable
        ↓
Mori Desktop keeps shell + registry + settings + cue center
```

判斷某功能是否應移出:

| Signal | Action |
|---|---|
| 平台依賴重 | standalone / sidecar |
| 需要獨立測試 | standalone |
| 有敏感資料邊界 | standalone-first |
| 可能給第三方使用 | body part |
| 只是一小段 UI glue | 留在 Desktop |
| 跟 Mori identity/runtime 高耦合 | 留在 core,但仍暴露 schema |

這表示現在的 Mori Desktop 是未來 Mori Desktop shell 的起點。它不是錯誤方向,只是需要
逐步把高耦合功能搬到 body parts。

## Transport-Agnostic Interface

Body Interface 的核心是 semantic contract,不是某一種 transport。外部 body part
只要能把自己的事件、命令、artifact、permission request 映射到 Mori schema,就能
接入 Mori Instance。HTTP、CLI、DDS、ROS2、Zenoh 都是 transport binding。

這件事必須從 manifest v1 就考慮,否則未來要接 robot / AGV / ROS2 / Zenoh body
parts 時會被 HTTP-only schema 卡住。

但 transport-agnostic 不代表 Mori 要取代 ROS2/DDS。若一個 Mori body 本來就在
ROS2 robot graph 中,它應保留 ROS2 native topics / services / actions。Mori
manifest 只描述這些 ROS interface 在 Mori 世界裡的 body ownership、capability、
permission、safety、memory/Annuli policy。也就是:

```text
ROS2/DDS defines robot communication.
Mori Body Interface defines semantic integration and governance.
```

### Semantic schema

這些資料結構不應綁定單一 transport:

- `MoriEvent`
- `MoriCommand`
- `MoriCommandResult`
- `MoriCue`
- `MoriSessionState`
- `MoriArtifactMetadata`
- `MoriPermissionRequest`
- `MoriHealth`

### Transport bindings

| Binding | 適用 |
|---|---|
| `http` | control API, health, sessions, commands |
| `sse` | local event stream |
| `websocket` | bidirectional realtime control |
| `cli-json` | one-shot command / status |
| `cli-jsonl` | streaming events from CLI |
| `zenoh` | distributed Mori-to-Mori / Mori-to-Hub pub-sub |
| `ros2` | ROS2 topic/service/action binding |
| `dds` | lower-level DDS binding when not going through ROS2 |

Manifest should allow multiple interfaces:

```json
{
  "interfaces": [
    {
      "name": "control",
      "transport": "http",
      "base_url": "http://127.0.0.1:48810"
    },
    {
      "name": "events",
      "transport": "sse",
      "url": "http://127.0.0.1:48810/events"
    },
    {
      "name": "events",
      "transport": "zenoh",
      "key_expr": "mori/body-parts/agent-plus/events/**"
    },
    {
      "name": "pose",
      "transport": "ros2",
      "topic": "/localization/pose",
      "message_type": "geometry_msgs/msg/PoseStamped",
      "capability": "location.pose.read",
      "ingestion": "metadata"
    }
  ]
}
```

MVP may implement only HTTP/SSE/CLI, but schema must not prevent DDS/ROS2/Zenoh.

## Communication Contract

Mori Desktop 和 body part 的溝通分四層。

### 1. Discovery

Mori Desktop 從以下位置掃描 manifest:

```text
~/.mori/body-parts/*.json
~/.mori/body-parts/*/manifest.json
<repo>/.mori-body/manifest.json
```

也可以手動新增 local API endpoint。

### 2. Control

Control API 用於啟動、停止、查狀態。

```http
GET  /health
GET  /manifest
GET  /sessions
POST /sessions
POST /sessions/:id/stop
POST /sessions/:id/export
```

CLI fallback:

```text
mori-meeting-recorder manifest --json
mori-meeting-recorder status --json
mori-meeting-recorder start --json
mori-meeting-recorder stop <session-id> --json
```

### 3. Events

Event stream 用於 cue、session 狀態、progress、health。

建議 SSE 起步,因為比 WebSocket 簡單,足夠單向狀態流:

```http
GET /events
```

event envelope:

```json
{
  "schema_version": 1,
  "event_id": "evt_01",
  "source": "mori.agent-plus",
  "type": "cue.waiting_input",
  "time": "2026-05-27T18:30:00+08:00",
  "session_id": "sess_abc",
  "run_id": "run_123",
  "severity": "attention",
  "summary": "Codex is waiting for user input.",
  "payload": {}
}
```

### 4. Artifact Handoff

body part 產出的資料先留在 body part 自己的 storage。要交給 Mori 時走 handoff。

```json
{
  "artifact_id": "artifact_001",
  "kind": "meeting.public.md",
  "path": "C:/Users/.../.mori/meetings/session/meeting.public.md",
  "visibility": "public",
  "suggested_actions": ["summarize", "extract_action_items"]
}
```

Mori Desktop 不應掃描 raw data 後自動 ingestion。所有 handoff 都要可見、可取消。

## Shared Settings

設定要分層,避免 body part 互相偷讀。

```text
~/.mori/config.json                  # Mori Desktop global settings
~/.mori/providers.json               # shared provider/model references
~/.mori/body-parts/<id>/settings.json # body part settings
~/.mori/secrets/                     # OS keyring backed,不可 plain text 儲 secret
```

### 設定分層規則

| Layer | Example | 誰可讀 |
|---|---|---|
| Global UI preference | theme, language | Mori Desktop |
| Shared provider refs | provider id, model name, local model path | 明確綁定的 body parts |
| Secret values | API keys, OAuth refresh token | OS keyring,只授權指定 body part |
| Body-specific settings | meeting output folder, Agent Plus voice cue | body part owner |
| Project-local settings | `.mori/settings.json` | 使用者明確開啟該 project 時 |

Mori Desktop 可以提供 unified settings UI,但保存時應寫回各 body part 自己的
settings file 或 local API,不要把所有設定集中成一份巨大的 config。

## Memory / Annuli Ingestion Policy

Memory 與 Annuli 都是 Mori 的 observability / reflection sinks。從傳統軟體觀點看,
它們接近 log / trace / audit,只是 Annuli 具備反思與調整未來行為的能力。因此
body part 產物是否進 memory / Annuli 不應寫死,應由使用者與 body-specific policy
控制。

Manifest 可宣告預設 ingestion policy:

```json
{
  "data_policy": {
    "owns_raw_data": true,
    "default_ingestion": "metadata",
    "allow_user_ingestion_policy": true,
    "classes": [
      {
        "kind": "meeting.public.md",
        "visibility": "public",
        "default_ingestion": "summary"
      },
      {
        "kind": "meeting.internal.md",
        "visibility": "internal",
        "default_ingestion": "metadata"
      },
      {
        "kind": "raw.mic_internal.wav",
        "visibility": "private",
        "default_ingestion": "off"
      }
    ]
  }
}
```

Supported levels:

| Level | Meaning |
|---|---|
| `off` | 不記錄到 Mori memory / Annuli |
| `metadata` | 只記錄存在、時間、來源、狀態 |
| `events` | 記錄結構化事件 / cue |
| `summary` | 只記錄使用者批准或 body part 產生的摘要 |
| `full_public` | 記錄 public 內容 |
| `full_internal` | 記錄 internal 內容,必須 explicit opt-in |
| `reflection` | 允許 Annuli 用於 digest / ring / reflection |

Default safe policy:

```text
public artifacts  -> metadata/events,可設定 summary/full_public
internal artifacts -> metadata only,可 explicit opt-in summary/full_internal
raw private data  -> off
```

這讓使用者能像調整 log level 一樣,決定哪些 body part 的哪些資料進入 memory /
Annuli,哪些只停留在該 body part 本地。

## Permission Broker

Mori universe 需要一層權限 broker,特別是當接入外部極簡 agent 框架時。

### Permission classes

| Class | Examples | Default |
|---|---|---|
| `read.public` | 讀 public artifact | allow |
| `read.project` | 讀目前 project | ask / scoped allow |
| `read.private` | 讀 journal、internal transcript | ask every time |
| `write.project` | 改 repo 檔案 | ask / policy |
| `write.private` | 改 memory / identity | deny unless explicit |
| `exec.safe` | `git status`, tests | allow by rule |
| `exec.risky` | package install, network mutation | ask |
| `exec.destructive` | delete, reset, force push | deny / explicit one-shot |
| `audio.capture` | mic/system audio | explicit consent |
| `network.external` | call third-party API | provider/user policy |

### Tool request envelope

任何 body part 或 external agent backend 若要透過 Mori broker 執行工具,都應提出
tool request:

```json
{
  "request_id": "toolreq_001",
  "session_id": "sess_abc",
  "tool": "shell.exec",
  "args": {
    "command": ["cargo", "test", "-p", "mori-core"]
  },
  "scope": {
    "cwd": "C:/Users/yazel/mori-universe/mori-desktop",
    "project": "mori-desktop"
  },
  "risk": "exec.safe",
  "reason": "Run focused tests after code change."
}
```

broker 回覆:

```json
{
  "request_id": "toolreq_001",
  "decision": "allow",
  "lease": {
    "expires_at": "2026-05-27T19:00:00+08:00",
    "max_uses": 1
  }
}
```

### 外部 agent 的權限包裝

某些外部 agent 的設計是「工具直接執行」。公開 PiAgent 文件就明確描述它沒有
permission prompt,並提供 read/write/edit/bash 工具與持久 session。使用者提到的
PyA / Python 極簡 agent 類模組也屬於同一類風險:核心可能很乾淨、很值得採用,但
若它不符合 Mori Body Interface,就不能直接放進 Mori 的信任核心。

Mori 的策略不是 fork 大改外部 agent,而是包一層:

```text
External Agent
  ↓ tool call / command / event
Mori Agent Adapter
  ↓ normalized tool request
Permission Broker
  ↓ allow/deny/ask
Sandbox / Shell / Filesystem
```

可行策略:

1. **CLI wrapper**:用 wrapper script 包外部 agent,攔截環境變數、工作目錄、輸出事件。
2. **Tool shim**:若外部 agent 支援自訂 tool provider,把 read/write/bash 導到 Mori broker。
3. **Filesystem sandbox**:讓外部 agent 只看到 worktree copy / worktree branch。
4. **Shell policy**:不讓它直接拿 unrestricted shell,改成受控 exec endpoint。
5. **Event parser**:解析外部 agent 的 session / waiting / done 事件,交給 Agent Plus。

目標是享受外部 agent 上游更新,但把危險能力收束在 Mori 的 permission layer。

如果外部 A 模組 / PyA 模組本身符合 Mori Body Part Manifest、event envelope、
permission request、artifact metadata,它就能直接作為 body part 接入。如果不符合,
可以包一層 adapter:

```text
External PyA / Agent Module
  ↓ native API / CLI / events
Mori Agent Adapter
  ↓ Mori Body Interface
Mori Instance
```

這表示 Mori 可以採用外部極簡 agent core,但採用的是「backend / body part /
sub-agent」角色,不是無條件把它變成 Mori identity 本身。

## Agent Core Strategy

Mori 不必現在決定「全部自研」或「完全採用外部 agent」。應保留三種 backend:

| Backend | 用途 | 風險 |
|---|---|---|
| Mori Native Agent | 最小、可控、深度整合 memory/skills/permissions | 要自行維護 |
| External PI/PiAgent Adapter | 借用成熟 session/provider/tool loop | 權限與資料邊界要包裝 |
| CLI Batch Adapters | Codex/Claude/Gemini 等 batch/session | 重、但能力強;適合作 sub-agent |

Mori 主 agent 應維持自己的 minimal core:

```text
Mori Native Agent
  prompt/context builder
  provider router
  tool registry
  permission broker client
  memory handoff policy
  session state
```

外部 agent 則作為可替換 backend / sub-agent,不是 Mori identity 的核心。

### 是否採用外部極簡 agent core

採用條件:

- 可把 tool execution 導入 Mori Permission Broker。
- session 存儲可被 Mori registry 掃描或 adapter 對接。
- provider settings 可映射到 Mori shared provider refs。
- 不要求 raw private data 自動進入它自己的 context。
- 上游更新不需要大量 fork patch。

不採用條件:

- 工具執行無法攔截。
- session 格式封閉且無法 export。
- 權限模型只能全開或全關。
- 強迫 central auth / SaaS relay。

## Agent Plus

Agent Plus 是 Mori universe 的 Pulse / Cue collector。它不必一開始負責主控,
先負責觀察多個 command line / coding agent session。

### Agent Plus owns

- CLI / terminal / coding agent hook。
- session 狀態偵測。
- waiting input / done / failed / blocked cue。
- 語音或桌面通知。
- 多 session list。
- event stream。

### Agent Plus does not own

- Mori main agent identity。
- Memory ingestion。
- Annuli reflection。
- 高風險 shell permission。
- body part registry。

### Agent Plus event types

```text
agent.session.started
agent.session.updated
agent.session.waiting_input
agent.session.done
agent.session.failed
agent.session.blocked
agent.session.needs_review
agent.session.long_running
agent.session.cost_warning
agent.session.context_warning
```

### Session record

```json
{
  "session_id": "codex_20260527_001",
  "backend": "codex",
  "cwd": "C:/Users/yazel/mori-universe/mori-desktop",
  "title": "Meeting Recorder docs",
  "status": "waiting_input",
  "started_at": "2026-05-27T18:00:00+08:00",
  "updated_at": "2026-05-27T18:30:00+08:00",
  "last_cue": "waiting_input",
  "open_command": "codex resume codex_20260527_001",
  "risk": "normal"
}
```

### Mori Desktop integration

Mori Desktop 對 Agent Plus 的第一階段整合:

- 顯示 session list。
- 顯示 cue queue。
- 點擊 session 可複製/執行 resume command。
- 全域通知 / TTS 可由 Mori Desktop 或 Agent Plus 任一方發出,但不要重複。
- 使用者可設定哪些 backend 要監控。

第二階段:

- Mori Desktop 可啟動 sub-agent run。
- Schedule 可觸發 Agent Plus 監控的 command。
- Agent Plus cue 可進 Mori 的 cue center。

## Multi-Session Mori

Mori 目前是一條線。未來要變成:

```text
Main Mori
  Session Registry
  Cue Queue
  Schedule Manager
  Permission Broker
  Body Registry
  Agent Dispatcher

Sub-Agent Runs
  Codex session
  Claude session
  Gemini session
  PI/PiAgent session
  shell task
  Mori native task
```

### Main Mori responsibilities

- 決定哪些 cue 需要 user。
- 決定哪些 task 可排程。
- 決定哪些 sub-agent backend 適合某任務。
- 管理 permission policy。
- 管理 handoff 到 memory / Annuli。
- 維持 Mori identity 和 user preference。

### Sub-agent responsibilities

- 完成局部任務。
- 回報狀態。
- 產出 artifact。
- 不直接寫 Mori memory。
- 不越權讀 private body part data。

### Session lifecycle

```text
created
queued
running
waiting_input
paused
blocked
failed
completed
archived
```

### Cue lifecycle

```text
created
notified
acknowledged
delegated
resolved
dismissed
expired
```

Cue 不等於 notification。Cue 是狀態物件;notification 只是其中一種呈現。

## Schedule Manager

Schedule 是未來能力,不應混在 Annuli。Annuli 是反思過去;Schedule 是觸發未來。

Schedule 可觸發:

- reminder。
- body part command。
- Agent Plus monitored command。
- Mori native prompt。
- sub-agent run。
- Annuli sleep / digest request。

Schedule record:

```json
{
  "schedule_id": "sched_001",
  "kind": "one_shot",
  "time": "2026-05-28T09:00:00+08:00",
  "action": {
    "type": "agent.run",
    "backend": "codex",
    "prompt": "Review open PR checks and summarize failures."
  },
  "permission_profile": "ask_before_run",
  "created_by": "user"
}
```

安全規則:

- schedule 不應自動執行高風險命令。
- schedule 觸發 agent run 時仍走 Permission Broker。
- schedule 的 cue 要進 Cue Queue。
- schedule 完成事件可選擇性送 Annuli,但不預設。

## Body Part Data Policy

每個 body part 必須宣告資料政策。

| Data class | Example | Default policy |
|---|---|---|
| Public artifact | `meeting.public.md` | 可 handoff |
| Internal artifact | `meeting.internal.md` | user explicit handoff only |
| Raw private audio | `mic-internal.wav` | never auto-read |
| Session metadata | status, duration, backend | registry 可讀 |
| Secret | API key, OAuth token | OS keyring only |
| Derived summary | action items, digest | depends on source visibility |

對 Mori agent 的核心規則:

```text
Mori may know that a private artifact exists.
Mori may not read it unless the user explicitly selects it.
```

## UI Model

Mori Desktop 應該有一個 Body / Integrations control surface。

### Body Dashboard

顯示:

- Body part name / icon / status。
- Install state。
- Health。
- Active sessions。
- Recent cues。
- Missing permissions。
- Missing dependencies。
- Last artifact。

### Body Detail

每個 body part detail 頁:

- Overview。
- Settings。
- Permissions。
- Sessions。
- Events。
- Artifacts。
- Logs。
- Handoff actions。

### Cue Center

Cue center 不只顯示通知,還要能操作:

- acknowledge。
- snooze。
- jump to session。
- delegate to sub-agent。
- convert to reminder。
- dismiss。

### Settings

Settings UI 要由 schema 生成,但要支持 Mori 風格的 grouping:

```text
General
Provider / Models
Permissions
Storage
Notifications
Advanced
```

## Versioning

所有 cross-body contract 都要版本化。

```text
manifest.schema_version
event.schema_version
settings.schema_version
artifact.schema_version
permission.schema_version
```

Mori Desktop 對舊版 body part:

- 能讀就降級讀。
- 不懂的 capability 不顯示。
- 不懂的 event type 記錄但不 crash。
- 對高風險未知 permission 預設 deny。

## Testing Strategy

每個 body part repo 都應有:

- manifest validation。
- settings schema validation。
- event contract tests。
- permission request tests。
- fixture sessions。
- export/handoff tests。
- local API health tests。

Mori Desktop integration tests:

- 掃描 fake body part manifest。
- 連 fake local API。
- 收 fake event stream。
- 顯示 session list。
- cue lifecycle。
- permission allow/deny。
- artifact handoff 不自動讀 private data。

Agent Plus specific tests:

- 偵測 done。
- 偵測 waiting input。
- 多 session list。
- backend crash recovery。
- duplicate cue suppression。
- notification throttling。

## Migration Plan

### Phase 0: Docs and contracts

- 建立本文件。
- 為 Mori Meeting Recorder 保留 standalone-first 決議。
- 為 Agent Plus 寫 integration spec。
- 決定 manifest / event / permission schema 初版。

### Phase 1: Body Registry skeleton

- Mori Desktop 新增 body registry reader。
- 支援讀 local manifest。
- UI 顯示 body list + health。
- 不執行任何高風險操作。

### Phase 2: Agent Plus standalone

- Agent Plus 獨立 repo / app。
- 提供 manifest。
- 提供 `/sessions` 和 `/events`。
- Mori Desktop 顯示 session list / cue queue。

### Phase 3: Permission Broker

- 建立 permission request schema。
- 支援 allow/deny/ask。
- 支援 per-project policy。
- 支援 audit log。

### Phase 4: Mori Meeting Recorder standalone

- 依 [Meeting Recorder 決議](meeting-recorder.md) 實作。
- 接 body manifest。
- public/internal artifact handoff。

### Phase 5: Multi-session / Schedule

- Session Registry。
- Cue Center。
- Schedule Manager。
- Sub-agent launcher。
- Agent backend adapters。

## Open Questions

- Mori Desktop 是否要內建 local API gateway,讓 body parts 只接一個固定 endpoint?
- body part 的 auth 要用 localhost token、named pipe,還是 OS user boundary 即可?
- Agent Plus 要不要自己發 TTS,還是只發 cue 讓 Mori Desktop 發 TTS?
- 外部 agent backend 的 tool shim 能做到多深?哪些只能用 worktree sandbox 包裝?
- Schedule 觸發 sub-agent 時,使用者 approval 要在觸發前、觸發時,還是每次 tool call?
- internal artifacts 的預設 retention policy 要由 body part 決定,還是 Mori Desktop 統一提供?

## Reference Notes

公開 PiAgent / PI 文件顯示,它以 VSCode extension / pi CLI / pi-coding-agent 為核心,
具備多 provider、read/write/edit/bash 工具、共享 `~/.pi/agent/` 設定與 session
持久化,並明確說明沒有 permission prompts。這類設計的優點是極簡與高效率,但 Mori
若接入,必須透過 adapter / permission broker / sandbox 包裝,不能直接把無限制工具
執行當作 Mori 的安全模型。

參考:

- PiAgent VSCode Marketplace:
  <https://marketplace.visualstudio.com/items?itemName=brijbyte.piagent-vscode>
- Smithers PI Integration:
  <https://smithers.sh/integrations/pi-integration>
- pyagent historical minimal infrastructure:
  <https://pyagent.sourceforge.net/>
