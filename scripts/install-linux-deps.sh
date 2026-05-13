#!/bin/bash
# Install Linux system libraries needed to build mori-desktop (Tauri 2).
#
# Tauri 2 wraps a system WebView (WebKitGTK on Linux) — these packages
# provide the headers/libraries it links against. Without them, `cargo
# build` fails inside any Tauri crate with cryptic linker errors.
#
# 同步來源:yazelin/ubuntu-26.04-setup/scripts/setup-tauri-deps.sh。在這
# repo 自帶一份是為了:
# - 新人 git clone mori-desktop 一個 repo 就完整,不用再去找另一個 repo
# - CI 不用 checkout 外部 repo,build/PR 重現性高
# - 改 deps 跟 mori-desktop 程式碼進同一個 commit,bisect 友善
#
# ─── Usage ───────────────────────────────────────────────────────────
#
#   sudo bash scripts/install-linux-deps.sh             # install
#   sudo bash scripts/install-linux-deps.sh --uninstall # remove
#
# ─── What gets installed ─────────────────────────────────────────────
#
#   libwebkit2gtk-4.1-dev          WebView (WebKitGTK 4.1, current standard)
#   libssl-dev                     TLS for HTTPS calls
#   libayatana-appindicator3-dev   System tray icon support
#   librsvg2-dev                   SVG rendering for icons
#   libsoup-3.0-dev                HTTP client used by WebKitGTK
#   libjavascriptcoregtk-4.1-dev   JS runtime headers
#   libasound2-dev                 ALSA headers (cpal — Mori 麥克風用)
#   pkg-config build-essential curl wget file   build glue
#
# Reference: https://v2.tauri.app/start/prerequisites/#linux

set -e

for arg in "$@"; do
    if [ "$arg" = "-h" ] || [ "$arg" = "--help" ]; then
        sed -n '/^#!/,/^set -e$/p' "$0" \
            | sed -e '1d' -e '/^set -e$/,$d' -e 's/^# \{0,1\}//'
        exit 0
    fi
done

if [ "$EUID" -ne 0 ]; then
    echo "Please run as: sudo bash $0 [flags]"
    exit 1
fi

ACTION="install"
for arg in "$@"; do
    case "$arg" in
        --uninstall) ACTION="uninstall" ;;
    esac
done

PACKAGES=(
    libwebkit2gtk-4.1-dev
    libssl-dev
    libayatana-appindicator3-dev
    librsvg2-dev
    libsoup-3.0-dev
    libjavascriptcoregtk-4.1-dev
    # ALSA headers — needed by cpal (mic capture in mori-tauri)
    libasound2-dev
    pkg-config
    build-essential
    curl
    wget
    file
)

if [ "$ACTION" = "uninstall" ]; then
    echo "==> Removing Tauri build dependencies"
    DEBIAN_FRONTEND=noninteractive apt-get remove -y "${PACKAGES[@]}" 2>/dev/null || true
    DEBIAN_FRONTEND=noninteractive apt-get autoremove -y
    echo "Done. Note: shared system libs that other apps depend on were not aggressively removed."
    exit 0
fi

echo "==> 1/2  apt update"
DEBIAN_FRONTEND=noninteractive apt-get update

echo "==> 2/2  Installing Tauri build dependencies"
DEBIAN_FRONTEND=noninteractive apt-get install -y "${PACKAGES[@]}"

echo ""
echo "Done. Now you can build mori-desktop:"
echo ""
echo "    npm install"
echo "    npm run build"
echo "    npm run tauri dev"
