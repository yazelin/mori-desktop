# Mori 自我開發 Phase D5 執行清單（審核交付優化）

D5 目標：把 D4 的 gate 資訊變成可直接交付給 PR 審核者的摘要內容。

## 目標

1. Self-Dev UI 可一鍵複製 gate 摘要。
2. 摘要內容可直接貼到 PR description。
3. 保留 human-in-the-loop：摘要只做輔助判斷。

## DoD

1. UI 有「複製 gate 摘要」按鈕。
2. 複製內容至少包含 deps/build/core 三項狀態。
3. 文檔中明確 gate 非自動 merge 判斷器。
