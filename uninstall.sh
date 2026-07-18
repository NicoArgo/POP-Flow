#!/usr/bin/env bash
# Restore the original system cosmic-launcher (undo install.sh).
set -euo pipefail
cd "$(dirname "$0")"

[ -f cosmic-launcher.orig ] || { echo "No backup (cosmic-launcher.orig) found."; exit 1; }

# Turn off auto-reapply first, or the next package op would re-patch the binary
# right after we restore the original.
if [ -f /etc/apt/apt.conf.d/99-pop-flow-cosmic-launcher ]; then
    echo "==> Removing auto-reapply hook (needs sudo)..."
    sudo rm -f /etc/apt/apt.conf.d/99-pop-flow-cosmic-launcher \
               /usr/local/lib/pop-flow/reapply-cosmic-launcher \
               /usr/local/lib/pop-flow/cosmic-launcher
    sudo rmdir --ignore-fail-on-non-empty /usr/local/lib/pop-flow 2>/dev/null || true
fi

echo "==> Restoring original /usr/bin/cosmic-launcher (needs sudo)..."
sudo install -m 0755 cosmic-launcher.orig /usr/bin/cosmic-launcher

pkill -x cosmic-launcher 2>/dev/null || true
echo "==> Restored."
