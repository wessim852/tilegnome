#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname -- "$SCRIPT_DIR")"

if [[ -x "$PROJECT_ROOT/target/debug/dwindle-daemon" ]]; then
    exec "$PROJECT_ROOT/target/debug/dwindle-daemon"
fi
exec cargo run --manifest-path "$PROJECT_ROOT/Cargo.toml" -p dwindle-daemon
