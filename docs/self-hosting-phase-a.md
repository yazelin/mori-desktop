# Mori 自我開發 Phase A 執行清單 (下一步)

本文件是 `docs/self-hosting-bootstrap.md` 的「立即可執行版」，聚焦 **Phase A (1~2 週)**。

## 目標

在不影響既有聊天/語音流程下，完成最小可用的「自我開發任務骨架」：
- 能建立 `DevTask` 並追蹤狀態
- 能在隔離 workspace 生成 patch（先不 commit / 不 push）
- 能輸出可審閱 `DevReport`

## 範圍 (In Scope)

1. `mori-core` 新增型別與流程骨架
   - `DevTask` / `DevPlan` / `DevAction` / `DevReport`
   - `DevTaskStatus`: `Queued | Planning | Executing | Succeeded | Failed | Aborted`
2. `mori-tauri` 新增最小 IPC
   - `start_dev_task`（建立任務）
   - `get_dev_report`（查詢結果）
   - `abort_dev_task`（中止）
3. 前端新增隱藏式實驗入口（feature flag）
   - 任務輸入框
   - 任務狀態
   - 報告摘要（不顯示完整 diff 也可）

## 不做 (Out of Scope)

- 不做自動 `git commit`
- 不做 PR 建立
- 不做多輪自動修復
- 不做雲端同步/跨裝置調度

## 驗收標準 (Phase A DoD)

1. 可從 UI 送出一個自我開發任務。
2. 任務可進入 `Executing` 並在隔離目錄產生檔案變更。
3. 任務完成時可取得 `DevReport`（含摘要、修改檔案列表、錯誤訊息）。
4. 中止任務可在 3 秒內停止子程序並回到 `Aborted`。
5. 不得修改 `main` 分支工作樹。

## 工作分解 (建議順序)

### A1. Core 資料模型
- 檔案建議：`crates/mori-core/src/dev_orchestrator/mod.rs`
- 定義 Phase A 所需 enum/struct
- 先以 in-memory store 保存任務狀態

### A2. 隔離工作區執行器
- 檔案建議：`crates/mori-core/src/dev_orchestrator/executor.rs`
- 每個任務建立臨時資料夾（或 `git worktree`）
- 僅允許白名單路徑：`src/`, `crates/`, `docs/`

### A3. Tauri IPC 接線
- 檔案建議：`crates/mori-tauri/src/main.rs`
- 曝露 start/get/abort 三個 command
- 回傳一致的錯誤型別給前端

### A4. Frontend 實驗面板
- 檔案建議：`src/tabs/` 下新增實驗頁籤
- 透過 feature flag 啟用，預設關閉
- 顯示 task id / status / report summary

### A5. 基本驗證
- `cargo check --workspace --all-targets`
- `cargo test -p mori-core --lib`
- 能手動走一次 start → executing → succeeded/failed

## 風險與緩解

- 風險：任務執行卡住子程序
  - 緩解：統一 child process registry + abort timeout
- 風險：誤改主工作樹
  - 緩解：執行前驗證 cwd 必須是隔離路徑
- 風險：provider 輸出格式不穩
  - 緩解：Phase A 先採「報告摘要」而非嚴格結構化 diff

## 建議指令

```bash
# 開發前
cargo check --workspace --all-targets

# 迭代後最小驗證
cargo test -p mori-core --lib
```

---

若 Phase A 完成，下一步直接接 `docs/self-hosting-bootstrap.md` 的 Phase B：
加入 verify profile、commit 與 PR draft。
