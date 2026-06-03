# BI-5 follow-up — Recorder ↔ Desktop 雙向偵測 + tray 整合(A+B)

> **狀態**:設計定案、§8 決定已鎖定(2026-06-04 yazelin 全採建議);待 go + 確認時機後動工(動 mori-desktop Rust 會中斷語音)。
> **範圍**:BI-5 當初刻意延後的「desktop 端那一半」(backlog §109 phase 2:mori-desktop launch / RecorderTab)。
> **原則**:**雙向偵測 + 自適應 UI** — 兩個 app 各自偵測對方是否在場,在場時各自調整 UI 呈現;**無硬依賴,standalone-first 不變**。
> **不做**:把錄音重新合進 mori-desktop(v0.8.0 #138/#139/#140 才刻意拆出,別走回頭路);tray 直接控制錄音(留作 follow-up C,需控制通道)。

## 1. 背景

- 現況:recorder 跟 mori-desktop **各有一個 tray icon**(recorder `src-tauri/src/main.rs:911 TrayIconBuilder` + `skip_taskbar(true)`;desktop `crates/mori-tauri/src/main.rs:6587+`)。兩個都裝就有兩個 tray 圖示 — 這是本 slice 要消的痛點。
- recorder 已用 `tauri-plugin-single-instance`(`main.rs:891`)→ 重複啟動會聚焦既有實例,不會開第二個。
- recorder 已寫 BodyManifest 到 `~/.mori/body-parts/mori.meeting-recorder/manifest.json`,內含 **`entrypoints.app` = recorder 的 `current_exe()` 絕對路徑**。desktop 要找 recorder 的 exe,讀這個就好,**不用猜安裝路徑**。
- mori-desktop 目前是 hub / reader,**沒有**為自己寫任何 presence marker → recorder 要偵測 desktop,需新增一個 marker(見 §3)。

## 2. 設計原則(採 yazelin 提案)

**雙向偵測 + 自適應 UI**:
- 不是「recorder 依賴 desktop」,也不是合體;是**兩邊各自看一眼對方在不在,在的話換一種 UI 呈現**。
- 偵測一律走 `~/.mori/` 的檔案 marker(跟既有 body-part 慣例一致,**不開 IPC**)。
- 任一邊單獨存在時行為完全不變(standalone-first 硬規矩)。

## 3. 偵測機制

| 方向 | 看什麼 | 現況 | 要做 |
|---|---|---|---|
| desktop → recorder | `~/.mori/body-parts/mori.meeting-recorder/manifest.json`(`entrypoints.app`)| 已存在 | 讀它判斷「recorder 已安裝」+ 取 exe 路徑 |
| recorder → desktop | desktop presence marker | 不存在 | desktop 啟動寫 `mori.desktop` marker(`pid` / `started_at` / `entrypoints.app`);結束盡量移除 |

**關鍵:installed vs running 要分清楚。**

- desktop → recorder:用「**installed**」(manifest 在)就夠 — 只是要不要顯示「會議錄音」入口。
- recorder → desktop 決定**藏不藏自己的 tray**:**必須用「running」**,不能只看「installed」。否則:desktop 裝了但沒開 → recorder 藏了 tray、desktop 也沒 tray → **你召喚不出 recorder(孤兒)**。所以 marker 要帶 `pid` + `started_at`,recorder 讀時驗 PID 還活著 / 時間夠新,才算 desktop running。

## 4. A — desktop 端(偵測到 recorder → 插入 UI + 啟動)

1. 啟動掃 `~/.mori/body-parts/*/manifest.json`(BI-1 已有 reader)。看到 `id == "mori.meeting-recorder"` → 標記 recorder 可用 + 記 `entrypoints.app`。
2. **tray 選單插一條「會議錄音」**(在既有 tray menu `main.rs:6587+`,用 `MenuItem::with_id(app, "launch_recorder", "會議錄音", …)`;`on_menu_event` 接 `"launch_recorder"`)。
3. 點擊 → spawn `entrypoints.app`,**帶 `--no-tray` flag**(Windows 用 `ShellExecuteExW` + `SEE_MASK_FLAG_NO_UI`,對齊 CLAUDE.md Windows quirk;Linux 走既有 `action_skills::platform` open helper)。
4. recorder 若已在跑 → single-instance 接手聚焦,不開第二個(已內建)。
5. recorder manifest 不在 → **這條完全不顯示**(自適應)。

## 5. B — recorder 端(偵測到 desktop → 藏自己的 tray)

兩個觸發(擇一或並用,見 Decision 1):
- **(a) 被帶 `--no-tray` 啟動**(desktop 啟它時帶):跳過 `TrayIconBuilder`(`main.rs:911` 包成 conditional)。最穩、可預測。
- **(b) 啟動時偵測 desktop running**(讀 §3 marker + 驗 PID 活著):即使你自己雙擊開的,只要 desktop 在跑,也藏 tray —— 這條才是 yazelin「雙向偵測」原則的完整實現。

**藏 tray 時的安全網(避免孤兒視窗)**:
- recorder 視窗本來 `skip_taskbar(true)` → 一旦藏 tray 又沒 taskbar 就「叫不出來」。所以藏 tray 時要嘛 (i) `skip_taskbar(false)` 回到工作列,要嘛 (ii) 啟動直接展開視窗(膠囊可見)。膠囊已有 ✕ 結束鈕(v0.1.1+),關閉 OK。
- 單獨跑(沒 desktop / 沒 flag)→ tray 照舊,零行為改變。

## 6. TDD / 驗證

- recorder:desktop-marker reader 純函式測試(marker 在 / 不在 / PID 已死 → running? true/false)。
- recorder:`--no-tray` flag 解析測試。
- desktop:scan body-parts 找到 recorder manifest → 解析 `entrypoints.app`(沿用 BI-1 reader,加 case)。
- **手測矩陣**:
  1. 只裝 recorder → 自己的 tray 在,行為不變。
  2. 裝兩個、desktop 沒開、雙擊 recorder → recorder tray 在(**沒孤兒**)。
  3. 裝兩個、desktop 開著、desktop tray 點「會議錄音」→ recorder 起、無第二個 tray、膠囊可操作。
  4. 裝兩個、desktop 開著、雙擊 recorder(走 (b))→ recorder 起、藏 tray、可從 desktop 或膠囊操作。

## 7. 不在範圍(本 slice 不做)

- **C — tray 直接控制錄音**(開始 / 停止):需控制通道(command 檔 / localhost port),留作 follow-up。
- desktop 動態偵測 recorder 啟停後即時改 menu(v1 啟動時掃一次)。
- recorder 動態偵測 desktop 啟停後即時長 / 收 tray(v1 啟動決定一次;dynamic re-show 列 later)。
- 把錄音合回 mori-desktop 同一 process。

## 8. 決定點(2026-06-04 yazelin 拍板 — 全採建議)

- [x] **1. recorder 藏 tray 的觸發 = (a)+(b)** — desktop 啟動時帶 `--no-tray`(最穩、可預測);recorder 自己被雙擊時也偵測 desktop running 而自適應藏 tray。
- [x] **2. desktop presence marker = `~/.mori/body-parts/mori.desktop/manifest.json`** — 把 hub 也當一個註冊條目,跟既有 body-parts reader 共用。marker 帶 `pid` / `started_at`(running 判定見 §3)。
- [x] **3. 藏 tray 時 = 啟動就展開膠囊**(不靠工作列)。膠囊 ✕ 結束鈕已存在(v0.1.1+)。
- [x] **4. desktop 入口 = tray + Body 分頁啟動鈕都給** — tray 快,Body tab 是正式的「身體部件」面。

## 9. repo touch points

- **mori-desktop**:`crates/mori-tauri/src/main.rs`(tray menu + `on_menu_event` 加 `launch_recorder`;啟動寫 hub marker)、BI-1 body-parts reader(加「找 recorder」)、(可選)`BodyTab.tsx` 加啟動鈕。
- **mori-meeting-recorder**:`src-tauri/src/main.rs`(`--no-tray` 解析 + conditional `TrayIconBuilder` + 藏 tray 時的 `skip_taskbar` / 展開)、新增 desktop-marker reader 模組。
- **文件**:本 plan;完成後回填 `docs/body-interface-backlog.md` §109 BI-5 phase 2 + 「Next action」。

---

**Owner**: yazelin　**狀態**:§8 已鎖定(全採建議);待 yazelin 給 go + 確認時機後動工。動 mori-desktop Rust 會中斷你正在用的語音,**動工前先確認時機**。
