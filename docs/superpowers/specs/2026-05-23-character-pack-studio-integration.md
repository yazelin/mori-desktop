# Character Pack Overhaul + Mori Sprite Studio Integration — Design Spec

**Status**: Approved (2026-05-23)
**Owner**: yazelin
**Source brainstorm**: 2026-05-23 session(在 PR #106 voice inbox 合進 main 後)
**Repurposes**: PR #107(原本只是 cross-platform backdrop,現擴成完整 character pack overhaul)

## 1. 背景

mori-desktop 既有 `~/.mori/characters/<name>/` character pack 系統(`crates/mori-tauri/src/character_pack.rs`),內含:
- `manifest.json` + `sprites/{idle,sleeping,recording,thinking,done,error,walking,dragging}.png`
- 內嵌 8 個 256×256 PNG placeholder bytes,`ensure_default` 用 `tile_4x4_placeholder()` 升 1024×1024 4×4 sheet(動畫 ON 不閃,但 16 格全同 frame)
- `upgrade_pack_to_4x4()` Tauri command 供 user 升級舊 single-frame sprite

yazelin 同時開發 **Mori Sprite Studio**(獨立 repo)— 可視化編輯器,**輸出**就是符合規格的 `*.moripack.zip`(4×4 sheet + backdrop)。Studio 已穩,所以:
- 「placeholder 升級」邏輯**不再需要** — 任何新 sprite pack 都是 Studio 出來的完整 4×4 sheet
- walking / dragging 移動動畫**user 尚未製作** — 可選保留 schema 欄但不再 ship 預設內嵌
- **Backdrop**(背景圖)是 Studio 新加的概念,該整進 character pack 一起 ship

同時 PR #107「cross-platform character backdrop」**已開未合**,記錄 backdrop 跟 sprite layer 分開的設計,但實際 floating render code 還沒接上 backdrop。

本 spec 把 character pack 升級 + Studio 對接 + #107 backdrop layer **整一個 PR ship**。

## 2. 目標

讓 yazelin(跟未來其他人)能用 Mori Sprite Studio 製作角色 → 輸出 `.moripack.zip` → 從 Mori ConfigTab 一鍵匯入 → 即時切換,**無需 hardcode / 重 build / 手動解壓**:

1. Studio 出來的 zip 是 Mori 唯一 sprite 來源規格
2. ConfigTab FloatingMori section 加 character dropdown + import button + metadata 顯示
3. Backdrop 跟 sprite 同屬 character pack — 換角色 = 連帶換背景
4. fresh install 自帶完整 Mori 角色(yazelin Studio 輸出的)而非 256×256 placeholder
5. PR #107 floating backdrop render code 真正接上 character pack

## 3. 非目標

- **Character pack delete UI** — MVP 只 import 不刪,user 手動 `rm -rf ~/.mori/characters/<name>/`
- **Online character pack registry** — 從遠端 list / download 別人 pack,follow-up
- **Studio 反向整合 / deep link** — Mori 開 Studio 編當前角色,follow-up
- **Walking / dragging 真實動作回歸** — schema 仍允許 optional_states 標,但 Mori 端 dispatch 邏輯不動;等 Studio 出新版輸出該 sprite,user re-import 即可
- **Character pack hot edit reload** — user 手改 `~/.mori/characters/X/` 內檔,Mori 不自動 reload(目前要 set_active 切換 trigger),follow-up
- **Multi-version coexist** — `mori v1.0.0` 跟 `mori v1.1.0` 並存,目前 import 直接覆蓋同 package_name(舊版進 backup 目錄)
- **Migration / 強制升級既有 user 的 placeholder mori** — `ensure_default` 尊重既有 `~/.mori/characters/mori/`,user 想升手動 rm + 重啟。Follow-up 加「重置 Mori 角色到出廠版」按鈕

## 4. 設計

### 4.1 架構與資料流

```
Repo:
  examples/characters/mori/                       ← yazelin Studio 輸出 commit 進 repo
    manifest.json
    sprites/{idle,sleeping,recording,thinking,done,error}.png  ← 真 4×4 1024×1024
    backdrop-light.png
    backdrop-dark.png

Build:
  binary 不再 include_bytes! 個別 sprite
  改 include_dir!("examples/characters/mori") — 透過 include_dir crate
  或 build.rs 在 build 時 copy 到 OUT_DIR 再 include

Runtime ensure_default:
  if !~/.mori/characters/mori/manifest.json 存在:
    bundled examples 全 extract 進 ~/.mori/characters/mori/
  else:
    不動(尊重 user state)

Tauri commands(import flow):
  character_pack_import_zip(zip_path) ─→ 讀 bytes
                                    ─→ ZipArchive::new(Cursor)
                                    ─→ 找 manifest.json → parse → validate(§4.3)
                                    ─→ fail: return Err(reason)
                                    ─→ pass: backup ~/.mori/characters/<package_name>/ 若已存在
                                              → unzip 進去
                                              → emit 'character-pack-imported'
                                              → return CharacterEntry

UI (ConfigTab FloatingMori sub-tab):
  Character dropdown + metadata + import button (§4.5)
  
  切 dropdown → invoke set_active → backend emit 'character-changed'
                                  → FloatingMori listener reload sprite + backdrop

Floating layer render:
  Sprite layer:    既有 sprite_path fallback chain(自己 state → mori state → 自己 idle → mori idle)
  Backdrop layer:  CSS `background-image: url(asset://characters/<active>/backdrop-light.png)`
                   html[data-theme-base="dark"] selector 用 backdrop-dark.png
                   兩張任一缺 → CSS background-image: none(degraded sprite-only)
```

### 4.2 Character pack schema(更新)

`manifest.json` 既有 schema 1.0 結構維持,但**默認值跟必含欄位調整**:

```json
{
  "schema_version": "1.0",
  "package_name": "mori",
  "display_name": "Mori",
  "version": "1.0.0",
  "author": "yazelin",
  "license": "CC-BY-NC-SA-4.0",
  "description": "森林精靈,Mori-desktop 預設角色",
  "tags": ["fantasy", "elf", "cute", "official"],
  "states": [
    "idle", "sleeping", "recording", "thinking", "done", "error"
  ],
  "optional_states": ["walking", "dragging"],
  "loop_modes": {
    "idle": "loop", "sleeping": "loop", "recording": "loop",
    "thinking": "loop", "done": "one-shot", "error": "one-shot"
  },
  "loop_durations_ms": {
    "idle": 3000, "sleeping": 5000, "recording": 1500,
    "thinking": 2000, "done": 1000, "error": 2000
  },
  "sprite_spec": {
    "format": "PNG-32",
    "grid": "4x4",
    "total_size": "1024x1024",
    "frame_size": "256x256",
    "frame_order": "row-major-left-to-right-top-to-bottom",
    "background": "transparent"
  }
}
```

**Key changes vs 既有**:
- `states` 從 8 個(含 walking/dragging)縮成 6 個 required
- `optional_states` 改成主要存放 walking / dragging
- `loop_modes` / `loop_durations_ms` 移除 walking / dragging entries(只有 sprite 提供才有意義)

**檔結構**(zip 內 / 解壓後):
```
manifest.json
sprites/idle.png            ← 6 個 required
sprites/sleeping.png
sprites/recording.png
sprites/thinking.png
sprites/done.png
sprites/error.png
sprites/walking.png         ← optional(沒則 fallback chain)
sprites/dragging.png        ← optional
backdrop-light.png          ← optional 但建議都提供
backdrop-dark.png           ← optional 但建議都提供
```

### 4.3 Import 驗證 rules

`character_pack_import_zip` 內部驗證,依序檢查:

| Rule | Fail → reject reason |
|---|---|
| zip 是有效 archive | "Invalid zip archive: {io_err}" |
| zip 內有 `manifest.json` | "Missing manifest.json in zip" |
| manifest.json parse 成 JSON | "Invalid manifest.json: {json_err}" |
| `schema_version` matches `^1\\.\\d+$`(forward compat 認 1.x) | "Unsupported schema_version: {ver} (本機支援 1.x)" |
| `package_name` 非空 + valid dir name(`[a-zA-Z0-9_-]+`) | "Invalid package_name: {name}" |
| `sprite_spec.grid == "4x4"` | "Unsupported sprite grid: {grid} (本機只支援 4x4)" |
| `sprite_spec.total_size == "1024x1024"` | "Unsupported total_size: {size}" |
| zip 內有 `sprites/idle.png` 6 個 required state | "Missing required sprite: {state}.png" |
| 6 個 sprite 都能 PNG decode 為 1024×1024 RGBA(`image::load_from_memory`) | "Required sprite decode failed: {state}.png ({err})" |

通過後**才**做 backup + extract;不通過 user state 不變。

Optional 檢查(只 warn,不 reject):
- walking.png / dragging.png 任一缺 → 不算錯,fallback chain 接
- backdrop-{light,dark}.png 缺 → 不算錯,CSS background-image: none

### 4.4 Cleanup 精確範圍

#### `crates/mori-tauri/src/character_pack.rs`

砍掉:
- `const SPRITE_IDLE` / `SPRITE_SLEEPING` / `SPRITE_RECORDING` / `SPRITE_THINKING` / `SPRITE_DONE` / `SPRITE_ERROR` / `SPRITE_WALKING` / `SPRITE_DRAGGING` 8 個 `include_bytes!` 常數
- `fn tile_4x4_placeholder(png_bytes: &[u8]) -> Result<Vec<u8>>` 整個 fn
- `pub fn upgrade_pack_to_4x4(stem: &str) -> Result<(usize, usize)>` 整個 fn
- `#[cfg(test)] tile_4x4_outputs_1024_square` test
- `#[cfg(test)] tile_4x4_cells_are_identical` test
- `image` crate 用法(若只剩 tile_4x4 用,確認後從 Cargo.toml 砍 dep)

改寫:
- `ensure_default()` — 從 bundled examples extract,不再 include_bytes! + tile_4x4
- `fn default_manifest()` — 不再寫 hardcoded,改用 bundled `examples/characters/mori/manifest.json` 為 source of truth

#### `crates/mori-tauri/src/main.rs`

砍掉:
- 對 `character_pack::upgrade_pack_to_4x4` 的 Tauri command 註冊(若有,grep `upgrade_pack_to_4x4` 確認)
- 對應 invoke_handler entry

加上:
- 新 Tauri command `character_pack_import_zip` 註冊
- `character_pack_set_active` 改 — 加 emit `character-changed` event

#### `src/tabs/ConfigTab.tsx`

砍掉:
- 「升級 placeholder」按鈕 / 區段(若既有有,grep `upgrade_pack` / `upgrade_to_4x4`)

加上:
- §4.5 描述的 character section

### 4.5 ConfigTab FloatingMori section UI

位置:**ConfigTab `floating` sub-tab**(verify path)內加新 `Section title="Character"`,放在 sprite path 設定附近。

Layout(mockup):

```
┌─ Character ─────────────────────────────────────┐
│                                                 │
│  當前角色                                       │
│  ┌──────────────┐                              │
│  │ Mori ▼      │  by yazelin · v1.0.0          │
│  └──────────────┘                              │
│  森林精靈,Mori-desktop 預設角色                 │
│                                                 │
│  [ 匯入 .moripack.zip ... ]                     │
│                                                 │
│  ℹ 製作自己的角色 → Mori Sprite Studio          │
│    (連結到 GitHub repo,user 自己開)            │
│                                                 │
└─────────────────────────────────────────────────┘
```

Component states:
- **Loading**:invoke `character_pack_list` 中,spinner
- **Idle**:dropdown + metadata + import button enabled
- **Importing**:button disabled + 「驗證 + 解壓中…」訊息
- **Success**:綠 toast `✅ 已匯入:{display_name} by {author}`,dropdown 自動切到剛 import 的角色,emit set_active
- **Error**:紅 chip 顯示 `❌ 匯入失敗:{reason}` inline 在 import button 下

Dropdown 切換 active 流程:
1. `setActive(newStem)` invoke `character_pack_set_active(newStem)`
2. backend `set_active` 寫 `~/.mori/characters/active`(既有邏輯)+ **新加 emit `character-changed` event**
3. FloatingMori component listen `character-changed` → re-fetch sprite + backdrop path → 重 render(無需重啟)

### 4.6 Backdrop layer 接到 floating window

`floating.css`(或 FloatingMori scoped CSS):

```css
/* 主 floating container */
.floating-mori-root {
  position: relative;
}

.floating-mori-backdrop {
  position: absolute;
  inset: 0;
  z-index: 1;
  background-image: url(/* set via JS asset:// URL from active character */);
  background-size: contain;
  background-repeat: no-repeat;
  background-position: center;
}

/* light/dark theme attr 切換 */
html[data-theme-base="light"] .floating-mori-backdrop {
  background-image: var(--character-backdrop-light, none);
}
html[data-theme-base="dark"] .floating-mori-backdrop {
  background-image: var(--character-backdrop-dark, none);
}
```

`FloatingMori.tsx` 在拿到 active character 後設 CSS custom property:
```tsx
document.documentElement.style.setProperty(
  '--character-backdrop-light',
  `url(asset://characters/${active}/backdrop-light.png)`
);
document.documentElement.style.setProperty(
  '--character-backdrop-dark',
  `url(asset://characters/${active}/backdrop-dark.png)`
);
```

若該 character 沒 backdrop-{light,dark}.png(import 時驗 optional)→ 設 `none`,sprite-only mode(對齊 PR #107 既有 graceful degradation 設計)。

Sprite layer 不動,仍走 `sprite_path` fallback chain。Sprite 跟 backdrop 是兩個獨立 z-index layer,sprite 永遠在前。

### 4.7 Tauri commands(新加 / 改 / 刪)

| Command | 新/改/刪 | 簽名 | 說明 |
|---|---|---|---|
| `character_pack_import_zip` | 新 | `(zip_path: PathBuf) -> Result<CharacterEntry, String>` | 驗證 + backup + 解壓 + return entry |
| `character_pack_list` | 既有 | `() -> Vec<CharacterEntry>` | 已 expose,verify |
| `character_pack_get_active` | 既有 | `() -> String` | 已 expose,verify |
| `character_pack_set_active` | 改 | `(stem: String) -> Result<(), String>` | 加 emit `character-changed` event |
| `character_pack_upgrade_to_4x4` | 刪 | (若 expose 的話,grep 確認) | 連同 fn 一起砍 |

### 4.8 Bundle yazelin 的 mori.moripack(2).zip 進 `examples/characters/mori/`

操作步驟(impl 階段執行):
1. `unzip /home/ct/下載/mori.moripack\ \(2\).zip -d examples/characters/mori/`
2. 確認結構:`manifest.json` + `sprites/*.png` × 6 + `backdrop-{light,dark}.png`
3. git add + commit 進 PR #107

manifest.json **不**自動修改 — 直接用 Studio 輸出原樣。如果未來 schema 變(例 1.0 → 1.1),Studio 自己出新版就好。

## 5. Error handling(整合 §4.3 + 額外)

| 情境 | 處理 |
|---|---|
| Import zip path 不存在 | Tauri command 回 Err("File not found") |
| Import zip 過大(>50MB?)| MVP 不設上限;follow-up 加 sanity check |
| Import 部分解壓 failed(disk full)| 嘗試 cleanup partial extract,error 顯示「Disk write failed,角色 state 不變」 |
| Import 同 package_name 已存在 | backup `.backup-{unix_ts}/` 然後寫 |
| Import zip 內 manifest.json 跟實際解壓 sprite count 不一致 | 取 manifest 為主,multipart 部分就 fallback chain |
| ConfigTab dropdown 看到 character 但磁碟上沒 manifest | 同 `set_active` fallback `mori`,UI 標記紅 |
| FloatingMori 拿不到 active character sprite(IPC error 等)| log + fallback default mori state chain(既有) |
| Backdrop file 存在但 PNG decode 失敗 | CSS `background-image: none`,log warn,sprite-only mode |
| Theme 切換時 backdrop image 不在 cache | 短暫 flash 沒 backdrop(不影響 sprite) |
| ensure_default 從 bundled examples extract 失敗 | log error,寫空 manifest 進 degraded mori,UI 顯示「⚠ 角色資源異常」chip |
| User 透過 ConfigTab textarea 手改 manifest.json 變無效 | 下次 set_active / list 時驗證,該 pack skipped(既有 `list()` 已有 warn) |

## 6. Testing

### Unit tests(Rust)

| 測試 | 檔 |
|---|---|
| `character_pack_import_zip` PASS for yazelin's mori.moripack(2).zip | `character_pack.rs::tests` |
| reject missing manifest.json | 同 |
| reject schema_version=2.0 | 同 |
| reject missing required sprite | 同 |
| accept missing optional walking/dragging | 同 |
| accept missing backdrop(任一缺) | 同 |
| backup existing same-name pack to `.backup-<ts>/` before overwrite | 同 |
| validate package_name pattern(reject `..` / `/` etc.) | 同 |
| `ensure_default` from bundled examples → sprite count == 6 + backdrop 2 + manifest | 同 |
| `set_active` emit `character-changed` event(integration test 或 mock event sink) | 同 |
| reject zip with malicious path traversal(`../../etc/passwd`)| 同(zip-slip protection) |

### TypeScript / UI tests

無 TS test infrastructure(對齊既有 PR 模式),走 manual smoke。

### Manual smoke(PR checklist)

- [ ] fresh install(`rm -rf ~/.mori/characters/`)→ 啟動 mori-tauri → `~/.mori/characters/mori/` 該有 yazelin 的 4×4 sheet + backdrop,floating sprite 動畫 OK
- [ ] ConfigTab → floating sub-tab → Character section 看到 dropdown「Mori ▼」+ metadata
- [ ] 切 light/dark theme,sprite 不變 + backdrop 換對應檔
- [ ] ConfigTab 點「匯入 .moripack.zip」→ file picker 開 → 選 yazelin 的 zip → 成功 toast → dropdown 列「Mori」(覆蓋自己,有 backup)
- [ ] 故意改 zip 內 manifest.json 為 `schema_version: 2.0` → import → 拒絕 + error 訊息對
- [ ] 故意刪 zip 內 `sprites/idle.png` → import → 拒絕 + 「Missing required sprite: idle.png」
- [ ] 故意刪 zip 內 `backdrop-dark.png` → import → 通過 + backdrop dark mode degraded(無 background)
- [ ] 切 active(若有多角色) → FloatingMori 真的換 sprite + backdrop,無需重啟
- [ ] 升級 PR 後既有 user(~/.mori/characters/mori/ 已有 placeholder 4×4 內容)→ 啟動不被覆蓋(user state 尊重)
- [ ] LogsTab 應有 `character-pack-imported` / `character-changed` event

## 7. 變更影響(file list)

**新檔**:
- `docs/superpowers/specs/2026-05-23-character-pack-studio-integration.md`(本檔)
- `examples/characters/mori/manifest.json`(從 yazelin zip 解壓)
- `examples/characters/mori/sprites/{idle,sleeping,recording,thinking,done,error}.png`(從 zip)
- `examples/characters/mori/backdrop-light.png` + `backdrop-dark.png`(從 zip)

**改既有**:
- `crates/mori-tauri/src/character_pack.rs`(大改:cleanup + new import fn + bundled examples extract)
- `crates/mori-tauri/src/main.rs`(register import command + emit on set_active + 刪 upgrade command)
- `crates/mori-tauri/Cargo.toml`(若可能,砍 `image` dep + 加 `zip` + `include_dir` dep)
- `src/tabs/ConfigTab.tsx`(加 Character section UI + 刪 upgrade button)
- `src/FloatingMori.tsx`(listen `character-changed` event + 設 CSS custom property for backdrop)
- `src/floating.css`(加 backdrop layer rule)
- `docs/character-pack.md`(更新 schema 反映新規格 + import 流程)
- 刪 `crates/mori-tauri/src/character_pack.rs` 內 8 個 include_bytes! + 2 個 fn + 2 個 test

**保留 binary asset**(需要時繼續用 — verify):
- `public/floating/mori-{idle,sleeping,...}.png` 8 個 placeholder PNG — 既有 include_bytes! 來源。新版 `ensure_default` 不再用,但是不是有別處用要 grep

## 8. Migration / Backward compat

**既有 user**(已跑過 v0.7.1 或更早):
- `~/.mori/characters/mori/` 內已有 placeholder 4×4 tile 過的 1024×1024 sheet
- 新版啟動 `ensure_default`:**檢測 manifest 存在 → 不動**(尊重 user state)
- User 想升級到 yazelin Studio baseline → ConfigTab 「重置出廠版」按鈕(follow-up)或 `rm -rf ~/.mori/characters/mori/` 重啟

**Schema version compat**:
- 認 schema_version 1.x(forward compat 1.0 / 1.1 / 1.2 ...)
- 2.x reject(由未來 schema 重大改寫時處理)

**已 import 但壞的 pack**:
- `~/.mori/characters/<broken>/` manifest invalid → `list()` skip + warn(既有邏輯,不動)
- ConfigTab dropdown 不顯示該 pack

## 9. 開放 / Follow-up

- **Character pack delete UI**:dropdown 旁 trash icon
- **Online character pack registry**:`https://github.com/yazelin/mori-characters` 之類遠端 list browse
- **Studio 反向整合**:Mori 開 Studio 編當前角色(deep link `mori-studio://edit?pack=mori`)
- **Walking / dragging 真實動作**:等 Studio 出新版輸出,user re-import 即可
- **Character pack hot edit reload**:user 手改本機 character pack 內檔,Mori 自動 reload
- **Multi-version coexist**:`mori v1.0.0` 跟 `mori v1.1.0` 並存
- **重置 Mori 角色到出廠版按鈕**:ConfigTab 強制 ensure_default --force
- **Generator app 文件對齊**:Mori Sprite Studio README 跟 docs/character-pack.md 連結互引
- **Sanity check import zip size**:>50MB reject(防 user 誤匯入非 sprite 內容)
- **Multi-spirit / multi-user character pack 命名空間**:若未來 spirit 系統演進到每 spirit 自帶 character pack,該怎麼 namespacing
