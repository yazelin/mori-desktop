# Floating Mori Sprite Spec

桌面常駐的 floating widget(`crates/mori-tauri/tauri.conf.json` 的 `floating`
window)是一個 **160×160** 透明、無框、永遠在最上層的小 Mori。它根據當下的
`Mode + Phase` 切換到對應的 sprite state,sprite 從 active character pack
(`~/.mori/characters/<active>/sprites/<state>.png`)讀。

完整 character pack 規範見 [`character-pack.md`](character-pack.md)。
本檔聚焦在「state → sprite 對照」+「engine 怎麼播」。

## State → 對應檔名

每個 state 對應 character pack 內一張 sprite:

| 狀態 | 何時 | sprite 檔(在 active character pack 內) |
|---|---|---|
| `idle` | Active mode、phase = Idle | `sprites/idle.png` |
| `sleeping` | Background mode | `sprites/sleeping.png` |
| `recording` | phase = Recording | `sprites/recording.png` |
| `thinking` | phase = Transcribing / Responding | `sprites/thinking.png` |
| `done` | phase = Done(短暫顯示) | `sprites/done.png` |
| `error` | phase = Error | `sprites/error.png` |

預設 character pack 在 `crates/mori-tauri/bundled-character/mori/`(app 啟動時
ensure 寫到 `~/.mori/characters/mori/`)。

## Sprite 檔案規格(5P 起的 4×4 sheet)

每張 sprite 是 **1024×1024 4×4 grid**(16 個 256×256 frame):

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

- **格式**:PNG-32(RGBA),透明背景
- **整張**:1024×1024
- **每 frame**:256×256
- **順序**:左→右、上→下(row-major,跟 CSS animation 預設一致)
- **背景**:完全透明 — 無底色、無底紋,陰影由 CSS 加(別畫進 PNG)
- **frame 1**:靜態代表姿勢 — 動畫 OFF(`Config → Appearance → animated`)時只顯示這格
- **loop 模式**:frame 1 跟 frame 16 應銜接平順,否則 loop 看到跳
- **one-shot 模式**:frame 16 是最終 pose(動畫播完停這格)

對應 manifest 內每 state 的 `loop_mode` / `loop_durations_ms[state]` —
規格細節見 [`character-pack.md`](character-pack.md)。

## 引擎實作(`src/FloatingMori.tsx` + `src/floating.css`)

```tsx
<div className={`mori-sprite mori-sprite-${visual}`}>     {/* 容器套既有 state animation */}
  <div className="mori-sprite-frame" style={spriteStyle(...)} />  {/* 子層跑 sheet loop */}
</div>
```

CSS:

```css
.mori-sprite-frame {
  width: 100%;
  height: 100%;
  background-repeat: no-repeat;
  /* background-image / -size / animation 由 React inline style 套(從 manifest 拿 duration) */
}
@keyframes mori-sprite-x { to { background-position-x: -400%; } }
@keyframes mori-sprite-y { to { background-position-y: -400%; } }
```

**為什麼兩軸獨立**:原本 spec 寫單一 `steps(16)` to `(-372, -372)` 是 buggy 的 —
那是「斜對角線」播放(frame 5 應在 row 1 col 0,但 steps(16) at 5/16 給斜線
中點)。正解:x 軸 `steps(4) duration/4` 跑一 row,y 軸 `steps(4) duration` 跑 4 row,
兩軸組合自然 row-major 16 frame loop。

引擎讀 manifest `loop_durations_ms[state]` 決定每 state duration。

## Character pack 系統(5P)

Sprite 跟角色設定可替換,不再 hardcode `public/floating/`。每個角色是
`~/.mori/characters/<name>/` 一個資料夾(內含 manifest.json + sprites/),
切角色透過 Config tab → Appearance → Character picker。完整規範(manifest
schema、loop_mode、loop_durations_ms、import / export 計畫等)見
[`character-pack.md`](character-pack.md)。

## 預設(目前的 placeholder)

預設 character pack 內的 sprite 是用 nanobanana(Gemini Pro Image)從
`docs/design/mori-1.png` 為 reference 生的角色表情 sprite sheet。品質夠
demo,但不是最終版 — 等正式設計師把 4×4 sheet 畫出來後,直接 overwrite
同名檔案即可(或做成新的 character pack 用 import / export)。
