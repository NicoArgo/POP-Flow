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

# Keep the auto-reapply golden copy in sync, or say out loud that this install
# is temporary — silence here used to hide the fact that a package update wipes
# the feature.
GOLDEN=/usr/local/lib/pop-flow/cosmic-launcher
if [ -f "$GOLDEN" ]; then
    echo "==> Refreshing auto-reapply golden copy"
    sudo install -m 0755 "$BIN" "$GOLDEN"
else
    echo "!! No auto-reapply hook installed: the next package update of"
    echo "   cosmic-launcher will silently restore the stock binary."
    echo "   Run ./setup-auto-reapply.sh to make this install stick."
fi

pkill -x cosmic-launcher 2>/dev/null || true
echo "==> Done. Open a few windows and press Alt+Tab."
