#!/usr/bin/env bash
# Undo setup-auto-reapply.sh: remove the APT hook and golden copy. Needs sudo.
# This does NOT touch /usr/bin/cosmic-launcher — use ./uninstall.sh to restore
# the original binary.
set -euo pipefail

COMP=cosmic-launcher

LIBDIR=/usr/local/lib/pop-flow

# Nothing installed is a normal state — uninstall.sh calls this blind.
if [ ! -e "/etc/apt/apt.conf.d/99-pop-flow-$COMP" ] && [ ! -e "$LIBDIR/$COMP" ]; then
    exit 0
fi

echo "==> Removing POP Flow auto-reapply hook for $COMP (needs sudo)..."
sudo rm -f "/etc/apt/apt.conf.d/99-pop-flow-$COMP" \
           "$LIBDIR/reapply-$COMP" \
           "$LIBDIR/$COMP"
# Only removes the shared dir once the last component has been removed from it.
sudo rmdir --ignore-fail-on-non-empty "$LIBDIR" 2>/dev/null || true
echo "==> Auto-reapply removed for $COMP."
