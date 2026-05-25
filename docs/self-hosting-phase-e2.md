# Mori 自我開發 E2（Stabilization）

目標：補 IPC + UI flow 的回歸測試。

- start_dev_task → list_dev_tasks → get_dev_task_snapshot
- rerun_dev_task / abort_dev_task / delete_dev_task
- preflight panel + gate panel 顯示一致性


## Regression flow checklist

1. `start_dev_task` 建立任務
2. `list_dev_tasks` 可見新任務
3. `get_dev_task_snapshot` 可讀 report
4. `rerun_dev_task` 產生新 task
5. `abort_dev_task` 可中止執行中任務
6. `delete_dev_task` 可清理單任務
