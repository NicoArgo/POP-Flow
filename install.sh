#!/usr/bin/env bash
# Build and install POP Flow (thumbnail Alt-Tab) over the system cosmic-launcher.
# Reversible with ./uninstall.sh
set -euo pipefail
cd "$(dirname "$0")"

echo "==> Building (cargo build --release)..."
cargo build --release

BIN="target/release/cosmic-launcher"
[ -f "$BIN" ] || { echo "Build failed: $BIN not found"; exit 1; }

if [ ! -f cosmic-launcher.orig ] && [ -f /usr/bin/cosmic-launcher ]; then
    echo "==> Backing up current /usr/bin/cosmic-launcher -> ./cosmic-launcher.orig"
    cp /usr/bin/cosmic-launcher cosmic-launcher.orig
fi

echo "==> Installing to /usr/bin/cosmic-launcher (needs sudo)..."
sudo install -m 0755 "$BIN" /usr/bin/cosmic-launcher

pkill -x cosmic-launcher 2>/dev/null || true
echo "==> Done. Open a few windows and press Alt+Tab."
