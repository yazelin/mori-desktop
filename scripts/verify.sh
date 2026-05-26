#!/usr/bin/env bash
# Shared verification entrypoint for local development, cloud agents, and CI.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

run() {
    echo ""
    echo "==> $*"
    "$@"
}

run npm run build
run npm test
run cargo test -p mori-core --lib
run cargo test -p mori-time --lib
run cargo test -p mori-file-loader --lib --tests
run cargo test -p mori-tauri --bin mori-tauri
run cargo check --workspace --all-targets

if [ "${VERIFY_STRICT:-0}" = "1" ]; then
    run cargo fmt --check
    run cargo clippy --workspace --all-targets -- -D warnings
else
    echo ""
    echo "==> Skipping strict Rust style checks. Run with VERIFY_STRICT=1 to include fmt and clippy."
fi
