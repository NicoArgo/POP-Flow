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

# Keep the auto-reapply golden copy in sync when a rebuild is installed.
if [ -f /usr/local/lib/pop-flow/cosmic-launcher ]; then
    echo "==> Refreshing auto-reapply golden copy"
    sudo install -m 0755 "$BIN" /usr/local/lib/pop-flow/cosmic-launcher
fi

pkill -x cosmic-launcher 2>/dev/null || true
echo "==> Done. Open a few windows and press Alt+Tab."
