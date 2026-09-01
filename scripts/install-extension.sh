#!/usr/bin/env bash
set -euo pipefail

UUID='dwindle-rs@dwindlers.dev'
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname -- "$SCRIPT_DIR")"
TARGET="$HOME/.local/share/gnome-shell/extensions/$UUID"

command -v glib-compile-schemas >/dev/null || {
    echo 'glib-compile-schemas not found; run this script inside nix develop' >&2
    exit 1
}

gnome-extensions disable "$UUID" 2>/dev/null || true
rm -rf -- "$TARGET"
install -d -- "$TARGET"
cp -R -- "$PROJECT_ROOT/extension/." "$TARGET/"
glib-compile-schemas --strict "$TARGET/schemas"
echo "Installed $UUID at $TARGET"
if gnome-extensions info "$UUID" >/dev/null 2>&1; then
    echo "Enable with: gnome-extensions enable $UUID"
else
    echo "GNOME Shell has not discovered this first-time install yet."
    echo "Log out and back in, then run: gnome-extensions enable $UUID"
fi
