#!/usr/bin/env bash
# Restore the original system cosmic-launcher (undo install.sh).
set -euo pipefail
cd "$(dirname "$0")"

[ -f cosmic-launcher.orig ] || { echo "No backup (cosmic-launcher.orig) found."; exit 1; }

# The backup was taken whenever this fork was first installed, and apt has moved
# since. Restoring a backup the package no longer ships would be a silent
# downgrade, so check it against the checksum dpkg recorded for the package.
stale_backup() { # <package> <binary> <backup>
    local want got
    want="$(awk -v f="usr/bin/$2" '$2 == f {print $1}' "/var/lib/dpkg/info/$1.md5sums" 2>/dev/null)"
    got="$(md5sum < "$3" | cut -d' ' -f1)"
    [ -n "$want" ] && [ "$want" != "$got" ]
}
if stale_backup cosmic-launcher cosmic-launcher cosmic-launcher.orig; then
    echo
    echo "!! cosmic-launcher.orig is not the binary the installed cosmic-launcher package"
    echo "   ships: apt has updated it since the backup was taken."
    echo
    echo "   Restoring it would be a downgrade. Let apt restore its own binary"
    echo "   instead, which is the real original:"
    echo "       ./remove-auto-reapply.sh"
    echo "       sudo apt install --reinstall cosmic-launcher"
    echo
    echo "   To restore the backup anyway, knowing the above:"
    echo "       POP_FLOW_ALLOW_STALE_BACKUP=1 ./uninstall.sh"
    echo
    [ "${POP_FLOW_ALLOW_STALE_BACKUP:-0}" = "1" ] || exit 1
fi

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
