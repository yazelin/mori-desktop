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
**3×3 sprite sheet**(同一個 state 的 9 個 motion frame),CSS 引擎
會自動播放。

升級規範:

- 檔名**不變**(還是 `mori-<state>.png`)
- 內部結構:**3 column × 3 row**,共 **9 個 frame**
- **每 frame 256×256**(整張 PNG 就是 768×768),或 **每 frame 512×512**
  (整張 1536×1536)
- 順序:**左→右、上→下**(row-major,跟 CSS animation 預設一致)
  ```
  ┌───┬───┬───┐
  │ 1 │ 2 │ 3 │
  ├───┼───┼───┤
  │ 4 │ 5 │ 6 │
  ├───┼───┼───┤
  │ 7 │ 8 │ 9 │
  └───┴───┴───┘
  ```
- frame 1 跟 frame 9 應該銜接得起來(loop 平滑),除非是
  one-shot 動畫(例如 `done` 是 0.6s 跑一次後停)
- 背景仍是透明,陰影仍由 CSS 加

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
  width: 92px;
  height: 92px;
  background-size: 300% 300%;
  /* 1.2s 跑完 9 格,steps(9) 讓畫面跳格不淡入 */
  animation: mori-cycle 1.2s steps(9) infinite;
}
@keyframes mori-cycle {
  to { background-position: -288px -276px; }  /* (3-1)*size each axis */
}
```
就完成。其他 transform / aura / opacity 的 state-specific 動效不動。

## 預設範例(目前的 placeholder)

`public/floating/mori-*.png` 是用 nanobanana(Gemini Pro Image)從
`docs/design/mori-1.png` 為 reference 生的 9 表情 sprite sheet,
chroma-key 純綠 #00ff00 → 透明,切成 6 個對應 state 的單張圖。
品質夠 demo,但不是最終版 — 等正式設計師把 6 張 3×3 動畫 sheet
畫出來後,直接 overwrite 同名檔案即可。
