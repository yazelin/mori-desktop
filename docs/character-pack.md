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

## `manifest.json` schema(v1.0)

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

`schema_version` 是 forward-compat 鑰匙:engine 讀到不認識的 version 會 warn
+ 嘗試 best-effort 載入(沿用必含欄位)。將來 schema 改變時舊 pack 仍可讀。

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

## 設計建議(給 generator app + sprite 作者)

### 「placeholder 階段」過渡技巧

開發中 sprite 還沒畫完?**16 格全填同張 frame 1**:
- 視覺看起來靜止(每 frame 都長一樣)
- 動畫 ON 跑兩軸 step 不會 visible jump
- 等正式 16-frame motion sheet 上來,覆蓋同檔名就動了

`character_upgrade_pack_to_4x4(stem)` IPC 把 single-frame 256×256 PNG 升 1024×1024
就用這策略(Config tab → Floating section →「升級此 pack 為 4×4 placeholder」按鈕)。

### Frame 之間連續性

Loop 模式設計(idle / sleeping / recording / thinking):
- frame 1 跟 frame 16 該銜接平順 — 否則 loop 看到跳
- 通常設計成「對稱波形」:frame 1 = 起點,frame 8 = 最大幅,frame 16 = 回到接近起點
- 例 idle 呼吸:frame 1 胸口正常,frame 8 胸口最高(吸氣頂點),frame 16 回正常

One-shot 模式(done / error):
- frame 1 = 起點(平靜),frame 16 = 最終 pose(燦笑 / 困擾)
- engine 跑完一輪停在 frame 16(`animation-iteration-count: 1, fill-mode: forwards`,
  目前未啟用 — 一律 infinite。Future commit 接)

### Walk sprite 左右問題

`walking.png` 設計向**右**走即可,engine 用 CSS `transform: scaleX(-1)` 鏡像
向左方向。**注意**:Mori 若手上有不對稱物件(如燈籠),鏡像後會「換手」
— 通常 99% user 不會注意。要 100% 對稱,設計時讓角色雙手都拿 / 或胸前抱。

### Dragging sprite

被滑鼠拎起來的視覺。引擎在 user 滑鼠按住 + 拖曳超過 4px 時切到此 state。
建議姿勢:**腳離地、輕微擺盪、表情驚訝或開心**。沒設計就走 fallback 顯示
idle.png + CSS scale 變大效果。

---

## Import / Export(下版做)

Future:
- ConfigTab Floating section 加「Import character pack」按鈕讀 `.moripack.zip`
- 「Export」按鈕把當前 active pack 打包成 `.moripack.zip`(含 manifest.json +
  sprites/ + 選填 README.md)
- Validate CLI:`mori validate-pack <path>` 給 generator app 整合驗證
- 規範 `.moripack.zip` 結構即 ~/.mori/characters/<name>/ 內檔,壓縮 distribute

---

## 給角色設計師的快速開始

1. **參考 default mori**:`~/.mori/characters/mori/` 抓 manifest 結構 + sprite size
2. **設計 6 個 state**:每張 1024×1024 4×4(16 frame,frame 1 = static rest pose)
3. **複製目錄結構**:`~/.mori/characters/<your-pack>/manifest.json` + sprites/
4. **重啟 mori-desktop**:Config tab → Floating → Character picker 看到你的 pack
5. **切到該 pack**:`character-changed` event 觸發 FloatingMori 即時 reload sprite

完成。
