# DwindleRS for GNOME 50

A personal GNOME Shell 50 tiling extension whose layout engine runs in Rust. GNOME Shell only observes Mutter state, sends typed JSON over the user session D-Bus, applies validated rectangles, and owns keybindings.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the responsibility boundary.

## Build and test

This repository pins nixpkgs in `flake.lock`. This NixOS host has flakes disabled globally, so enter the shell without changing global configuration:

```sh
nix --extra-experimental-features 'nix-command flakes' develop
cargo test
cargo build
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

If flakes are already enabled in your Nix configuration, plain `nix develop` is enough.

## Install and run in the real GNOME Wayland session

Inside `nix develop`:

```sh
cargo build
./scripts/install-extension.sh
```

GNOME Shell 50 enumerates manually installed extensions when the Shell session starts. On the first install, log out and back in once; restarting Shell with `Alt+F2`, `r` is not supported on Wayland. GJS source updates in the real Wayland session also require logging out and back in; use the nested devkit workflow below for a faster edit/test loop. Rust-only updates need only a daemon restart.

Start the daemon in a terminal and leave it running:

```sh
RUST_LOG=dwindle_daemon=info,dwindle_core=debug ./scripts/run-daemon.sh
```

In another terminal:

```sh
gnome-extensions enable dwindle-rs@dwindlers.dev
gnome-extensions info dwindle-rs@dwindlers.dev
```

Disable or remove it safely:

```sh
gnome-extensions disable dwindle-rs@dwindlers.dev
./scripts/uninstall-extension.sh
```

Re-run `install-extension.sh` after editing GJS or schemas; it disables the old copy before replacement. No NixOS, Home Manager, or systemd configuration is modified.

## Shortcuts

| Action | Default |
|---|---|
| Focus left/down/up/right | `Super+Alt+H`, `Super+J`, `Super+K`, `Super+Alt+L` |
| Swap left/down/up/right | `Super+Shift+H/J/K/L` |
| Resize left/down/up/right | `Super+Ctrl+H/J/K/L` |
| Toggle floating | `Super+Alt+Space` |
| Toggle fullscreen | `Super+F` |
| Group/ungroup focused subtree | `Super+G` |
| Previous/next window in group | `Super+Alt+K/J` |

The installed GNOME 50 configuration reserves `Super+H` for minimize, `Super+L` for locking the session, and `Super+Shift+Space` for reverse input-source switching. Those defaults are avoided. To reclaim two of them intentionally, clear GNOME's bindings and change this extension's settings:

```sh
gsettings set org.gnome.desktop.wm.keybindings minimize '@as []'
gsettings set org.gnome.desktop.wm.keybindings switch-input-source-backward '@as []'
export GSETTINGS_SCHEMA_DIR="$HOME/.local/share/gnome-shell/extensions/dwindle-rs@dwindlers.dev/schemas"
gsettings set dev.dwindlers dwindle-focus-left "['<Super>h']"
gsettings set dev.dwindlers dwindle-toggle-floating "['<Shift><Super>space']"
```

## Settings

The schema defaults are enabled, 8-pixel inner/outer gaps, 0.5 new-split ratio, 0.05 resize step, and smart splitting. For example:

```sh
export GSETTINGS_SCHEMA_DIR="$HOME/.local/share/gnome-shell/extensions/dwindle-rs@dwindlers.dev/schemas"
gsettings set dev.dwindlers inner-gap 12
gsettings set dev.dwindlers outer-gap 12
gsettings set dev.dwindlers split-ratio 0.55
gsettings set dev.dwindlers resize-step 0.05
gsettings set dev.dwindlers smart-split true
gsettings set dev.dwindlers ignored-apps "['org.gnome.Calculator', 'steam']"
gsettings set dev.dwindlers enabled false
```

Geometry, filtering, and enabled-setting changes trigger a complete reconciliation and relayout.

## Logs

Rust logs stay in the terminal running `run-daemon.sh`. Increase detail with `RUST_LOG=dwindle_core=debug,dwindle_daemon=debug`.

Follow GNOME Shell adapter logs in another terminal:

```sh
journalctl --user -f -o cat | rg '\[DwindleRS\]'
```

The daemon can be inspected directly on the session bus:

```sh
busctl --user introspect dev.dwindlers.Engine /dev/dwindlers/Engine
```

If the daemon exits, the extension pauses without moving windows. Restarting it causes the persistent proxy to notice the new bus owner and send a FullSync.

## Nested GNOME 50 test

The installed `gnome-shell --help` advertises `--devkit` and `--wayland`. Install the extension first, then run this inside `nix develop`:

```sh
./scripts/install-extension.sh
./scripts/test-nested.sh
```

The script starts both `dwindle-daemon` and `gnome-shell --devkit --wayland --no-x11` inside one `dbus-run-session`, then enables the extension on that nested session. The regular desktop's daemon is intentionally not visible there.

## Manual multi-monitor acceptance test

1. Run `cargo test`, `cargo build`, and the install/start/enable commands above.
2. Open Firefox on monitor 1. Confirm it fills that monitor's work area minus the outer gap.
3. Open one terminal on monitor 1. Confirm only that context becomes a left/right split.
4. Focus the terminal and open another terminal. Confirm its leaf splits top/bottom.
5. Open Zed and another application on monitor 2. Confirm monitor 1 does not move.
6. Close a monitor-1 window. Confirm its parent collapses and the sibling fills the freed branch.
7. Drag a tiled window across monitors and release. Confirm both contexts retile.
8. Move a tiled window to another workspace. Visit both workspaces and confirm each context is complete.
9. Test focus with `Super+Alt+H` for left, `Super+J/K` for down/up, and `Super+Alt+L` for right.
10. Test swaps with `Super+Shift+H/J/K/L` and ratio changes with `Super+Ctrl+H/J/K/L`.
11. Press `Super+F` on one window. Confirm it leaves tiling and becomes fullscreen; press it again and confirm it rejoins the layout.
12. Press `Super+G` on a tiled window. Confirm its sibling subtree shares one rectangle, then cycle the visible member with `Super+Alt+J/K`; press `Super+G` again to restore separate leaves.
13. Toggle floating with `Super+Alt+Space`, move/resize the window, then toggle again and confirm it rejoins the focused context.
14. Disable the extension, exercise GNOME normally, and confirm no shortcut remains active. Re-enable it and confirm current windows are tiled by FullSync.
15. Stop and restart the Rust daemon while windows remain open. Confirm the extension logs reconnection and reconstructs every workspace/monitor context.
16. Hotplug or reconfigure a monitor. Confirm a topology FullSync rebuilds all contexts without stale placements.

## Known MVP limitations

- FullSync and monitor topology changes rebuild trees deterministically; they do not preserve the exact previous split tree.
- Floating state is in Rust and is intentionally not persisted. A daemon/extension restart tiles eligible floating windows again.
- Tab groups are rebuilt as ordinary dwindle leaves after a daemon or extension restart.
- Directional focus stays within one workspace/monitor context.
- Mouse grabs only migrate context or restore the existing layout; there are no drop targets or insertion previews.
- Mutter/client minimum-size constraints may prevent a client from matching a very small calculated rectangle exactly.
- With `smart-split=false`, new splits are horizontal. There is no alternate layout, preferences UI, animation, gesture, or overview integration.
