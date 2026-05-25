# Mori 自我開發 E3（Stabilization）

目標：定義 release gate 與失敗回復。

- release gate 最低要求：build pass + core tests pass
- 若 fail：記錄原因、停在人工審核、不得自動 merge
- 回復流程：修復依賴 / 重跑 verify / 更新 gate 摘要


## Recovery checklist

1. 先確認 preflight 缺失依賴
2. 補齊依賴後按 Recheck deps
3. 先跑 quick verify，再視情況跑 full verify
4. 更新 gate 摘要與 review note
5. 若仍失敗，停在人工審核，不進 merge 候選


## Release gate (minimum)

- build == pass
- core == pass
- deps != fail

若任一條件不成立：
- 標記為 `not-ready`
- 僅允許產出審核摘要，不可進入 merge 候選
