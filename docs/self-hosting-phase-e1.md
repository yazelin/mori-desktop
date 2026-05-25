# Mori 自我開發 E1（Stabilization）

目標：讓 gate status 來自更真實信號，而不是純 UI 推測。

- quick verify (`cargo check -p mori-core`) => build signal
- full verify (`bash scripts/verify.sh`) => build + core tests signal
- 無 verify => build/core 皆 unknown
