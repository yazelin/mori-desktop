---
description: Build the Mori desktop release, prompt user to install + run targeted tests, then read JSONL log to verify the changes actually work. Invoke when the user says "ship and verify" / "build + test" / "驗證一下" after a code change, especially Windows release-only behavior. Reads `~/.mori/logs/mori-YYYY-MM-DD.jsonl` to cross-check user-reported behavior against event_log truth.
---

# ship-and-verify — 邊裝邊驗,log 對照測項

當你剛改完一條(或一批)會影響 release-only behavior 的 code,跑這條 skill 把「build → 提醒安裝 → 等 user 測 → 讀 log 對照」全程走一輪。

## When to invoke

- 改完 CREATE_NO_WINDOW / spawn / installer / release flow 相關的 code
- 改完某個 only-in-release 才會踩到的 bug(`windows_subsystem = "windows"` GUI subsystem 限定的、MSI 裝完才出現的、tray quit 才觸發的 cleanup)
- User 說「build 看看 / 裝起來測 / verify changes」

不適用:
- dev 模式即可重現的 bug → `npm run tauri dev` 跑就好,不必走完整 release
- 純文件 / 純 frontend hot-reload 變動 → `npm run dev` 看 vite output 即可

## Steps

### 1. Build

```bash
cd <repo>
npm run tauri build
```

**用 background mode**(`run_in_background=true`)— Tauri build Windows 完整 cycle 1-3 分鐘,不該卡 conversation。CI release.yml 在 GitHub 也跑同樣 chain。

跑前提醒 user:**完全結束 Mori**(tray icon 右鍵 Quit,不只關視窗 — 要關 tray 才能釋放鎖檔)。沒關 NSIS 會跳「無法刪除某檔」。

### 2. 列出 install path + 該測什麼

跨平台 installer 位置(都在 `target/release/bundle/` 下):

| 平台 | Output 路徑 | 建議 |
|---|---|---|
| Windows | `target/release/bundle/nsis/Mori_<version>_x64-setup.exe` | 推薦 NSIS(快、不要 admin) |
| Windows | `target/release/bundle/msi/Mori_<version>_x64_en-US.msi` | 備案 |
| Linux | `target/release/bundle/deb/mori_<version>_amd64.deb` | Ubuntu / Debian |
| Linux | `target/release/bundle/appimage/mori_<version>_amd64.AppImage` | 其他發行版 |
| macOS | `target/release/bundle/dmg/Mori_<version>_x64.dmg` | (殼還沒接,先 N/A) |

明確列出**這次該測的 user-facing scenario**(不是 unit test 層 — 那 cargo test 跑掉了),示例:

```
裝完開 Mori → 試:
1. <跟 code change 直接相關的 path>
2. <相鄰 regression risk 的 path>
3. <log 該看到什麼新 event>
```

每條都要可驗證 — 跟 log JSONL 對得起來最好。

### 3. 等 user 回報

User 跑完會說「裝完了」/「測完了」/「測項都 ok」/「踩到 X」。**別自己猜測通過** —— release-only behavior 有些只有實機才看得到(黑框、CFFI dialog、MSI bundling),log 不一定能驗。

### 4. 讀 log 對照

跨平台 log 路徑:`~/.mori/logs/mori-YYYY-MM-DD.jsonl`(`~` 在 Git Bash for Windows 跟 Linux bash 都解到 `$HOME` / `$USERPROFILE`)。

```bash
wc -l ~/.mori/logs/mori-$(date +%Y-%m-%d).jsonl
```

拿到行數,**只讀新增段**(別重讀已知的 lines 浪費 context)。比對思路:

| user 報的測項 | log 該有的 event | log 該**沒**的 event |
|---|---|---|
| Hey Mori 觸發過 | `wake_word_event` | `wake_listener_error` |
| codex agent 跑 | `llm_call binary=codex ok=true` | `llm_call ok=false ... Not inside a trusted directory` |
| open_url 真的開了 | `skill_dispatch skill=open_url ok=true` | (若只有 `agent_completed.response` 寫「已開啟」但無 `skill_dispatch` → LLM 幻覺) |
| DepsTab 重檢過 | (沒對應 event,只能憑 user 回報 UI 觀察) | — |
| Mori 結束乾淨 | (沒對應 event;Python-CFFI dialog 是 UI 層) | — |

**新欄位看一下也順手:** `llm_call.stdin_tail_preview` / `response_preview` / `system_prompt_chars` —— 確認 LLM 拿到的 context 對、回的內容對。

### 5. 報告

跑表格給 user 看哪些測項從 log 驗到 ✅、哪些只能憑 user 觀察 ✓、哪些沒過 ✗。風格範例:

```
| 測項 | 結果 | log 證據 |
|---|---|---|
| codex agent | ✅ | line 354 / 357 `llm_call binary=codex ok=true` |
| open_url 真有開 | ✅ | line 298 `skill_dispatch skill=open_url ok=true` 跟 agent response 對齊 |
| 黑框消失 | ✓ user 觀察 OK | log 不可見 |
| ... | ... | ... |
```

對不上 / fail 的 → 把該段 JSONL 抓出來貼 + 解讀。

## Cross-platform

- `~/.mori/logs/` 路徑跨平台同邏輯。Git Bash on Windows 解 `~` → `C:\Users\<user>`,Linux 解到 `$HOME`。
- `date +%Y-%m-%d` 跨平台 bash 都認。
- Tauri output bundle 路徑跨平台 — `target/release/bundle/<format>/`。
- 提示 user 結束 Mori 的方式跟平台無關(tray icon 右鍵 Quit)。

## 失敗 recovery

- **`npm run tauri build` 卡 / fail** — 看 `target/release/build-*.log` 跟 Tauri 自家 log。常見問題:沒網路抓 webview2 / Linux 缺 deps(`scripts/install-linux-deps.sh`)/ cargo cache 壞(`cargo clean -p mori-tauri`)。
- **User 說「裝不上」** — `tasklist | grep -i mori`(Windows) / `pgrep -fa mori`(Linux)看有沒有舊 process 鎖檔。
- **Log 對不上 user 講的** — 確認你看對日期的 JSONL(timestamp 是 UTC,user 時區可能差幾小時)。
