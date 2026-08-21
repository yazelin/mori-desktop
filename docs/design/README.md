# docs/design

Mori 的視覺設定原檔。

| 檔案 | 是什麼 |
|---|---|
| `mori-1.png`、`mori-2.png` | **角色設計稿正本**。1024×1536，一張紙上放完整立繪、三視圖、Q 版三視圖、表情集、姿態集、細節特寫、sprite 素材、尺寸對照、色彩計畫。角色長什麼樣以這兩張為準。 |
| `mori-full-illustration.webp` | 完整立繪的高解析版，1086×1448。**這是後製重畫的，不是設計稿的原始檔。** 來歷見下面。 |
| `Logo.png`、`mori-logo*.png`、`mori-logo-badge.png` | 品牌識別 |
| `mori-brand.png` | 品牌應用 |
| `mori-desktop-ui.png`、`mori-floating-backplate.png`、`mori-tray.png` | 介面設計稿 |
| `annuli-*.md`、`jarvis-direction-notes.md` | 架構與方向筆記 |

## mori-full-illustration.webp 的來歷

設計稿裡「01. FULL ILLUSTRATION」那一格在原圖上只有 235×410，拿去做網頁的角色卡會糊，而且沒有更大的來源。

放大這條路試過不通：`realesrgan-ncnn-vulkan` 在開發機上三個 GPU index 全部吐全黑圖，退出碼還是 0。

所以改成重畫：把設計稿裁出「完整立繪」與「正面三視圖」兩張乾淨的單人參考圖（沒有標籤文字），照 [cast-lock](https://github.com/yazelin/cast-lock-skill) 的做法把識別特徵與「不可以出現的東西」逐條寫死，交給生圖模型重畫一張，出圖後裁下來放大逐項驗收。

**人與 AI 的分工**：構圖、造型、配色、氛圍全部來自既有的設計稿；重畫這一步由 AI 執行，驗收由人做。它是設計稿的衍生物，**角色正典仍然是 `mori-1.png` 與 `mori-2.png`**，兩者衝突時以設計稿為準。

用途：需要一張夠大的 Mori 立繪時直接拿這張，不必再從設計稿裁。目前用在 [blog 的角色頁](https://yazelin.github.io/characters/mori/)。
