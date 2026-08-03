#!/usr/bin/env bash
# Make POP Flow's cosmic-launcher survive system/package updates.
#
# A package update overwrites /usr/bin/cosmic-launcher with the stock binary.
# This installs an APT/dpkg post-invoke hook that runs (as root, no password)
# after every package operation and reinstalls our build whenever the on-disk
# binary no longer matches our "golden" copy.
#
# One-time setup; needs sudo. Undo with ./remove-auto-reapply.sh
set -euo pipefail
cd "$(dirname "$0")"

# --- component-specific settings ------------------------------------------
COMP=cosmic-launcher                      # name under /usr/bin
BUILT="target/release/cosmic-launcher"    # our build
RELOAD='pkill -x cosmic-launcher 2>/dev/null || true'   # reload after reapply
# --------------------------------------------------------------------------

LIBDIR=/usr/local/lib/pop-flow
GOLDEN="$LIBDIR/$COMP"
REAPPLY="$LIBDIR/reapply-$COMP"
HOOK="/etc/apt/apt.conf.d/99-pop-flow-$COMP"

[ -f "$BUILT" ] || { echo "Build first: ./install.sh (or cargo build --release)"; exit 1; }

echo "==> Installing golden copy + reapply hook for $COMP (needs sudo)..."
sudo install -d -m 0755 "$LIBDIR"
sudo install -m 0755 "$BUILT" "$GOLDEN"

# The reapplier: reinstall our binary if the system one drifted (or vanished).
# `cmp` follows symlinks, so a package that restores the target as a symlink to
# a stock binary also counts as drift. `install` unlinks the destination first,
# so replacing a symlink leaves whatever it pointed at untouched.
sudo tee "$REAPPLY" >/dev/null <<EOS
#!/usr/bin/env bash
set -e
GOLDEN=$GOLDEN
TARGET=/usr/bin/$COMP
[ -f "\$GOLDEN" ] || exit 0
if [ ! -f "\$TARGET" ] || [ -L "\$TARGET" ] || ! cmp -s "\$GOLDEN" "\$TARGET"; then
    install -m 0755 -o root -g root "\$GOLDEN" "\$TARGET"
    command -v logger >/dev/null 2>&1 && logger -t pop-flow "reapplied $COMP after change"
    $RELOAD
fi
EOS
sudo chmod 0755 "$REAPPLY"

# The hook. `|| true` guarantees a failing reapplier can never break apt.
sudo tee "$HOOK" >/dev/null <<EOS
// POP Flow: reapply our $COMP after any package operation that overwrites
// /usr/bin/$COMP. Remove with ./remove-auto-reapply.sh
DPkg::Post-Invoke { "$REAPPLY || true"; };
EOS

echo "==> Done. POP Flow's $COMP will be reapplied automatically after updates."
echo "    golden: $GOLDEN"
echo "    hook:   $HOOK"
