#!/usr/bin/env bash
# Restore the original system cosmic-launcher (undo install.sh).
set -euo pipefail
cd "$(dirname "$0")"

[ -f cosmic-launcher.orig ] || { echo "No backup (cosmic-launcher.orig) found."; exit 1; }

echo "==> Restoring original /usr/bin/cosmic-launcher (needs sudo)..."
sudo install -m 0755 cosmic-launcher.orig /usr/bin/cosmic-launcher

pkill -x cosmic-launcher 2>/dev/null || true
echo "==> Restored."
