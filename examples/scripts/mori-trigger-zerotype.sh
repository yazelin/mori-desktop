#!/bin/bash
# Helper script for examples/agent/AGENT-03.ZeroType Agent.md profile.
#
# 把 Mori 優化過的 prompt 餵給 Chrome 內的 ZeroType Agent extension。
# 流程:寫剪貼簿 → Ctrl+Shift+Period 開啟 ZeroType Agent 對話框 → Ctrl+V 貼上
#
# 使用前提:
#   - Chrome / Chromium 已開啟,且 focus 在要操作的網頁
#   - ZeroType Agent extension 已安裝
#   - extension 預設啟動快捷鍵 Ctrl+Shift+Period(若你改過 hotkey,改下面 ydotool key)
#   - 系統有 xclip + ydotool(setup-tauri-deps.sh / setup-wayland-input.sh cover)
#
# 安裝:
#   cp examples/scripts/mori-trigger-zerotype.sh ~/bin/
#   chmod +x ~/bin/mori-trigger-zerotype.sh
#
# 為何不用 wl-copy:
#   wl-copy 在 GNOME Wayland 有 clipboard portal 通知要 user 確認,
#   每次 trigger 都跳通知 — 體驗差。xclip 走 X11 selection(XWayland 自動橋接),
#   Chrome 讀剪貼簿時拿得到,且沒通知。
#
# 為何 sleep 1.2:
#   Ctrl+Shift+Period 後 ZeroType dialog 開啟 → 拿 input focus 有 jitter,
#   0.5s 偶爾不夠導致 Ctrl+V 送出去被 swallow → 1.2s 較穩。

PROMPT="$1"
echo -n "$PROMPT" | xclip -selection clipboard
ydotool key 29:1 42:1 52:1 52:0 42:0 29:0    # Ctrl+Shift+Period 開 ZeroType
sleep 1.2                                     # 等 dialog 拿 input focus
ydotool key 29:1 47:1 47:0 29:0              # Ctrl+V 貼上
