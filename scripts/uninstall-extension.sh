#!/usr/bin/env bash
set -euo pipefail

UUID='tilegnome@wessim852.github.com'
TARGET="$HOME/.local/share/gnome-shell/extensions/$UUID"

gnome-extensions disable "$UUID" 2>/dev/null || true
rm -rf -- "$TARGET"
echo "Removed $TARGET"
