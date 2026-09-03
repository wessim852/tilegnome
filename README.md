# TileGNOME

TileGNOME is a binary-tree tiling window manager for GNOME Shell 50 on Wayland. It provides a Hyprland-inspired dwindle layout while remaining native to GNOME and Mutter.

The project keeps window-management integration small and explicit:

- A GNOME Shell extension observes windows, workspaces, monitors, focus, and work areas.
- A Rust daemon owns the tiling tree and all layout decisions.
- The two components communicate over the user D-Bus session.
- Mutter applies placements directly, without `wmctrl`, `xdotool`, or other X11 tools.

Each workspace and monitor has an independent layout tree. Window insertion splits the focused leaf, removal immediately collapses its parent, and geometry is always derived from the current tree. See [ARCHITECTURE.md](ARCHITECTURE.md) for the component boundaries.

## Features

- Dwindle/BSP insertion around the focused tiled window
- Independent layouts for every workspace and monitor
- Directional focus, swapping, and split-ratio resizing
- Per-window floating mode
- Per-context layout maximize that preserves the underlying tree
- Configurable gaps, split ratio, resize step, and ignored applications
- Focus borders with active-window highlighting
- D-Bus reconnection and full-state recovery after daemon restarts
- Native GNOME Wayland window placement through Mutter

## Requirements

- GNOME Shell 50
- Wayland session
- Rust and Cargo
- GJS and GNOME Shell development tools
- GLib utilities, including `glib-compile-schemas`
- D-Bus session utilities

This repository currently targets GNOME Shell 50 only. Check your session before installing:

```sh
gnome-shell --version
echo "$XDG_SESSION_TYPE"
```

## Install on NixOS

Clone the repository and build/install from the included development shell. This does not modify your NixOS or Home Manager configuration.

```sh
git clone https://github.com/wessim852/tilegnome.git
cd tilegnome
nix --extra-experimental-features 'nix-command flakes' develop path:. -c bash -c 'cargo build --workspace && ./scripts/install-extension.sh'
```

If flakes are enabled globally, the shorter form is:

```sh
nix develop path:. -c bash -c 'cargo build --workspace && ./scripts/install-extension.sh'
```

On the first installation, log out and back in so GNOME Shell discovers the extension. Wayland does not support reloading Shell with `Alt+F2`, `r`.

Start the Rust daemon and leave this terminal open:

```sh
cd tilegnome
nix --extra-experimental-features 'nix-command flakes' develop path:. -c ./scripts/run-daemon.sh
```

Enable the extension in another terminal:

```sh
gnome-extensions enable tilegnome@wessim852.github.com
gnome-extensions info tilegnome@wessim852.github.com
```

## Install on other Linux distributions

Install the build and GNOME runtime dependencies using your distribution's package manager.

Fedora:

```sh
sudo dnf install git rust cargo gcc pkgconf-pkg-config gjs gnome-shell glib2-devel dbus-daemon
```

Arch Linux:

```sh
sudo pacman -S --needed git rust gcc pkgconf gjs gnome-shell glib2 dbus
```

Ubuntu/Debian:

```sh
sudo apt install git rustc cargo build-essential pkg-config gjs gnome-shell libglib2.0-dev dbus
```

Your distribution must still provide GNOME Shell 50. Then build and install:

```sh
git clone https://github.com/wessim852/tilegnome.git
cd tilegnome
cargo build --workspace
./scripts/install-extension.sh
```

Log out and back in after the first installation. Start the daemon:

```sh
cd tilegnome
RUST_LOG=dwindle_daemon=info,dwindle_core=info ./scripts/run-daemon.sh
```

In another terminal, enable TileGNOME:

```sh
gnome-extensions enable tilegnome@wessim852.github.com
```

## Keyboard shortcuts

| Action | Shortcut |
| --- | --- |
| Focus left | `Super+Alt+H` |
| Focus down | `Super+J` |
| Focus up | `Super+K` |
| Focus right | `Super+Alt+L` |
| Swap window left/down/up/right | `Super+Shift+H/J/K/L` |
| Resize split left/down/up/right | `Super+Ctrl+H/J/K/L` |
| Toggle floating | `Super+Alt+Space` |
| Toggle layout maximize | `Super+F` |

`Super+H`, `Super+L`, and `Super+Shift+Space` are intentionally avoided because GNOME commonly reserves them for minimize, screen lock, and input-source switching.

## Configuration

TileGNOME uses the stable `dev.dwindlers` GSettings schema. Point `gsettings` at the extension's locally compiled schema before changing values:

```sh
export GSETTINGS_SCHEMA_DIR="$HOME/.local/share/gnome-shell/extensions/tilegnome@wessim852.github.com/schemas"
gsettings set dev.dwindlers inner-gap 12
gsettings set dev.dwindlers outer-gap 12
gsettings set dev.dwindlers split-ratio 0.55
gsettings set dev.dwindlers resize-step 0.05
gsettings set dev.dwindlers smart-split true
gsettings set dev.dwindlers ignored-apps "['org.gnome.Calculator', 'steam']"
```

Available settings:

| Setting | Default | Purpose |
| --- | --- | --- |
| `enabled` | `true` | Enables automatic tiling |
| `inner-gap` | `8` | Space between tiled windows |
| `outer-gap` | `8` | Space around each monitor work area |
| `split-ratio` | `0.5` | Ratio assigned to newly created splits |
| `resize-step` | `0.05` | Keyboard resize increment |
| `smart-split` | `true` | Selects the split axis from the current tree branch |
| `ignored-apps` | `[]` | Application IDs excluded from tiling |

Geometry settings relayout existing trees without rebuilding them. The default split ratio only affects future insertions.

## Updating

```sh
cd tilegnome
git pull --ff-only
cargo build --workspace
./scripts/install-extension.sh
```

Restart the daemon after Rust changes. Log out and back in after extension or schema changes so GNOME Shell does not reuse cached modules.

## Logs and diagnostics

Rust logs are written to the daemon terminal. Enable detailed output with:

```sh
RUST_LOG=dwindle_daemon=debug,dwindle_core=debug ./scripts/run-daemon.sh
```

Follow GNOME Shell adapter logs with:

```sh
journalctl --user -f -o cat | rg '\[TileGNOME\]'
```

Inspect the daemon on the user session bus:

```sh
busctl --user introspect dev.dwindlers.Engine /dev/dwindlers/Engine
```

If the daemon is unavailable, the extension pauses placement safely. Restarting the daemon reconnects the adapter and reconciles the current window state.

## Development

Run the complete Rust validation suite:

```sh
nix --extra-experimental-features 'nix-command flakes' develop path:. -c cargo fmt --all --check
nix --extra-experimental-features 'nix-command flakes' develop path:. -c cargo clippy --workspace --all-targets -- -D warnings
nix --extra-experimental-features 'nix-command flakes' develop path:. -c cargo test --workspace
```

For a nested GNOME Shell session, install the extension and run:

```sh
./scripts/test-nested.sh
```

The script starts the daemon and `gnome-shell --devkit --wayland --no-x11` inside the same isolated D-Bus session.

## Disable or uninstall

```sh
gnome-extensions disable tilegnome@wessim852.github.com
./scripts/uninstall-extension.sh
```

The uninstall script removes only the per-user extension directory. It does not change system configuration.
