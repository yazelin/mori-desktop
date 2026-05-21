# Release body 格式

GitHub Release 頁面的 body 統一走這份 template。CHANGELOG.md 是完整工程紀錄,Release body 是給「**準備下載 installer 的人**」看的精簡版 —— 兩者目標讀者不同,**Release body 不要照搬整段 CHANGELOG**。

## 何時用

每次 `git tag v<x.y.z> && git push --tags` 觸發 release.yml 後,GitHub 會建立 draft Release。Publish 前,**人類或 agent** 手動把 draft body 改寫成下面格式。

## Template

```markdown
## Mori v<version> — <one-line tagline>

<2-3 句 hook:這版 ship 什麼 + 為什麼重要。讀者是準備下載 installer 的人,不是 reviewer,語氣偏「這版你會拿到什麼」>

### Highlights

- **<feature 1>** — <一行說明,只寫 user 感知得到的差別,不寫 internal refactor>
- **<feature 2>** — <同上>
- ...(3-6 個 bullet 為佳;超過 6 個代表該分次發或太雜)

### Downloads

- **Windows**:`Mori_<version>_x64-setup.exe`(NSIS,推薦)或 `Mori_<version>_x64_en-US.msi`
- **Linux**:`Mori_<version>_amd64.deb`(Ubuntu / Debian)/ `Mori_<version>_amd64.AppImage`(其他發行版)/ `Mori-<version>-1.x86_64.rpm`(Red Hat / Fedora)

### Verified

實機驗:
- ✅ <被測過跑通的 scenario>
- ✅ <同上,3-8 條為佳>

### 升級

<從前一版升上來要注意什麼。沒有就一行寫「無 breaking change」。有 schema 變動 / config key 改名 / migration script 必跑等都在這列>

### Closes

- #<N> — <一行 issue 描述,GitHub 會自動 cross-link>(若有)
- (沒 close issue 就刪整個 ### Closes section)

完整 changelog 見 [CHANGELOG.md](https://github.com/yazelin/mori-desktop/blob/main/CHANGELOG.md#v<anchor>)。
```

## 各區塊的「該寫 / 不該寫」

### Tagline(`## Mori v<version> — <tagline>`)

- ✅ 用 1 行抓住這版主軸(例:「Windows port 全面修補」「Annuli transparent setup」「Phase 3 Hey Mori 喚醒生態」)
- ✅ 用 `+` 串 2 個主題沒問題(例:「Windows port + verifier-trained wake-word」)
- ❌ 不要寫 PR 編號或 phase 代號(那是內部分期,user 不在意)
- ❌ 不要寫「fix several bugs」「general improvements」這種空話

### Hook(2-3 句)

- ✅ 講「前一版的什麼問題 / 缺口 → 這版解了什麼」(例:「v0.6.6 號稱 Windows first-class,但拿乾淨 Windows 機跑會踩...本版把整條 cycle 在 Windows 跑通」)
- ✅ 對 user 講「故事」: 場景 → 痛點 → 解法
- ❌ 不要列技術堆疊或 dependency 版本
- ❌ 不要把 CHANGELOG 開頭整段貼進來(CHANGELOG 已給 reviewer 看,body 寫給 user 看)

### Highlights(3-6 bullets)

- ✅ 每條 bullet **粗體開頭加一句說明**:`- **<title>** — <description>`
- ✅ 描述 user 感知得到的差別(「Hey Mori 真的能用」「一鍵裝」「不再卡死」)
- ✅ 若是修 bug,用「修了 X」而非「fixed X」— 主動語態 + 中文
- ❌ 不要寫純 internal refactor(「重構 cli_command」「split mori-core」)
- ❌ 不要超過 6 條,超過 → 合併同主題或分次 release
- ❌ 不寫 emoji 除非已是 repo 慣例(過去 v0.6.0 用了 emoji,從 v0.6.5 起 drop 掉,沿用 drop 狀態)

### Downloads

- ✅ 列實際的檔名(不是泛稱「Windows installer」),user 才能對得上 release page 的檔
- ✅ 標哪個推薦(Windows NSIS `.exe` 通常比 `.msi` 推薦,Linux `.deb` / `.AppImage` 平起平坐)
- ❌ 不要寫安裝步驟(那是 README 工作)
- ❌ 不要列 source code zip(GitHub 自動掛,提它沒意義)

### Verified

- ✅ 列「實機跑過確認 work」的 user-facing scenario(不是 unit test)
- ✅ 用 ✅ emoji 開頭表示驗過
- ✅ 跨平台時兩個都驗過才列(只驗 Linux 沒驗 Windows 的就老實寫只 Linux 驗過 / 用 ⚠️ 標)
- ❌ 不要把 verify.sh 內的 unit test 列進來(那是 CI 工作,user 不在意)
- ❌ 沒實測過的別寫 ✅ — 寫 ⚠️ 待驗 或乾脆不列

### 升級 / Migration

- ✅ 沒 breaking change 就直接寫「無 breaking change」(一句話交代)
- ✅ 有的話分 3 種:(1) config schema 變(列 key 對照)、(2) data 結構變(列 migration script)、(3) UI 流程變(列 user 該再做一次什麼)
- ❌ 不要把 CHANGELOG 「### Migration」整段貼過來,只挑「user 必須動手做的事」
- ❌ 沒事不要硬寫「relogin needed」「reset state」之類嚇人話

### Closes

- ✅ 列本版實際 close 掉的 issue,GitHub 自動 cross-link
- ✅ 一行 issue 描述(夠 user 認出來)+ 必要時補一句 root cause(例:「主要 root cause 是 X」)
- ❌ 沒 close issue 就**整個 ### Closes section 刪掉**,不要留空標題

### 結尾連結

- ✅ 一律連到 main 上的 CHANGELOG.md,**錨點對齊 version 標題**(例:`#v070--windows-port-...`),user 點下去看完整紀錄
- ❌ 不要連到自己 release page、不要連到 PR(那讀者已在 release page 了)

## 範例

最近一版 v0.7.0 是按本 template 寫的:https://github.com/yazelin/mori-desktop/releases/tag/v0.7.0

## 提醒 agent

寫 release body 時:
1. **先讀 CHANGELOG.md 對應 version entry** — 那是工程詳實版,你要做的是「**翻成給 user 看的精簡版**」
2. **不要從零想 bullet** — Highlights 直接從 CHANGELOG 提 3-6 個最 user-facing 的,改寫成 user 語言
3. **Downloads 區塊照 release.yml 實際 build 出的檔名寫**,不要憑印象
4. **Verified 區塊只列「已實機跑過」的,沒跑過別寫 ✅**
5. **publish 前讓 user(yazelin)review** — 不要 agent 自己 publish,除非 user 明確說「你 release」
