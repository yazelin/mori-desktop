# Floating Mori Sprite Spec

The floating widget(`crates/mori-tauri/tauri.conf.json` 的 `floating`
window)是一個 96×96 透明、無框、永遠在最上層的小 Mori。它從
`/floating/mori-<state>.png` 讀取 6 張 sprite,根據當下的 `Mode +
Phase` 切換顯示。

## State → 檔名

| 狀態 | 何時 | 檔名 |
|---|---|---|
| `idle` | Active mode、phase = Idle | `public/floating/mori-idle.png` |
| `sleeping` | Background mode | `public/floating/mori-sleeping.png` |
| `recording` | phase = Recording | `public/floating/mori-recording.png` |
| `thinking` | phase = Transcribing / Responding | `public/floating/mori-thinking.png` |
| `done` | phase = Done(短暫顯示) | `public/floating/mori-done.png` |
| `error` | phase = Error | `public/floating/mori-error.png` |

## 檔案規範(目前用的「靜態」版本)

- **格式**:PNG-32(RGBA),透明背景
- **解析度**:**256×256**(最低),建議 **512×512** 以上(retina hi-DPI 會 2× 顯示)
- **構圖**:角色置中,周圍留 ~10% padding,**不要在 PNG 內畫陰影**(CSS 自己加 drop-shadow,我們才好控)
- **背景**:**完全透明**,無底色、無底紋。
  - 若藝術家用綠幕畫,提供前先用工具去背 → PNG 帶 alpha
  - 我們有 `scripts/`(待建)裡的 chroma-key 腳本可參考
- **風格**:對齊 `docs/design/mori-1.png` / `docs/design/mori-2.png` 的
  Q 版森林精靈樣式(綠髮、葉飾、botanical 服裝、淡膚色、尖耳)

## 升級到 sprite animation(未來)

目前 6 張都是靜態圖。等藝術家有空時,把每張 PNG 升級成
**4×4 sprite sheet**(同一個 state 的 16 個 motion frame),CSS 引擎
會自動播放。

升級規範:

- 檔名**不變**(還是 `mori-<state>.png`)
- 內部結構:**4 column × 4 row**,共 **16 個 frame**
- **每 frame 256×256**(整張 PNG 就是 1024×1024)— 跟現在 single-frame
  尺寸一致,引擎只要改 `background-size` 即可
- 順序:**左→右、上→下**(row-major,跟 CSS animation 預設一致)
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
- frame 1 跟 frame 16 應該銜接得起來(loop 平滑),除非是
  one-shot 動畫(例如 `done` 是 0.6s 跑一次後停)
- 背景仍是透明,陰影仍由 CSS 加
- 16 frame 比 3×3 (9 frame) 動畫更流暢,適合 idle 呼吸 / talk
  口型 / walk 步伐這類需要中間 frame 銜接的動作

引擎側只要把 `src/FloatingMori.tsx` 裡那行 `<img src=...>` 換成:
```tsx
<div
  className="mori-sprite"
  style={{ backgroundImage: `url(${SPRITE_SRC[visual]})` }}
/>
```
然後 CSS 改成:
```css
.mori-sprite {
  width: 124px;
  height: 124px;
  background-size: 400% 400%;
  /* 1.6s 跑完 16 格,steps(16) 讓畫面跳格不淡入 */
  animation: mori-cycle 1.6s steps(16) infinite;
}
@keyframes mori-cycle {
  /* (4-1)*size 各軸 = -3*124 = -372px */
  to { background-position: -372px -372px; }
}
```
就完成。其他 transform / aura / opacity 的 state-specific 動效不動。

## 預設範例(目前的 placeholder)

`public/floating/mori-*.png` 是用 nanobanana(Gemini Pro Image)從
`docs/design/mori-1.png` 為 reference 生的 9 表情 sprite sheet,
chroma-key 純綠 #00ff00 → 透明,切成 6 個對應 state 的單張圖。
品質夠 demo,但不是最終版 — 等正式設計師把 6 張 3×3 動畫 sheet
畫出來後,直接 overwrite 同名檔案即可。
