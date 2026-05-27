# Mori Instance Direction 決議

> 狀態:architecture direction,尚未完整實作。
> 目的:整理 Mori 未來從桌面軟體走向具身代理人的核心方向,釐清 Mori Desktop、
> Mori runtime、Mori body、Mori Hub、World Tree、DDS/ROS2/Zenoh/HTTP 的關係,
> 並列出目前討論中容易互相矛盾的地方與決議。

## 核心定義

Mori 不是 Mori Desktop。

Mori 是一個可運行在某個載體上的智慧代理人,具備 identity、runtime、body
registry、permission broker、session state、memory handoff policy。這個載體現在
可以是一台 Ubuntu / Windows 電腦;未來也可以是一台服務型機器人、一台 AGV 車、一個
捷運站服務處端點,或一組由 hub 管理的機器。

```text
Mori Instance
  identity
  runtime
  body registry
  permission / safety policy
  session state
  cue queue
  memory handoff policy
  transport bindings
```

Mori Desktop 是 Mori Instance 的一種 shell / UI / 互動方式,不是 Mori 本身。它專門
在 Ubuntu Desktop、Windows Desktop 這類桌面作業系統上執行,提供聊天 UI、設定 UI、
body registry UI、cue center UI 與 permission broker UI。

```text
Mori Desktop
  = desktop shell
  = chat UI / interaction UI
  = settings console
  = body registry UI
  = cue center UI
  = local permission broker UI
```

如果同一個 Mori Instance 跑在 AGV 車或服務型機器人的內嵌電腦上,它可能完全不需要
Mori Desktop;或即使有 desktop shell,主要用途也可能只是設定與檢查相連的 body
parts,而不是提供該設備的主要語音服務。未來若 Mori Instance 跑在 Android 手機上,
可能會有另一種 shell,例如 Mori Mobile / Mori Phone。名稱未定,但概念上它和 Mori
Desktop 同屬 shell。

未來的 Mori 可以有身體:

```text
Mori Body
  ears       = microphones / wake word / Mori Ear
  eyes       = cameras / vision recognition
  voice      = TTS / speaker
  display    = screen / signage
  location   = GPS / XYD localization / map pose
  perception = radar / lidar / camera recognition
  mobility   = wheels / navigation / patrol planner
  hands      = manipulator / service actuator
```

如果 Mori 運行在一台 AGV 車或服務型機器人上,可以理解為:

```text
一台車 / 一台機器人 = 一個 Mori Instance
車上的感測器與執行器 = Mori Body Parts
Mori runtime = 這個身體的大腦 / 協調層
```

## 為什麼要這樣定義

如果 Mori Desktop 被當成宇宙中心,所有能力都會變成 tab、設定頁、表格,最後形成一個
高耦合桌面 monolith。這會妨礙:

- Mori Ear 獨立運行。
- Mori Meeting Recorder 獨立開發與測試。
- Agent Plus 觀察多個 CLI / coding agent session。
- LINE / Telegram / Discord / YouTube / OBS connectors 以自己的 runtime 運行。
- 未來服務機器人或 AGV 車把 radar、定位、camera、speaker 接成 body parts。
- 多個 Mori 之間透過 hub / center 協調。

因此 Mori 的長期方向是:

```text
Mori Instance first.
Mori Desktop is one shell.
Body parts are standalone-first.
Hub / World Tree are separate higher layers.
Transport is a binding, not the semantic core.
```

## 層級模型

```text
World Tree / Universe Layer
  shared lore
  public protocol specs
  body interface schemas
  capability vocabulary
  trust / signing rules
  optional catalog / registry

Mori Hub / Center
  routes between Mori instances
  fleet / station / organization overview
  policy distribution
  cross-Mori messages
  audit / sync / dispatch

Mori Instance
  local runtime
  local body registry
  local sessions
  local permission broker
  local cue queue
  local memory handoff

Body Parts
  sensors
  actuators
  local apps
  connectors
  agent/session observers

Shells
  Mori Desktop
  Mori Mobile / Mori Phone
  robot screen
  mobile companion
  web console
  CLI
```

### World Tree

World Tree 是共享規格、世界觀、lore、body interface、capability vocabulary、儀式、
信任根的層。它不是某一台 Mori 的身體,也不是某一台 Mori 的 runtime。

World Tree 可以成為 universe registry / protocol server,但它不應該接管本地
Mori 的 raw private data。

### Mori Hub / Center

Hub 是多個 Mori Instance 的中心,例如:

- 捷運線服務中心。
- 車隊中心。
- 辦公室 Mori 管理中心。
- 多台機器人的 dispatch center。

Hub 可以知道哪些 Mori online、有哪些 capability、有哪些 cue、哪些 session 需要
協調。Hub 可以 route message,但仍要尊重各 Mori Instance 的 permission / data
policy。

### Mori Instance

Mori Instance 是具體的一個 Mori。它可以是:

- 目前的個人桌面 Mori。
- 一台服務機器人 Mori。
- 一台 AGV 車 Mori。
- 一個捷運站服務處 Mori。
- 一個 headless server Mori。

每個 Mori Instance 都應有 stable identity:

```json
{
  "mori_id": "mori.station.taipei-main.service-01",
  "role": "station_service",
  "owner": "metro-operator",
  "location": "taipei-main",
  "capabilities": [
    "speech.input",
    "speech.output",
    "customer_service",
    "station_navigation"
  ]
}
```

### Body Part

Body part 屬於某個 Mori Instance:

```json
{
  "body_id": "mori.station.taipei-main.service-01.ear",
  "parent_mori": "mori.station.taipei-main.service-01",
  "capabilities": ["audio.input", "wake_word", "speech.segment"]
}
```

Body part 可以透過 CLI / HTTP / SSE / WebSocket / Zenoh / DDS / ROS2 接入,但它必須
轉成 Mori semantic event / state / command,而不是讓 Mori agent 直接吃 raw topic。

## 具身 Mori 的資料流

服務型機器人或 AGV 車上的 Mori:

```text
Sensors
  mic / camera / radar / lidar / localization / battery / map
        ↓
Body Adapters
  ROS2/DDS / Zenoh / local driver / HTTP / CLI
        ↓
Mori Semantic Layer
  speech.utterance
  vision.object_detected
  vehicle.pose
  obstacle.nearby
  battery.low
  customer.request
        ↓
Mori Runtime
  session context
  intent routing
  permission broker
  safety policy
  agent reasoning
        ↓
Actions
  speak
  display
  notify staff
  ask hub
  create cue
  propose route
  limited safe commands
```

重要原則:

```text
Raw sensor data is not the agent interface.
Semantic state/event is the agent interface.
```

Mori 可以知道「前方 1.2m 有障礙物」,但不應讓 LLM 直接處理整個雷達點雲。
Mori 可以要求「顯示這段回答」,但不應讓 LLM 直接控制高風險馬達。

## Perception vs Actuation

感測與行動必須分開。

| 類型 | 例子 | 預設政策 |
|---|---|---|
| Perception | mic, camera, radar, localization, battery | 可讀,但需資料政策 |
| Communication | speak, display, send message | 需 channel policy |
| Low-risk action | change UI, create cue, start local recording | scoped allow / ask |
| High-risk action | drive, brake, steering, door, physical actuator | deterministic safety gate |
| Critical action | safety override, emergency stop | 不由 LLM 直接控制 |

Mori agent 可以提出 action proposal:

```json
{
  "type": "action.proposal",
  "action": "navigation.go_to",
  "target": "service_counter",
  "reason": "Customer asked for assistance.",
  "risk": "physical.motion"
}
```

但真正執行必須經過 deterministic controller / safety policy:

```text
LLM proposal
  ↓
Permission / Safety Broker
  ↓
Robot controller / navigation stack
  ↓
Actuator
```

## Transport 與 ROS2/DDS 決議

我們不把 Mori 綁死在 HTTP、DDS、Zenoh 任一種 transport。Mori 的核心是 semantic
contract,transport 只是 binding。

如果 Mori 生長在 ROS2 robot / AGV 系統裡,ROS2/DDS 應該是身體神經系統,不是 Mori
要取代的東西。雷達、定位、camera、nav stack、控制器若已經用 ROS2 topics /
services / actions 溝通,就應保留 ROS2 原生 interface。Mori 不重新發明一套 parallel
robot bus。

Mori 需要定義的是上層語意與治理:

- 這個 ROS node / topic / action 屬於哪個 Mori Instance / body part。
- 它代表什麼 capability。
- Mori agent 能否讀取、提議、執行。
- 是否需要 permission / safety gate。
- 事件能否進 memory / Annuli reflection。
- 是否要變成 cue、artifact、session state 或 hub message。

```text
ROS2/DDS = robot body nervous system
Mori Body Interface = semantic / policy / governance layer
Mori Runtime = brain / coordination layer
```

### Transport 分層

| 場景 | 建議 transport |
|---|---|
| 本機 app / body part 整合 | HTTP/SSE, WebSocket, CLI JSONL |
| 本機 robot stack / sensor-actuator | ROS2/DDS |
| 跨 Mori / 跨站點 / 多設備 pub-sub | HTTP/WebSocket 起步,Zenoh 作未來 binding |
| World Tree / lore / registry sync | Git / HTTPS / API |
| 外部服務 | 各自 connector: webhook / polling / WebSocket / service API |

### DDS/ROS2 的位置

DDS/ROS2 適合 robot / vehicle 內部的即時感測與控制:

```text
/radar/points
/localization/pose
/camera/detections
/battery/state
/cmd_vel
```

Mori 不直接把 DDS topic 當 agent context,也不定義新的 transport 取代 ROS2。
應透過 adapter 把 ROS2 原生 interface 映射成 Mori semantic state / event /
capability / policy:

```text
ROS2/DDS topic / service / action
  ↓
Mori ROS2/DDS Adapter
  ↓
Mori semantic event/state/capability
```

範例:ROS2 原生定位 topic 不需要被 Mori 重做。

```json
{
  "body_id": "mori.vehicle.agv-001.localization",
  "parent_mori": "mori.vehicle.agv-001",
  "interfaces": [
    {
      "transport": "ros2",
      "topic": "/localization/pose",
      "message_type": "geometry_msgs/msg/PoseStamped"
    }
  ],
  "capabilities": ["location.pose.read"],
  "data_policy": {
    "default_ingestion": "metadata"
  }
}
```

範例:ROS2 navigation action 也保留原生 action,但 Mori 宣告它的 capability 與
safety policy。

```json
{
  "capability": "navigation.goal.propose",
  "risk": "physical.motion",
  "execution": {
    "transport": "ros2",
    "action": "/navigate_to_pose",
    "action_type": "nav2_msgs/action/NavigateToPose"
  },
  "policy": {
    "llm_direct_execute": false,
    "requires_safety_gate": true
  }
}
```

### Zenoh 的位置

Zenoh 適合未來跨多 Mori / 多設備 / 多站點的 distributed event bus。它可以成為
Mori-to-Mori 或 Mori-to-Hub transport binding。

```text
mori/instances/{mori_id}/health
mori/instances/{mori_id}/events/**
mori/hubs/{hub_id}/commands/**
mori/sessions/{session_id}/turns/**
```

第一版不把 Zenoh 設為必備依賴,但 schema 必須從現在開始 transport-agnostic。

### HTTP/WebSocket 的位置

HTTP/WebSocket 是第一版最務實的跨站點 / connector transport:

- 容易穿過企業網路。
- 容易做 auth / TLS / logs / retry。
- 容易 debug。
- 容易讓第三方開發者接入。

因此:

```text
MVP uses HTTP/SSE/WebSocket/CLI.
Mori contract stays transport-agnostic.
DDS/ROS2 and Zenoh are future/current specialized bindings, not core assumptions.
```

## 多 Mori 的關係

一個 Mori universe 可以有很多 Mori Instance:

```text
Mori Hub / Center
  ├─ mori.station.a.service-01
  ├─ mori.station.b.service-01
  ├─ mori.vehicle.agv-001
  ├─ mori.vehicle.agv-002
  └─ mori.desktop.yazelin-main
```

跨 Mori 訊息必須是 envelope,不是直接 prompt:

```json
{
  "schema_version": 1,
  "message_id": "msg_001",
  "from": "mori.station.a.service-01",
  "to": "mori.hub.metro-red-line",
  "type": "agent.request",
  "session_id": "sess_123",
  "turn_id": "turn_001",
  "capability_required": ["station.navigation"],
  "payload": {
    "text": "請問往淡水線怎麼走?"
  }
}
```

回覆:

```json
{
  "schema_version": 1,
  "message_id": "msg_002",
  "correlation_id": "msg_001",
  "from": "mori.hub.metro-red-line",
  "to": "mori.station.a.service-01",
  "type": "agent.response",
  "session_id": "sess_123",
  "turn_id": "turn_001",
  "payload": {
    "text": "請往紅線指標方向,搭往淡水或北投方向的列車。"
  }
}
```

## 矛盾與決議

### 1. Mori Desktop 是 Mori 嗎?

矛盾:

- 目前使用時,Mori Desktop 看起來就是 Mori。
- 未來具身後,Mori 會是車或機器人的大腦,不一定有 desktop UI。

決議:

```text
Mori Desktop is a shell.
Mori Instance is Mori.
Mori Runtime is the brain.
Body parts are the body.
```

### 2. 功能要塞進 Mori Desktop,還是拆成獨立 repo?

矛盾:

- 塞進 Mori Desktop 開發快,UI 立即可用。
- 大型能力如 Meeting Recorder、Agent Plus、Mori Ear 需要獨立測試、獨立權限、獨立資料邊界。

決議:

```text
Large capabilities are standalone-first.
Mori Desktop integrates them through Body Interface.
Small tightly-coupled UI features may remain in Mori Desktop.
```

### 3. HTTP 還是 DDS/ROS2/Zenoh?

矛盾:

- HTTP/CLI 容易開發與 debug。
- DDS/ROS2 才像 robot/vehicle 的自然通訊方式。
- Zenoh 適合分散式多設備。

決議:

```text
Semantic schema first.
Transport binding second.
HTTP/SSE/CLI for MVP.
DDS/ROS2 for local robot body.
Zenoh for future distributed Mori-to-Mori / Mori-to-Hub event bus.
```

### 4. Mori agent 能不能直接控制身體?

矛盾:

- 如果 Mori 是大腦,它應該能行動。
- 如果 LLM 直接控制車或機器人,風險太高。

決議:

```text
Mori agent may propose actions.
Safety-critical execution goes through deterministic safety controller.
LLM does not directly control critical actuators.
```

### 5. 外部極簡 agent core 要不要採用?

矛盾:

- PiAgent / PI 類工具簡潔、有 session、provider、tool loop,可以省大量開發。
- 它們可能缺 permission prompt,工具可直接執行,不符合 Mori 的安全模型。

決議:

```text
Mori may use external agent backends through adapters.
External tool execution must be wrapped by Mori Permission Broker or sandbox.
Mori native agent remains the identity/core path.
```

### 6. Body part 產物要不要自動進 memory / Annuli?

矛盾:

- 自動 ingestion 很方便。
- 會議私聊、內部紀錄、感測器資料可能非常敏感。

決議:

```text
Existence metadata may be visible.
Content handoff defaults to explicit.
Ingestion can be enabled by user policy.
Internal/private data never auto-ingests unless a user-created policy explicitly allows it.
```

Memory / Annuli ingestion should be treated like observability policy, closer to log levels than
hardcoded behavior. A body part may emit events and artifacts, but whether those enter short-term
memory, long-term memory, Annuli events, or Annuli reflection should be configurable per body part,
per artifact class, and per visibility level.

Example policy levels:

| Level | Meaning |
|---|---|
| `off` | Do not record beyond local body part state |
| `metadata` | Record existence, timestamps, status, no content |
| `events` | Record structured events and cues |
| `summary` | Record user-approved summaries only |
| `full_public` | Record public artifacts/content |
| `full_internal` | Record internal content; requires explicit user opt-in |
| `reflection` | Allow Annuli to use the material for reflection/digests |

Default:

```text
public artifact  -> metadata/events, user may opt into summary/full_public
internal artifact -> metadata only, user may explicitly opt into summary/full_internal
raw private data -> off unless body-specific policy says otherwise
```

### 7. World Tree 是中心嗎?

矛盾:

- World Tree 像整個 universe 的核心。
- Local Mori 應該可離線運作,不應依賴中央服務。

決議:

```text
World Tree is protocol/lore/registry/governance layer.
It is not required for local runtime.
Local Mori remains sovereign and offline-capable.
```

## 未來發展方向

### Near-term

- 完成 [Mori Body Interface 決議](mori-body-interface.md) 的 manifest / event /
  permission / artifact handoff schema。
- 將 Mori Meeting Recorder 作為 standalone-first body part 實作。
- 將 Agent Plus 作為 cue/session observer body part 實作。
- Mori Desktop 先成為 body registry + cue center 的 shell。

### Mid-term

- 建立 Mori Instance identity model。
- 建立 Mori Hub / Center message envelope。
- 建立 transport binding abstraction。
- 加入 Connector Introspection,讓 CLI help / OpenAPI / skill doc 生成 connector
  schema。
- 建立 memory / Annuli ingestion policy,讓使用者像設定 log level 一樣決定哪些
  body part 的哪些資料可被記憶或反思。

### Long-term

- 支援 ROS2/DDS body adapters。
- 支援 Zenoh distributed event bus binding。
- 支援多 Mori fleet / station / robot center。
- 讓 Mori 成為可具身的代理人:能聽、看、移動、回應、巡邏、服務,但始終受安全層控管。

## 一句話總結

Mori 是一個能運行在不同載體上的智慧代理人。Mori Desktop 只是它現在的桌面外殼。
未來 Mori 可以是一台服務機器人或 AGV 車的大腦;感測器與執行器透過嚴謹的 body
interface 接入;多個 Mori 透過 hub / world-tree 協調;所有 transport 都只是語意
介面的 binding,而不是 Mori 的本體。
