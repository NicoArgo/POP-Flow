#!/usr/bin/env bash
# Restore the original system cosmic-launcher (undo install.sh).
set -euo pipefail
cd "$(dirname "$0")"

[ -f cosmic-launcher.orig ] || { echo "No backup (cosmic-launcher.orig) found."; exit 1; }

# Turn off auto-reapply first, or the next package operation would re-patch the
# binary right after we restore the original. Delegating to the script that owns
# those paths rather than repeating them: this used to remove only the golden
# copy, leaving a root-owned APT hook behind for good.
if [ -x ./remove-auto-reapply.sh ]; then
    ./remove-auto-reapply.sh
fi

echo "==> Restoring original /usr/bin/cosmic-launcher (needs sudo)..."
sudo install -m 0755 cosmic-launcher.orig /usr/bin/cosmic-launcher

pkill -x cosmic-launcher 2>/dev/null || true
echo "==> Restored."
