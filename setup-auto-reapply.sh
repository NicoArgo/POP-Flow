#!/usr/bin/env bash
# Make POP Flow survive system/package updates.
#
# A cosmic-launcher package update overwrites /usr/bin/cosmic-launcher with the
# stock binary. This installs an APT/dpkg post-invoke hook that runs (as root,
# no password) after every package operation and reinstalls our build whenever
# the on-disk binary no longer matches our "golden" copy.
#
# One-time setup; needs sudo. Undo with ./remove-auto-reapply.sh
set -euo pipefail
cd "$(dirname "$0")"

LIBDIR=/usr/local/lib/pop-flow
GOLDEN="$LIBDIR/cosmic-launcher"
REAPPLY="$LIBDIR/reapply-cosmic-launcher"
HOOK=/etc/apt/apt.conf.d/99-pop-flow-cosmic-launcher
BUILT="target/release/cosmic-launcher"

[ -f "$BUILT" ] || { echo "Build first: ./install.sh (or cargo build --release)"; exit 1; }

echo "==> Installing golden copy + reapply hook (needs sudo)..."
sudo install -d -m 0755 "$LIBDIR"
sudo install -m 0755 "$BUILT" "$GOLDEN"

# The reapplier: reinstall our binary if the system one drifted (or vanished).
sudo tee "$REAPPLY" >/dev/null <<'EOS'
#!/usr/bin/env bash
set -e
GOLDEN=/usr/local/lib/pop-flow/cosmic-launcher
TARGET=/usr/bin/cosmic-launcher
[ -f "$GOLDEN" ] || exit 0
if [ ! -f "$TARGET" ] || ! cmp -s "$GOLDEN" "$TARGET"; then
    install -m 0755 -o root -g root "$GOLDEN" "$TARGET"
    command -v logger >/dev/null 2>&1 && logger -t pop-flow "reapplied cosmic-launcher after change"
    pkill -x cosmic-launcher 2>/dev/null || true
fi
EOS
sudo chmod 0755 "$REAPPLY"

# The hook. `|| true` guarantees a failing reapplier can never break apt.
sudo tee "$HOOK" >/dev/null <<EOS
// POP Flow: reapply our cosmic-launcher after any package operation that
// overwrites /usr/bin/cosmic-launcher. Remove with ./remove-auto-reapply.sh
DPkg::Post-Invoke { "$REAPPLY || true"; };
EOS

echo "==> Done. POP Flow will be reapplied automatically after updates."
echo "    golden: $GOLDEN"
echo "    hook:   $HOOK"
