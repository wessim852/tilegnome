#!/usr/bin/env bash
set -euo pipefail

UUID='tilegnome@wessim852.github.com'
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname -- "$SCRIPT_DIR")"

if [[ ! -x "$PROJECT_ROOT/target/debug/dwindle-daemon" ]]; then
    cargo build --manifest-path "$PROJECT_ROOT/Cargo.toml" -p dwindle-daemon
fi

dbus-run-session -- bash -c '
set -euo pipefail
PROJECT_ROOT="$1"
UUID="$2"

"$PROJECT_ROOT/target/debug/dwindle-daemon" &
DAEMON_PID=$!
gnome-shell --devkit --wayland --no-x11 &
SHELL_PID=$!

cleanup() {
    kill "$SHELL_PID" "$DAEMON_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

ENABLED=0
for _attempt in {1..40}; do
    if gnome-extensions list 2>/dev/null | grep -Fxq "$UUID"; then
        if gnome-extensions enable "$UUID"; then
            ENABLED=1
            break
        fi
    fi
    sleep 0.5
done
if [[ "$ENABLED" -ne 1 ]]; then
    echo "Could not enable $UUID in the nested session" >&2
    exit 1
fi

wait "$SHELL_PID"
' bash "$PROJECT_ROOT" "$UUID"
