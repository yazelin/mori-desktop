# Self-Dev Beta Go/No-Go Checklist

本文件是 Self-Dev Beta 上線前的本機驗證 runbook。主線設計見
[`docs/self-hosting-bootstrap.md`](self-hosting-bootstrap.md)，E2/E3 詳細流程見
[`docs/self-hosting-phase-e2.md`](self-hosting-phase-e2.md) 與
[`docs/self-hosting-phase-e3.md`](self-hosting-phase-e3.md)。

## 1. Core 單元測試（必過）

```bash
cargo test -p mori-core --lib
```

Go：`0 failed`。
No-Go：任何 `dev_orchestrator` lifecycle 測試失敗。

## 2. E2 回歸 smoke（必過）

```bash
cargo test -p mori-core --lib e2_regression_flow_lifecycle_smoke
```

Go：通過。
No-Go：start/list/snapshot/rerun/abort/delete 任一路徑失敗。

## 3. Workspace 編譯檢查（必過）

```bash
cargo check --workspace --all-targets
```

Go：成功。
No-Go：任何 crate compile fail。

## 4. Frontend build（必過）

```bash
npm run build
```

Go：TypeScript + Vite 完成。
No-Go：TS error 或 build fail。chunk warning 可先接受。

## 5. Linux 依賴 preflight（Linux 必做）

```bash
sudo bash scripts/install-linux-deps.sh
bash scripts/verify.sh
```

Go：verify 完成。
No-Go：缺 `glib-2.0` 等基礎依賴仍未解。

## 6. Self-Dev UI 手動流程（必做）

- enable Codex execution
- start task
- list tasks
- open report
- confirm executor command / Codex output
- confirm changed files / Git diff
- rerun
- abort
- delete

Go：流程可完整走通。
No-Go：任一 IPC 或按鈕不一致，或 Codex 任務沒有產出可審核 diff。

## 7. Gate 一致性檢查（必做）

- 無 verify：build/core = `unknown`
- quick verify：build pass/fail，core `unknown`
- full verify：build/core 同步 pass/fail

Go：UI 與規則一致。
No-Go：gate 與 `verify_command` 結果不一致。

## 8. 審核交付輸出（必做）

在 UI 測試以下複製功能：

- Copy gate summary
- Copy PR-ready summary
- Copy review note template
- Copy regression checklist
- Copy recovery checklist
- Copy release gate status

Go：全部可貼到 PR 或 review 使用。
No-Go：任何關鍵輸出缺失或格式不可讀。

## 9. 安全邊界檢查（必做）

- Codex 只能在隔離 workspace copy 內修改
- 不自動 commit / push / merge
- 失敗時停在 human review，狀態為 not-ready

Go：human-in-the-loop 完整保留。
No-Go：出現自動高權限動作。

## 10. Beta 發布決策（最終）

Go for Beta（內測）條件：

- 1 到 9 全綠，或只有可接受 warning
- 有已知限制清單：Windows/macOS 目前手動 setup
- 有回復流程：E3 checklist

No-Go for GA：

- 尚未把 E2/E3 完整接入 CI release gate。
