# Mori 自我開發 Phase D4 執行清單（Release Gate 自動化）

D4 目標：把前置檢查與平台驗證結果，收斂成可審核的 release gate 摘要。

## 目標

1. 在 Self-Dev Report 顯示 gate checklist 狀態。
2. 可複製 gate 摘要貼到 PR description。
3. 保留 human-in-the-loop，僅提供判斷資訊。

## Gate Checklist（初版）

- Preflight deps: pass / fail
- Build: pass / fail / unknown
- Core tests: pass / fail / unknown

## DoD

1. UI 可見 gate checklist。
2. 文案明確標示「僅供審核，不自動 merge」。
3. PR 說明可附上 gate 摘要。
