# Character Pack 規範(5P 系統)

Mori 的 floating sprite + 設定打包成 **character pack**,放在
`~/.mori/characters/<name>/`。User 可替換成自製角色 — 設計目標:讓另開的
sprite generator app 能輸出**完全符合規格**的 `.moripack.zip`,user import
後就能在 Config tab 內切到該角色。

---

## 資料夾結構

```
~/.mori/characters/
├── mori/                          ← 預設角色(app 啟動時 ensure 寫入)
│   ├── manifest.json
│   ├── backdrop-dark.png          ← optional,dark theme 時顯示
│   ├── backdrop-light.png         ← optional,light theme 時顯示
│   └── sprites/
│       ├── idle.png                ← 1024×1024 4×4 sheet,16 frame
│       ├── sleeping.png
│       ├── recording.png
│       ├── thinking.png
│       ├── done.png
│       ├── error.png
│       └── (optional) walking.png / dragging.png
├── <user-imported>/                ← user 自製或從 .moripack.zip import
│   ├── manifest.json
│   └── sprites/...
└── active                          ← 一行,當前 active character 名(沒檔回 "mori")
```

---

## Backdrop(optional)

角色作者可以在 pack 根目錄(跟 `manifest.json` 同層,**不在** `sprites/` 下)
放兩張 PNG 當作角色專屬背板:

```
~/.mori/characters/<stem>/
├── manifest.json
├── sprites/...
├── backdrop-dark.png    ← optional,dark theme 時顯示
└── backdrop-light.png   ← optional,light theme 時顯示
```

兩張都是 optional;只有一張也行(對應 theme 沒檔就走下一層 fallback)。

### 顯示條件

`floating.backplate` 缺欄位時由 runtime 依目前平台推導：
X11 因透明 / 半透明支援較脆弱，預設 `"logo"`（顯示背板）；
Wayland / Windows / macOS 預設 `"plain"`（不顯示背板）。
使用者明確改成 `"logo"` 時會跟隨 active character pack 的背板；
明確改成 `"plain"` 時，不論角色提不提供背板都不顯示。

### Fallback chain(高優先到低)

1. character pack 自帶的 `backdrop-{dark,light}.png`
2. 使用者全域 `~/.mori/floating/backplate-{dark,light}.png`
3. 內建預設(shipped)

### 規格建議

- 尺寸:建議 320×320 或更大的方形 PNG(會 `background-size: cover` 縮放填滿 160×160 sprite-area)
- 格式:PNG-32(透明背景 OK,但這層通常做不透明 — 整個區域是 opaque 才能緩解 X11 + WebKit2GTK 的 half-alpha 渲染問題)
- 風格:留 sprite 中央區域空白 / 柔光,避免角色被背板蓋住

---

## `manifest.json` schema(v1.x)

```json
{
  "schema_version": "1.0",
  "package_name": "mori",                          // 唯一 ID(snake_case),= 資料夾名
  "display_name": "Mori",                          // UI 顯示名(可含中文 / emoji)
  "version": "1.0.0",                              // 此 pack 版本(semver)
  "author": "yazelin",
  "license": "CC-BY-NC-SA-4.0",
  "description": "森林精靈,Mori-desktop 預設角色",
  "tags": ["fantasy", "elf", "cute", "official"],
  "states": [
    "idle",
    "sleeping",
    "recording",
    "thinking",
    "done",
    "error"
  ],
  "optional_states": ["walking", "dragging"],      // 沒提供 → fallback 用 default mori
  "loop_modes": {
    "idle":      "loop",
    "sleeping":  "loop",
    "recording": "loop",
    "thinking":  "loop",
    "done":      "one-shot",                      // 跑一輪停在 frame 16
    "error":     "one-shot"
  },
  "loop_durations_ms": {                           // 整 sheet 一輪要多久(ms)
    "idle":      3000,
    "sleeping":  5000,                             // 慢呼吸
    "recording": 1500,                             // 反應感
    "thinking":  2000,
    "done":      600,
    "error":     800
  },
  "sprite_spec": {
    "format":      "PNG-32",
    "grid":        "4x4",                          // "4x4" / "1x1"(static)
    "total_size":  "1024x1024",
    "frame_size":  "256x256",
    "frame_order": "row-major-left-to-right-top-to-bottom",
    "background":  "transparent"
  }
}
```

### Schema 版本規則

`schema_version` 遵守 **1.x forward compat** 策略:

- Engine 接受 `1.0`、`1.1`、`1.2` … 任何 `1.*` 版本 — 不認識的欄位 skip,缺少的 optional 欄位用預設值
- Engine **拒絕** `2.x` 以上 — 顯示錯誤「此角色包需要更新版 Mori-desktop」
- 舊 pack(schema 1.0)在新 engine 上永遠可讀

---

## Sprite states

### Required(必須提供,共 6 個)

| State | 用途 | loop 模式 |
|---|---|---|
| `idle` | 預設待機 | loop |
| `sleeping` | 待機超時或休眠 | loop |
| `recording` | 語音 / 文字輸入中 | loop |
| `thinking` | LLM 處理中 | loop |
| `done` | 回應完成,短暫慶祝 | one-shot |
| `error` | 發生錯誤 | one-shot |

### Optional(選填,共 2 個)

| State | 用途 | 沒提供時的 fallback |
|---|---|---|
| `walking` | 角色移動到新位置時 | 無動畫,直接跳位 |
| `dragging` | 使用者滑鼠拖拽中 | `idle.png` + CSS scale 放大 |

`walking.png` 設計向**右**走即可,engine 用 CSS `transform: scaleX(-1)` 鏡像
向左方向。**注意**:Mori 若手上有不對稱物件(如燈籠),鏡像後會「換手」
— 通常 99% user 不會注意。要 100% 對稱,設計時讓角色雙手都拿 / 或胸前抱。

`dragging.png` 建議姿勢:**腳離地、輕微擺盪、表情驚訝或開心**。

---

## Sprite 規格

每個 state 對應一張 PNG,結構:

```
┌───┬───┬───┬───┐
│ 1 │ 2 │ 3 │ 4 │
├───┼───┼───┼───┤
│ 5 │ 6 │ 7 │ 8 │
├───┼───┼───┼───┤
│ 9 │10 │11 │12 │
├───┼───┼───┼───┤
│13 │14 │15 │16 │
└───┴───┴───┴───┘
```

- **整張**:1024×1024 PNG-32(RGBA,透明背景)
- **每 frame**:256×256
- **順序**:左→右、上→下(row-major)
- **背景**:**完全透明**(無底色、無底紋。陰影由 CSS 加,別畫進 PNG)
- **frame 1 應為「靜態代表姿勢」**:動畫 OFF 時只顯示這格
- **loop 模式 frame 1 與 frame 16 應該銜接平順**(否則 loop 看到跳)
- **one-shot 模式 frame 16 是最終 pose**(動畫播完停這格)

---

## Engine 怎麼播

```css
.mori-sprite-frame {
  background-image: url(<state>.png);   /* 1024×1024 */
  background-size: 400% 400%;            /* viewport 1/4,即 256×256 顯示在 124×124 area */
  animation:
    mori-sprite-x <duration/4>ms steps(4) infinite,  /* x 軸 4 frame 跑一 row */
    mori-sprite-y <duration>ms   steps(4) infinite;  /* y 軸 4 row 跑整 sheet */
}
@keyframes mori-sprite-x { to { background-position-x: -400%; } }
@keyframes mori-sprite-y { to { background-position-y: -400%; } }
```

兩軸獨立 step,自動湊出 row-major 16 frame loop。`duration` 從 manifest
`loop_durations_ms[state]` 拿。

---

## Import flow

### `.moripack.zip` 結構

```
<package_name>.moripack.zip
├── manifest.json               ← 必須在 zip root
├── sprites/
│   ├── idle.png
│   ├── sleeping.png
│   ├── recording.png
│   ├── thinking.png
│   ├── done.png
│   ├── error.png
│   └── (optional) walking.png / dragging.png
├── backdrop-dark.png           ← optional
└── backdrop-light.png          ← optional
```

### 操作路徑

Config tab → Floating sub-tab → Character → **「匯入 .moripack.zip」** 按鈕

### 後台流程

1. **驗 schema**:`schema_version` 必須是 `1.*`,否則 reject
2. **驗 package_name**:只允許 `[a-z0-9_-]`(snake_case),且不得是 `mori`(保護預設角色)
3. **驗 6 required sprites**:每張必須存在且為有效 PNG
4. **驗 sprite 規格**:grid = `4x4`、total_size = `1024×1024`(寬容:4×4 構型正確即接受)
5. **備份舊同名 pack**:若 `~/.mori/characters/<name>/` 已存在,先 rename 成
   `<name>_backup_<timestamp>/`
6. **Extract**:解壓到 `~/.mori/characters/<name>/`
7. **Auto set-active**:寫 `~/.mori/characters/active` 為 `<name>`,並觸發
   `character-changed` event → FloatingMori 即時 reload

### Validation rules 摘要

| 欄位 | 規則 |
|---|---|
| `schema_version` | 必須匹配 `^1\\.` |
| `package_name` | `[a-z0-9_-]+`,不得為 `mori` |
| `grid` | 必須為 `"4x4"` |
| `total_size` | 必須為 `"1024x1024"` |
| required sprites | `idle` / `sleeping` / `recording` / `thinking` / `done` / `error` 全部存在 |
| 每張 PNG | 有效 PNG 格式 |

---

## Migration 保護

**既有 `~/.mori/characters/mori/` 永遠不被 import 覆蓋。**

import 流程拒絕 `package_name = "mori"` 的 zip。預設角色由 app 啟動時 ensure
寫入(只要比較 bundled hash,需要時覆蓋),不受 user import 操作影響。

User import 只會建立 / 更新 `mori/` 以外的目錄。

---

## 設計建議(給 sprite 作者)

### Frame 之間連續性

Loop 模式設計(idle / sleeping / recording / thinking):
- frame 1 跟 frame 16 該銜接平順 — 否則 loop 看到跳
- 通常設計成「對稱波形」:frame 1 = 起點,frame 8 = 最大幅,frame 16 = 回到接近起點
- 例 idle 呼吸:frame 1 胸口正常,frame 8 胸口最高(吸氣頂點),frame 16 回正常

One-shot 模式(done / error):
- frame 1 = 起點(平靜),frame 16 = 最終 pose(燦笑 / 困擾)
- engine 跑完一輪停在 frame 16(`animation-iteration-count: 1, fill-mode: forwards`,
  目前未啟用 — 一律 infinite。Future commit 接)

---

## Creating character packs

**Mori Sprite Studio** 是 yazelin 開發的可視化 character pack 編輯器,直接輸出
符合規格的 `.moripack.zip`(4×4 1024×1024 sprite sheet + manifest.json + backdrop)。

- Repo:https://github.com/yazelin/mori-sprite-studio
- 輸出格式:`.moripack.zip`(可直接拖入 Mori-desktop 匯入)
- 無需手動組裝目錄結構或跑任何轉換腳本

手工製作角色包也完全支援,只要符合上述規格即可。

---

## 給角色設計師的快速開始

1. **參考 default mori**:`~/.mori/characters/mori/` 抓 manifest 結構 + sprite size
2. **設計 6 個 required state**:每張 1024×1024 4×4(16 frame,frame 1 = static rest pose)
3. **選填 2 個 optional state**:walking / dragging(沒有走 fallback,不強制)
4. **打包成 `.moripack.zip`**:zip root 放 manifest.json + sprites/ + 選填 backdrop-*.png
5. **匯入**:Config tab → Floating → Character → 「匯入 .moripack.zip」
6. **自動切到新角色**:`character-changed` event 觸發 FloatingMori 即時 reload sprite

完成。
