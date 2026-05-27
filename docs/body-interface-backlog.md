# Body Interface 實作 Backlog

> **狀態**:decisions locked(2026-05-27 — A1 / B1 / D1 採用,章節名暫掛 placeholder)。**這是 build-now 的執行線,不是再一份架構文件。**
> **目的**:把四份 body-interface 系列決議收斂成「一條可執行、有順序、有依賴」的實作
> backlog;統一編號;畫清「現在做 vs 北極星」;並列出需要 yazelin 拍板的決定點。
> **上游決議**(都已 freeze):
> - [Mori Instance Direction](mori-instance-direction.md) — 最高層北極星
> - [Mori Body Interface](mori-body-interface.md) — 核心契約(manifest / provisioning / transport / permission / ingestion)
> - [MoriPack Integration](moripack-integration.md) — 第一個 artifact-first 樣板
> - [Meeting Recorder](meeting-recorder.md) — 第一個要「移出」的獨立部件

---

## 0. 為什麼有這份文件

四份決議定義的是**架構與契約**,不是**順序**。它們各自暗示了不同的起手順序,而且彼此
對不上(見 [§3 順序收斂](#3-順序收斂a)）。同時 repo 裡已經有三套並存的編號
(`Wave` / `Phase` / 詩意章節),進度永遠對不上。

這份文件做四件事:

1. **畫線**:build-now(desktop body parts)vs 北極星(robot / fleet / ROS2,只影響設計、現在不做)。
2. **收斂順序**:把三份文件互相打架的順序收成一條 canonical sequence。
3. **統一編號**:本軌一律用 `BI-N`(Body Interface stage N)。
4. **列決定點**:把需要 yazelin 拍板的 A / B / D 列成勾選清單。

---

## 1. 範圍紀律 — build-now vs 北極星(決定 E)

> 這套架構橫跨到 AGV / 捷運站 / 機器人車隊。**當北極星極好**(讓今天的每個決定都不會
> 結構性擋住未來);但**接下來幾個月的實作必須狠狠限縮在 desktop body parts**。最大的
> 風險是「在 0 個實作時,先把通用層(registry / broker / ROS2 schema)蓋滿」。

| 層 | 例子 | 現在的態度 |
|---|---|---|
| **Build now** | MoriPack 整合、Body Registry(讀)、最小 Permission Broker、Agent Plus observer、Cue surface、Meeting Recorder | **做**,但每個都 standalone-first、最小可用 |
| **設計影響、現在不做** | manifest 的 transport-agnostic 結構、ingestion-as-log-level 欄位 | schema **保留欄位**,但只實作 HTTP/SSE/CLI binding 與 desktop 需要的部分 |
| **北極星(不寫 code)** | Mori Instance identity model、Mori Hub envelope、ROS2/DDS adapter、Zenoh bus、多 Mori fleet、robot perception/actuation、safety controller | **不做**。只當「新功能設計時的對照」,確保不畫死未來 |

**判準**:一個欄位 / 抽象,如果現在沒有 ≥1 個真實 desktop body part 會用到,就**不實作**,
最多在 schema 裡保留位置 + 註明 `reserved`。

---

## 2. 編號收斂(決定 C)

| 舊編號 | 出處 | 之後怎麼處理 |
|---|---|---|
| `Wave 1-8` | 2026-05 大爆發 + 各 stream | **歷史**。已 ship,不再新增 Wave 編號 |
| `Wave 0-5`(annuli 軌)| `implementation/CHECKLIST.md` | annuli 對接軌,Wave 4 done、Wave 5(creator 拆 repo)延後。**獨立軌,不混進本文件** |
| `Phase 0-5`(body-interface)| `mori-body-interface.md` Migration Plan | 對應到本文件 `BI-N`,該文件保留為「契約」,本文件是「執行順序」 |
| 詩意章節 | `roadmap.md` | 高層願景視圖。本軌的章節名 **待 yazelin 命名**(見下) |

**本軌一律用 `BI-N`。** 詩意章節名我不自己發明(專有名詞 / 詩意命名是你的領域)——
暫掛 placeholder「**眾身之森**」(Mori 可以有很多身體 / 身體部件如林),你要改隨你改。

---

## 3. 順序收斂(決定 A)

三份文件的順序對照:

| 文件 | 它暗示的順序 |
|---|---|
| `body-interface` Migration Plan | Registry → **Agent Plus** → Permission Broker → **Meeting Recorder** → Multi-session |
| `moripack` 移出/移入(L276-313)| MoriPack → **Meeting Recorder** → **Agent Plus** → connectors → Mori Ear |
| `instance-direction` Near-term(L597-606)| schema/provisioning → **Meeting Recorder** → **Agent Plus** → registry/cue shell |

**真矛盾**:`body-interface` 把 Agent Plus 排在 Meeting Recorder **前**;另兩份排在 **後**。

**我的建議解(待你確認)= 採 `body-interface` 的順序,理由:**

1. **Agent Plus 是 observe-only**(只讀 session 狀態、發 cue),風險遠低於 Meeting Recorder 的 `audio.capture`。低風險的先做,讓契約在安全的部件上成熟。
2. **Agent Plus 對你個人 ROI 最高**:你每天在跑 Codex / Claude / Gemini session,「哪個在等輸入 / 跑完了 / 卡住」直接有用。
3. **Meeting Recorder 需要 Permission Broker + audio policy**,這些應該先存在(解掉決定 B)。
4. **MoriPack 第一刀**是 `body-interface` 自己列的 Phase 0 sample(L1010),零權限、runtime 已存在,大家都同意。

> ⚠️ 這推翻了 `moripack` / `instance-direction` 把 Meeting Recorder 排第二的寫法。
> 如果你**主觀上最想先要 Meeting Recorder**(它可能是你最有感的功能),這是你的偏好
> 拍板權 —— 見 [§5 決定點 B](#5-決定點請-yazelin-勾)。

---

## 4. Canonical Backlog(BI-0 → BI-6 + 北極星)

> 每個 stage 標:**驗證什麼契約** / **依賴** / **repo** / **build-now?**。
> 順序是依賴排出來的,不是拍腦袋。

| Stage | 範圍 | 驗證的契約 | 依賴 | repo | tag |
|---|---|---|---|---|---|
| **BI-0** | **MoriPack artifact-first 整合**:Appearance UI 加 Open Studio / Import / Validate / Activate,把 `.moripack.zip` 正式建模成 artifact handoff envelope(`artifact_id/kind/path/visibility/suggested_actions`)| **Artifact handoff** 那半 | 無(character pack runtime + sprite studio #107 已存在)| mori-desktop | ✅ now |
| **BI-1** | **Body Registry(讀)**:manifest reader(`~/.mori/body-parts/*/manifest.json` + `<repo>/.mori-body/`)+ body list/health UI,read-only,**不執行任何高風險操作**。把 BI-0 的 MoriPack 回填成第一個 registered manifest | **Discovery + manifest v1** | BI-0(有真實 manifest 可註冊)| mori-desktop | ✅ now |
| **BI-2** | **Permission Broker(最小版)**:permission request schema + allow/deny/ask + audit log。**刻意提前到 Meeting Recorder 之前**(解決定 B)| **Permission** envelope | BI-1 | mori-desktop | ✅ now |
| **BI-3** | **Agent Plus(observer)**:獨立 repo/sidecar,提供 manifest + `/sessions` + `/events`(SSE);偵測 waiting_input/done/failed。Desktop 顯示 session list + **最小 cue surface** | **Event stream + cue** | BI-1, BI-2 | 新 repo + mori-desktop | ✅ now |
| **BI-4** | **Cue Center**:把 BI-3 的 cue surface 升級成可操作(acknowledge / snooze / jump / dismiss)| Cue lifecycle | BI-3 | mori-desktop | ✅ now |
| **BI-5** | **Meeting Recorder standalone**:依 [meeting-recorder.md](meeting-recorder.md),standalone-first repo;Desktop 只做 launch / recent sessions / public·internal artifact handoff。**此時 broker + event + handoff 都已就緒** | 整套契約合流(audio + 敏感資料邊界 + ingestion policy)| BI-1, BI-2, BI-4 | 新 repo + mori-desktop | ✅ now(大)|
| **BI-6** | **Multi-session / Schedule**:Session Registry、Schedule Manager、sub-agent launcher | 編排層 | BI-3, BI-4 | mori-desktop | 🟡 later |
| **北極星** | Mori Instance identity model、Hub envelope、ROS2/DDS/Zenoh adapter、多 Mori fleet、robot perception/actuation/safety gate | — | — | — | ⭐ 不做,只別擋 |

### 各 stage 的「完成判準」(避免 scope 漂)

- **BI-0 done** = 能從 Appearance 開 Studio → 匯入 zip → 驗證(zip-slip / manifest / required sprites)→ 套用 → floating sprite reload;且匯入流程內部走的是**正式 artifact envelope**(不是隨手寫的 import 函式),這樣才真的驗到契約。
- **BI-1 done** = Desktop 掃到 ≥1 個 manifest(含 BI-0 的 MoriPack)並顯示 install/health,**完全不能**觸發任何 write/exec。
- **BI-2 done** = 有一條 fake 高風險請求被 broker 攔下並寫 audit log;allow/deny/ask 三路徑都有 test。
- **BI-3 done** = 跑一個 Codex session,Desktop 能收到 waiting_input / done cue(不重複)。
- **BI-5 done** = 依 meeting-recorder.md 的 e2e,public/internal 軌分流 + handoff 不自動讀 private raw audio。

---

## 5. 決定點(已鎖定 2026-05-27)

> yazelin 2026-05-27 拍板:A1 / B1 / D1。章節名暫留 placeholder。

### A — Agent Plus vs Meeting Recorder 先後

- [x] **A1(採用)**:採 `body-interface` 順序 = Agent Plus(BI-3)先,Meeting Recorder(BI-5)後。
- [ ] ~~A2:Meeting Recorder 拉前~~

### B — Permission Broker 時機

- [x] **B1(採用)**:Broker 當 BI-2,**在任何 `audio.capture` 部件之前**就位。
- [ ] ~~B2:Broker 延後 + 簡化 consent~~

### D — manifest v1 範圍

- [x] **D1(採用)**:v1 欄位**只放** desktop 前 2 個部件(MoriPack / Agent Plus)真正需要的:`id / kind / entrypoints / interfaces(http·sse·cli)/ capabilities / permissions / data_policy`。`zenoh/ros2/dds` binding 與 robot 欄位**只在 schema 保留、標 `reserved`、不實作**。
- [ ] ~~D2:v1 就做全 binding~~

### 章節命名

- [ ] 詩意章節名:暫掛 placeholder「**眾身之森**」,yazelin 之後決定是否改名(你的領域,我不自作主張)。

---

## 6. 跟既有文件的關係

- 本文件 = body-interface 軌的**執行順序 + 進度**(類似 annuli 軌的 `implementation/CHECKLIST.md`,但分開)。
- `mori-body-interface.md` / 其餘三份 = **契約 source of truth**,不因本文件改動。
- `roadmap.md` =高層詩意願景;本軌完成的 stage 之後回填到 roadmap 對應章節。
- 每個 BI-N 開工前,**先針對該 slice 寫一份聚焦的 implementation plan + TDD**,再動 code(本 backlog 只排順序,不取代 per-slice 的設計)。

---

**Last updated**: 2026-05-27
**Owner**: yazelin
**Next action**: 寫 BI-0(MoriPack)per-slice implementation plan → TDD → code
