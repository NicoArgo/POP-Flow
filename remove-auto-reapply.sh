#!/usr/bin/env bash
# Undo setup-auto-reapply.sh: remove the APT hook and golden copy. Needs sudo.
# This does NOT touch /usr/bin/cosmic-launcher — use ./uninstall.sh to restore
# the original launcher.
set -euo pipefail

echo "==> Removing POP Flow auto-reapply hook (needs sudo)..."
sudo rm -f /etc/apt/apt.conf.d/99-pop-flow-cosmic-launcher \
           /usr/local/lib/pop-flow/reapply-cosmic-launcher \
           /usr/local/lib/pop-flow/cosmic-launcher
sudo rmdir --ignore-fail-on-non-empty /usr/local/lib/pop-flow 2>/dev/null || true
echo "==> Auto-reapply removed."
