#!/usr/bin/env bash
# Free disk space in src-tauri/target/ by removing incremental caches.
#
# Default mode (safe, fast): drop target/debug/incremental/ only.
#   Compiled deps stay → next `cargo test` rebuilds only what changed.
#   Typical win: 5-10GB on a project with frequent builds.
#
# --full mode (aggressive): run `cargo clean` → wipes debug + release.
#   Next build is 5-15min cold compile of all deps.
#   Use after release closure (CI builds the public binary anyway).
#
# Usage:
#   bash scripts/cleanup-target.sh            # safe mode
#   bash scripts/cleanup-target.sh --full     # aggressive mode

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="$REPO_ROOT/src-tauri/target"

mode="safe"
if [[ "${1:-}" == "--full" ]]; then
    mode="full"
fi

if [[ ! -d "$TARGET_DIR" ]]; then
    echo "target/ does not exist yet — nothing to clean."
    exit 0
fi

before=$(du -sh "$TARGET_DIR" 2>/dev/null | awk '{print $1}')
echo "Before: $TARGET_DIR = $before"

if [[ "$mode" == "full" ]]; then
    echo "Mode: --full → cargo clean (wipes debug + release)"
    (cd "$REPO_ROOT/src-tauri" && cargo clean)
else
    echo "Mode: safe → dropping target/debug/incremental/ only"
    inc="$TARGET_DIR/debug/incremental"
    if [[ -d "$inc" ]]; then
        rm -rf "$inc"
        echo "  Removed: $inc"
    else
        echo "  No incremental/ dir to remove"
    fi
fi

after=$(du -sh "$TARGET_DIR" 2>/dev/null | awk '{print $1}')
echo "After:  $TARGET_DIR = $after"
