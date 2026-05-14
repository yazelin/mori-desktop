#!/usr/bin/env bash
# 在 Linux 登入時自動啟 mori-desktop 的 XDG autostart entry。
# (Mac / Windows 之後加。)
#
# Linux 機制:
#   ~/.config/autostart/<id>.desktop 是 freedesktop 標準,GNOME / KDE / XFCE
#   / Hyprland 等 session 啟動時掃這個目錄、跑所有 Exec=。
#
# 為什麼 mori-desktop 要 autostart:
#   1. mori-desktop 啟動會 spawn annuli admin server(D-1),annuli 內建
#      APScheduler 才能準時跑 explore / learn / study / post
#   2. user 不用每次開機手動點 icon
#   3. 跨 device 一致 Mori 體驗:每台機器都是 login 就在
#
# 用法:
#   bash scripts/install-autostart.sh           # 安裝 release(預設,推薦)
#   bash scripts/install-autostart.sh --debug   # 安裝指向 target/debug 的版本(暫用,容量大啟動慢)
#   bash scripts/install-autostart.sh --remove  # 移除

set -euo pipefail
cd "$(dirname "$0")/.."

REPO="$(pwd)"
AUTOSTART_DIR="$HOME/.config/autostart"
DESKTOP_FILE="$AUTOSTART_DIR/mori-desktop.desktop"

MODE="${1:-release}"

if [[ "$MODE" == "--remove" ]]; then
    if [[ -f "$DESKTOP_FILE" ]]; then
        rm "$DESKTOP_FILE"
        echo "✓ removed $DESKTOP_FILE"
    else
        echo "(沒安裝過,nothing to do)"
    fi
    exit 0
fi

if [[ "$MODE" == "--debug" ]]; then
    BINARY="$REPO/target/debug/mori-tauri"
    LABEL="(debug build)"
else
    BINARY="$REPO/target/release/mori-tauri"
    LABEL="(release build)"
fi

if [[ ! -x "$BINARY" ]]; then
    echo "❌ binary 不存在: $BINARY"
    if [[ "$MODE" == "--debug" ]]; then
        echo "   先跑 \`cargo build -p mori-tauri\` 或 \`npm run tauri dev\` 產 debug build"
    else
        echo "   先跑 \`npm run tauri build\` 產 release build"
        echo "   或暫時用 debug build:bash scripts/install-autostart.sh --debug"
    fi
    exit 1
fi

ICON="$REPO/crates/mori-tauri/icons/128x128.png"
if [[ ! -f "$ICON" ]]; then
    # fallback:用 source PNG
    ICON="$REPO/public/logo.png"
fi

mkdir -p "$AUTOSTART_DIR"

cat > "$DESKTOP_FILE" <<EOF
[Desktop Entry]
Type=Application
Name=Mori
Comment=森林精靈 Mori — 個人 AI 管家(autostart at login)
Exec=$BINARY
Icon=$ICON
StartupNotify=false
Terminal=false
Categories=Utility;
X-GNOME-Autostart-enabled=true
X-GNOME-Autostart-Delay=3
EOF

echo "✓ installed $DESKTOP_FILE $LABEL"
echo "  Exec=$BINARY"
echo
echo "下次登入 Mori 會自動啟動。"
echo "立即測試:nohup \"$BINARY\" > /tmp/mori-autostart-test.log 2>&1 &"
echo
echo "移除:bash scripts/install-autostart.sh --remove"
