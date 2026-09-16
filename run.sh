#!/usr/bin/env bash
#
# Build and open the triage window.
#
#   ./run.sh              on your real mail, release build
#   ./run.sh --dev        on the scratch store, against the Docker dev stack
#   ./run.sh --debug      a fast build, for iterating on the UI
#   ./run.sh --data-dir ~/somewhere
#
# Release is the default because the list wants it: the Tauri spike measured a
# debug build at roughly three times the IPC cost, which is the one number that
# decides whether scrolling holds 60fps. See docs/spike-tauri-list.md. Use
# --debug when the thing being changed is markup and the wait matters more.

set -euo pipefail

# Run from the repo root whichever directory this was invoked from, so relative
# paths like .devdata mean the same thing every time.
cd "$(dirname "${BASH_SOURCE[0]}")"

# cargo is not on PATH in a non-login shell on this machine.
export PATH="$HOME/.cargo/bin:$PATH"

PROFILE=release
DATA_DIR=""
DEV=0

usage() {
    sed -n '3,14p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dev)      DEV=1; shift ;;
        --debug)    PROFILE=debug; shift ;;
        --release)  PROFILE=release; shift ;;
        --data-dir)
            [[ $# -ge 2 ]] || { echo "--data-dir needs a path" >&2; exit 1; }
            DATA_DIR="$2"; shift 2 ;;
        -h|--help)  usage 0 ;;
        *)          echo "unknown option: $1" >&2; usage 1 ;;
    esac
done

if [[ $DEV -eq 1 && -z $DATA_DIR ]]; then
    # The scratch store, which is gitignored and safe to delete.
    DATA_DIR=".devdata"
fi

if ! command -v cargo >/dev/null; then
    echo "cargo not found. Install Rust from https://rustup.rs, then try again." >&2
    exit 1
fi

# -j 2 is not a preference. A full-parallelism build of this workspace gets
# OOM-killed on this machine, and a killed build looks like a mystery rather
# than like running out of memory.
BUILD=(cargo build -j 2 -p kuverta-desktop)
[[ $PROFILE == release ]] && BUILD+=(--release)

echo "building (${PROFILE})…"
"${BUILD[@]}"

if [[ $DEV -eq 1 ]]; then
    # The dev stack is where --dev's mail comes from. Not fatal if it is down:
    # the window still opens, it just has nothing to sync against, and saying
    # so beats a connection error later with no explanation.
    if ! nc -z 127.0.0.1 10143 2>/dev/null; then
        echo "dev IMAP server is not up — start it with: make dev-up" >&2
    fi
fi

BIN="./target/${PROFILE}/kuverta-desktop"
if [[ -n $DATA_DIR ]]; then
    echo "opening on ${DATA_DIR}"
    exec env KUVERTA_DATA_DIR="$DATA_DIR" "$BIN"
else
    # No KUVERTA_DATA_DIR: the app falls back to ~/.local/share/kuverta.
    echo "opening on the default data directory"
    exec "$BIN"
fi
