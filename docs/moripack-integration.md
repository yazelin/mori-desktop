# MoriPack Integration 決議

> 狀態:integration design,尚未完整實作。
> 目的:把 Mori Sprite Animation Pack / MoriPack 作為第一個「standalone tool
> ↔ Mori Desktop」整合樣板,定義 artifact handoff、外部 editor、匯入驗證、套用
> 流程,並作為未來功能移出 / 移入 Mori Desktop 的低風險範例。

## 背景

Mori Desktop 目前已經有 Character Pack 系統,詳見
[Character Pack 規範](character-pack.md)。Character pack 放在
`~/.mori/characters/<name>/`,可從 `.moripack.zip` 匯入,並由 floating sprite runtime
讀取 manifest + sprites。

Mori Sprite Animation Pack / MoriPack Studio 是獨立網站 / 獨立 repo / 獨立工具。
它不應被塞進 Mori Desktop 內部。較好的方向是:

```text
MoriPack Studio
  owns:
    sprite editing
    preview
    export .moripack.zip

Mori Desktop
  owns:
    launch/open editor
    import artifact
    validate character pack
    activate character pack
    render floating sprite
```

這個整合可以成為 Mori Body Interface 的第一個實戰樣板,因為它低風險、artifact
邊界清楚、不需要長時間 daemon、不需要 sensitive memory ingestion。

## 核心決議

MoriPack 整合採用 **artifact-first integration**。

第一版不要求 MoriPack Studio 提供 local API,也不要求 Mori Desktop 內嵌 editor。
最小流程:

```text
Mori Desktop → 開啟 MoriPack Studio
MoriPack Studio → 匯出 .moripack.zip
Mori Desktop → 匯入 .moripack.zip
Mori Desktop → 驗證 / 安裝 / 套用
```

未來再升級為:

```text
MoriPack Studio manifest
MoriPack Studio local API
custom URL handoff
direct export to ~/.mori/characters
```

## 為什麼先做 MoriPack

MoriPack 是最適合作為第一個 body/artifact integration 的候選:

- 已經是獨立網站 / repo / tool。
- 產物是明確 artifact:`.moripack.zip`。
- 不需要 background service。
- 不牽涉外部 API token。
- 不牽涉高風險 shell / physical control。
- 不需要進 memory / Annuli。
- 失敗只影響角色外觀,不會破壞 Mori runtime。
- 可以驗證 Mori Desktop 如何與 standalone tool 協作。

這比一開始拆 Meeting Recorder、Agent Plus、Mori Ear 更安全。那些功能更重要,但涉及
音訊、權限、session、長時間 runtime 或 privacy。

## Artifact Contract

MoriPack Studio 的主要 handoff 產物是:

```text
<package_name>.moripack.zip
```

結構沿用 [Character Pack 規範](character-pack.md):

```text
<package_name>.moripack.zip
├── manifest.json
├── sprites/
│   ├── idle.png
│   ├── sleeping.png
│   ├── recording.png
│   ├── thinking.png
│   ├── done.png
│   ├── error.png
│   └── optional walking.png / dragging.png
├── backdrop-dark.png
└── backdrop-light.png
```

Mori Desktop 匯入時應視它為 artifact:

```json
{
  "artifact_id": "character_pack_001",
  "kind": "mori.character-pack",
  "path": "C:/Users/.../Downloads/mori.moripack.zip",
  "visibility": "local",
  "mime": "application/zip",
  "suggested_actions": ["validate", "import", "activate"]
}
```

## Desktop Integration Flow

### Phase 1: Manual artifact handoff

1. 使用者在 Mori Desktop 的 Appearance / Character section 點 `Edit / Create in MoriPack Studio`。
2. Mori Desktop 開啟 MoriPack Studio 網站或本機 app。
3. 使用者在 Studio 編輯角色。
4. Studio 匯出 `.moripack.zip`。
5. 使用者回 Mori Desktop 點 `Import MoriPack`。
6. Mori Desktop 驗證 zip。
7. Mori Desktop 安裝到 `~/.mori/characters/<package_name>/`。
8. Mori Desktop 設為 active 或詢問是否套用。
9. Floating sprite 收 `character-changed` event,即時 reload。

這個 phase 不需要任何 local API。

### Phase 2: Custom URL handoff

MoriPack Studio 可在 export 後呼叫:

```text
mori://character-pack/import?path=<encoded-path>
```

或:

```text
mori://character-pack/import?artifact=<artifact-id>
```

Desktop 收到後仍要:

- 檢查檔案存在。
- 驗證副檔名與 zip 內容。
- 驗證 manifest。
- 讓使用者確認 import。

Custom URL 只負責 handoff,不代表自動信任。

### Phase 3: Local API / body part manifest

若 MoriPack Studio 變成 local app / sidecar,可提供 manifest:

```json
{
  "schema_version": 1,
  "id": "mori.moripack-studio",
  "name": "MoriPack Studio",
  "kind": "standalone_app",
  "capabilities": [
    "character_pack.edit",
    "character_pack.export"
  ],
  "entrypoints": {
    "app": "mori-pack-studio",
    "web": "https://yazelin.github.io/mori-sprite-studio/"
  },
  "interfaces": [
    {
      "name": "control",
      "transport": "http",
      "base_url": "http://127.0.0.1:48910"
    }
  ],
  "permissions": [
    "filesystem.read.character_pack",
    "filesystem.write.character_pack_export"
  ],
  "data_policy": {
    "owns_raw_data": false,
    "default_ingestion": "off"
  }
}
```

可選 API:

```http
GET  /manifest
GET  /health
POST /packs/open
POST /packs/export
```

這不是 MVP blocker。

## Validation Rules

Mori Desktop import 必須繼續做本地驗證,不能信任外部 Studio:

- zip root 必須有 `manifest.json`。
- `schema_version` 必須是支援的 major。
- `package_name` 必須是安全 stem,不可含 path separator。
- required sprites 必須存在。
- 必須防 zip-slip。
- 同名 pack 匯入前要 backup。
- optional backdrop 缺少時可 degraded。
- import 後不可覆蓋 active file 以外的無關使用者資料。

這些規則已在現有 Character Pack 系統中有基礎;本文件只是把它提升成 integration
contract。

## UI Design

Appearance / Character section 應逐步演進:

```text
Character Packs
  Active: Mori

  [Create in MoriPack Studio]
  [Edit current in MoriPack Studio]
  [Import .moripack.zip]
  [Open characters folder]

Installed packs:
  Mori
  User Pack A
  User Pack B
```

第一版 `Edit current` 可以只是:

- 開啟 Studio。
- 開啟目前 pack 資料夾。
- 提醒使用者匯出 zip 後回 Mori Desktop import。

未來 local API ready 後才做直接 open/export。

## Data Policy

Character pack 是 local visual asset。預設不進 memory / Annuli。

```json
{
  "kind": "mori.character-pack",
  "visibility": "local",
  "default_ingestion": "off"
}
```

可以記錄 technical event 到 Logs:

- character_pack_imported
- character_pack_activated
- character_pack_validation_failed

但不需要進 long-term memory 或 reflection,除非使用者把某個角色創作流程明確交給 Mori
討論。

## Body Interface Lessons

MoriPack integration 要驗證這幾件事:

- 外部 standalone tool 不必被塞進 Mori Desktop。
- Mori Desktop 可以只做 launch / import / validate / apply。
- Artifact handoff 可以先於 local API。
- manifest / schema / validation 比深度耦合更重要。
- 使用者可以一邊使用 standalone website,一邊把產物接回 Mori Desktop。

這是未來抽出其他功能的 template。

## 移出 / 移入順序

### 先移入:MoriPack Studio

不是把 Studio code 搬進 Mori Desktop,而是把它接進來:

```text
standalone MoriPack Studio
  → .moripack.zip
  → Mori Desktop import
```

### 再移出:Mori Meeting Recorder

新 repo standalone-first。Desktop 後續只做:

- launch。
- show recent sessions。
- import public/internal artifacts。
- handoff to Mori agent。

### 再移入/整合:Agent Plus

Agent Plus 以 cue/session observer 接入:

- manifest。
- `/sessions`。
- `/events`。
- cue center。

### 後續移入:connectors

LINE / Telegram / Discord / OBS / YouTube Live 依 Mori Connector Contract 接入。

### 最後再處理:Mori Ear / core voice path

Mori Ear / wake-word / voice runtime 較接近現有 Mori 主流程,不要第一個拆。等
Body Interface、permission、audio policy 成熟後再移出。

## Open Questions

- MoriPack Studio 是純 web 還是也要有 local app?
- 是否需要 `mori://` custom URL scheme?
- Browser sandbox 中如何安全 handoff local zip path?
- 是否要支援 drag-and-drop `.moripack.zip` 到 Appearance section?
- 是否要支援 pack preview image / gallery metadata?
- 是否要支援 signed official packs?
- 是否要讓 World Tree / public gallery 成為 character pack catalog?

## 實作進度(BI-0)

- ✅ Phase 1 manual artifact handoff(Open Studio / Import / Validate / Activate / reload)— #107 已落地。
- ✅ Artifact envelope 正式化:`mori_core::body::MoriArtifact` + `classify_artifact` + `inspect_artifact` command;匯入前顯示可見、可取消的 handoff(BI-0,branch `feat/bi-0-artifact-handoff`)。
- ⏳ Phase 2 custom URL(`mori://character-pack/import`)、Phase 3 local API — 未做(YAGNI,等需求)。

## 下一步

1. 保留 [Character Pack 規範](character-pack.md) 作為 zip / manifest source of truth。
2. 在 Mori Body Interface 中把 MoriPack 作為 artifact-first sample。
3. 在 Mori Desktop Appearance UI 加 `Open MoriPack Studio` / `Import MoriPack` 的清楚流程。
4. 後續再評估 custom URL / local API。
