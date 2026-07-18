# POP Flow

**A Windows-style Alt-Tab window switcher with live thumbnails for Pop!_OS COSMIC.**

COSMIC's stock Alt-Tab is a text list. POP Flow replaces it with a grid of **live
window previews** — like the Windows Alt-Tab — so you can see what you're switching to.

POP Flow is a small patch on top of System76's
[`cosmic-launcher`](https://github.com/pop-os/cosmic-launcher) (the program COSMIC
already invokes as `cosmic-launcher alt-tab`). Nothing else in your desktop is
touched — the compositor is left completely alone.

## Features

- 🖼️ **Window thumbnails** captured through the Wayland `ext-image-copy-capture`
  (screencopy) protocol — the real contents of each window, snapshotted (and
  downscaled) each time the switcher opens.
- ▦ **Two-column grid** layout, thumbnail-first.
- 🔲 **Rounded thumbnail corners** (anti-aliased alpha mask).
- 🏷️ **Clean by default** — the window title is hidden and only appears on the
  **selected** or **hovered** thumbnail.
- ❌ **Close from the switcher** — a Windows-style close button appears on the
  hovered/selected thumbnail (or middle-click any thumbnail) to close that window
  via the Wayland `cosmic-toplevel-management` protocol, without switching to it.
- 🪶 Capturing only runs **while the switcher is open**, so there's no idle cost.

> Screenshot: _add your own — the switcher renders whatever windows you have open._

## How it works

- `src/wayland.rs` — a dedicated Wayland thread that lists open toplevels
  (`ext-foreign-toplevel-list` + cosmic toplevel-info) and captures each one to an
  `image::Handle` via screencopy. It is gated by a command channel so it only
  captures while Alt-Tab is on screen.
- `src/app.rs` — stores the thumbnails, correlates them to the switcher entries by
  window title (normalizing the terminal's animated spinner glyph), and renders the
  grid in `alt_tab_view`.

No new system dependencies and **no libcosmic bump** were needed — the screencopy
and toplevel-info APIs are already available through the `cosmic-client-toolkit`
that `cosmic-launcher` ships with.

## Requirements

- Pop!_OS 24.04 (COSMIC) or another COSMIC session on Wayland.
- Rust toolchain (`rustup`).
- Build dependencies:

  ```bash
  sudo apt install -y libxkbcommon-dev libwayland-dev pkg-config
  ```

## Install

```bash
git clone https://github.com/NicoArgo/POP-Flow.git
cd POP-Flow
./install.sh
```

`install.sh` builds a release binary, backs up your current
`/usr/bin/cosmic-launcher` to `./cosmic-launcher.orig`, installs the patched
binary, and restarts the launcher. Then just press **Alt+Tab**.

## Uninstall

```bash
./uninstall.sh
```

Restores the original binary from the backup.

> Note: a system update to the `cosmic-launcher` package will overwrite the patched
> binary with the stock one. Just run `./install.sh` again to reapply.

## Credits & License

POP Flow is a derivative work of
[`pop-os/cosmic-launcher`](https://github.com/pop-os/cosmic-launcher) by System76.
The original project README is preserved as [`README.upstream.md`](./README.upstream.md).

Licensed under the **GNU General Public License v3.0** — see [`LICENSE.md`](./LICENSE.md).
