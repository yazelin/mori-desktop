# Mori 自我開發 Phase D3 執行清單（平台驗證收斂）

D3 目標：讓 Self-Dev preflight 從「看得懂」走到「可驗證與可收斂」，建立 release 前的跨平台最低門檻。

## 目標

1. 補齊 Windows/macOS 的手動驗證 checklist。
2. 在 preflight 區塊明確提供下一步動作。
3. 將 Linux/Windows compile 與 core tests 納入 release gate。

## 建議驗證矩陣

- Linux (Ubuntu)
  - `bash scripts/install-linux-deps.sh`
  - `npm ci`
  - `bash scripts/verify.sh`
- Windows
  - `npm ci`
  - `npm run build`
  - `cargo check --workspace --all-targets`
- macOS
  - `npm ci`
  - `npm run build`
  - `cargo check --workspace --all-targets`

## DoD

1. 文件可直接作為平台驗證 runbook。
2. Self-Dev preflight 顯示具體下一步。
3. PR 說明需附上實際執行過的平台檢查命令。
