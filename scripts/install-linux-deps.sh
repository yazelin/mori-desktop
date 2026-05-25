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
#   See scripts/linux-build-packages.txt. Keep that package list explicit so
#   Codex Cloud, CI, and humans can provision the same Ubuntu build image.
#
# Reference: https://v2.tauri.app/start/prerequisites/#linux

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PACKAGE_FILE="$SCRIPT_DIR/linux-build-packages.txt"

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

if [ ! -f "$PACKAGE_FILE" ]; then
    echo "Missing package list: $PACKAGE_FILE" >&2
    exit 1
fi

mapfile -t PACKAGES < <(grep -Ev '^[[:space:]]*(#|$)' "$PACKAGE_FILE")
if [ "${#PACKAGES[@]}" -eq 0 ]; then
    echo "No packages listed in $PACKAGE_FILE" >&2
    exit 1
fi

if [ "$ACTION" = "uninstall" ]; then
    echo "==> Removing Tauri build dependencies"
    DEBIAN_FRONTEND=noninteractive apt-get remove -y "${PACKAGES[@]}" 2>/dev/null || true
    DEBIAN_FRONTEND=noninteractive apt-get autoremove -y
    echo "Done. Note: shared system libs that other apps depend on were not aggressively removed."
    exit 0
fi

echo "==> 1/2  apt update"
DEBIAN_FRONTEND=noninteractive apt-get update

echo "==> 2/2  Installing Tauri build dependencies from $PACKAGE_FILE"
DEBIAN_FRONTEND=noninteractive apt-get install -y "${PACKAGES[@]}"

echo ""
echo "Done. Now you can build mori-desktop:"
echo ""
echo "    npm install"
echo "    npm run build"
echo "    npm run tauri dev"
