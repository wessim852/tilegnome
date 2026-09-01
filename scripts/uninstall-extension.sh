#!/usr/bin/env bash
set -euo pipefail

UUID='dwindle-rs@dwindlers.dev'
TARGET="$HOME/.local/share/gnome-shell/extensions/$UUID"

gnome-extensions disable "$UUID" 2>/dev/null || true
rm -rf -- "$TARGET"
echo "Removed $TARGET"
