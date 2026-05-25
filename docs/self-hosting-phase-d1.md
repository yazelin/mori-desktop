# Mori 自我開發 Phase D1 執行清單（Linux / Ubuntu）

本階段目標：把既有 Self-Dev 功能從「可用」推進到「可穩定部署」，先鎖定 Linux / Ubuntu。

## 目標

1. 使用者可在 Self-Dev UI 一鍵檢查環境是否可執行。
2. 若依賴缺失，提供可直接複製的修復指令。
3. 在 Ubuntu runner 上可通過標準驗證流程。

## D1 範圍

- 使用既有依賴檢查 IPC（`deps_list`）作為 Self-Dev preflight 資料來源。
- Self-Dev 入口在開始任務前提示缺失項目，降低中途失敗率。
- 文件化 Linux / Ubuntu 最小驗證流程。

## Linux / Ubuntu 驗證流程

```bash
bash scripts/install-linux-deps.sh
npm ci
bash scripts/verify.sh
```

## 驗收標準（DoD）

1. 在 Ubuntu 可完成上述三條指令且 `verify.sh` 成功。
2. Self-Dev 使用流程中可看到依賴檢查結果與缺失提醒。
3. 文檔清楚說明「若檢查失敗，先修依賴再啟動任務」。

## 已知限制

- D1 只涵蓋 Linux / Ubuntu；Windows 與 macOS 需在 D2/D3 追加驗證矩陣。
- 依賴檢查結果受本機環境差異影響，UI 不應硬阻擋，只做風險提示。
