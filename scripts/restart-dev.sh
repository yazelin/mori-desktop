#!/usr/bin/env bash
# Force-kill 任何 mori dev process,然後重啟 `npm run tauri dev`。
#
# 何時用:
# - webview state 髒(theme HMR 沒套到 / 殘留 visible 視窗 / event listener 舊版)
# - 改了 src/ChatBubble.tsx / Picker.tsx / main.tsx 等需要 webview 完整 remount
# - 改了 Rust code 後 cargo watch 沒跑或卡住
# - dock 出現多個 Mori icon 堆疊
#
# 用法:
#   bash scripts/restart-dev.sh
# 或 npm script:
#   npm run dev:restart

set -e
cd "$(dirname "$0")/.."

PATTERNS=(
  "mori-tauri"
  "tauri dev"
  "cargo watch"
  "node .*vite"
)

echo "→ 找現有 mori dev process..."
ANY_FOUND=0
for p in "${PATTERNS[@]}"; do
  if pgrep -f "$p" > /dev/null 2>&1; then
    echo "   匹配: $p"
    pgrep -af "$p" | grep -v "$0" || true
    ANY_FOUND=1
  fi
done

if [ "$ANY_FOUND" -eq 0 ]; then
  echo "   (沒有 dev process 在跑)"
else
  echo ""
  echo "→ SIGTERM..."
  for p in "${PATTERNS[@]}"; do
    pkill -f "$p" 2>/dev/null || true
  done

  # 等 2 秒看是否乾淨
  sleep 2

  # 還活著的 SIGKILL
  STILL_ALIVE=0
  for p in "${PATTERNS[@]}"; do
    if pgrep -f "$p" > /dev/null 2>&1; then
      STILL_ALIVE=1
      break
    fi
  done

  if [ "$STILL_ALIVE" -eq 1 ]; then
    echo "→ 還活著,SIGKILL..."
    for p in "${PATTERNS[@]}"; do
      pkill -9 -f "$p" 2>/dev/null || true
    done
    sleep 1
  fi

  # 最後確認
  for p in "${PATTERNS[@]}"; do
    if pgrep -f "$p" > /dev/null 2>&1; then
      echo "⚠ 仍有 process 抓不下($p)— 自己 kill 看看:"
      pgrep -af "$p" || true
      exit 1
    fi
  done
  echo "✓ 全部清乾淨"
fi

echo ""
echo "→ 啟動 npm run tauri dev..."
exec npm run tauri dev
