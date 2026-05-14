# Ritual ambient audio

Quickstart 的「召喚儀式」模式會嘗試播 `ritual-ambient.mp3`。沒檔案 → fallback Web Audio API 合成的 ambient pad(會像白噪音)。

## 目前 bundled 的音檔

`ritual-ambient.mp3` — "Film" by **Leberch** ([Pixabay 作者頁](https://pixabay.com/users/leberch-42823964/))

授權:Pixabay Content License。商業 / 個人 / 開源 都可用,**不需署名**(但這裡仍註記表示尊重)。可改:user 把同名檔丟進來會覆蓋。

## 找一條音檔

放 `public/audio/ritual-ambient.mp3`,Vite 會 serve 在 `/audio/ritual-ambient.mp3`。建議:

- **長度**:30s ~ 2min,會 loop
- **風格**:ambient pad / 森林 / mystical / 不要太強烈節奏(不該蓋過敘事)
- **格式**:mp3(也支援 ogg / wav,但要改 ritualAudio.ts 的 path)
- **音量**:正規化中等(`-14 LUFS` 之類),別爆音

## 推薦 CC0 / royalty-free 來源

| 來源 | 授權 | 直接下載? |
|---|---|---|
| [Pixabay Music ambient](https://pixabay.com/music/search/ambient/) | Pixabay License(不需署名) | 需 click,但乾淨 |
| [Mixkit peaceful](https://mixkit.co/free-stock-music/mood/peaceful/) | Mixkit License | 需 click |
| [Free Music Archive ambient](https://freemusicarchive.org/genre/Ambient/) | 多數 CC BY / CC0 | 看 track |
| [Freesound ambient loop](https://freesound.org/search/?q=ambient+loop&f=license:%22Creative+Commons+0%22) | 篩選 CC0 | 需登入 |

下載 → 改檔名 `ritual-ambient.mp3` → 放這個資料夾 → 重 reload mori-desktop → 進 ritual mode 就會播。

## 想 commit 進 repo 嗎?

要的話 注意:
- 留在 `public/audio/` 會被 bundle 進 release(增加 binary size 100-500KB)
- 若 LICENSE 要求 attribution,把 credit 加進 README + LICENSES 檔
- CC0 / Pixabay License 不要 attribution,但**禮貌上仍可註記**作者
