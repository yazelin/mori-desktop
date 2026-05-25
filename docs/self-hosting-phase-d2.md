# Mori 自我開發 Phase D2 執行清單（跨平台前置檢查）

D2 目標：把 D1 的 Linux/Ubuntu preflight 擴成跨平台可理解的入口，不讓 Windows/macOS 使用者在第一步卡住。

## 目標

1. Self-Dev preflight 顯示目前作業系統。
2. 依平台顯示對應 setup 指引：
   - Linux: `sudo bash scripts/install-linux-deps.sh`
   - Windows/macOS: 顯示手動 setup 提示（先不自動安裝）
3. 建立跨平台驗證矩陣，明確列出「可驗證」與「待補」。

## 驗證矩陣（D2）

- Ubuntu: `sudo bash scripts/install-linux-deps.sh` + `npm ci` + `bash scripts/verify.sh`
- Windows: 先確認 build 可完成（手動安裝依賴，暫不提供自動安裝腳本）
- macOS: 先確認 build 可完成（手動安裝依賴，暫不提供自動安裝腳本）

## DoD

1. Self-Dev preflight 可顯示 OS 與對應指引。
2. Linux 使用者可一鍵複製安裝指令。
3. 文件清楚說明 Windows/macOS 目前是手動 setup 路徑。
