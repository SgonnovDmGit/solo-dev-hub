#!/usr/bin/env bash
# Regenerate docs/schema/ from the current migrations (T-000083).
#
#   bash scripts/schema-docs.sh [--no-open]
#
# Run this after any schema-changing migration and commit docs/schema/ with it.
#
# The schema is read from a throwaway database built by running every migration
# against an empty file — NOT from your app data. That keeps the docs a function
# of the code, reproducible on any machine and in CI.
#
# Requires:
#   - tbls  — go install github.com/k1LoW/tbls@latest
#             IMPORTANT: must be built with CGO_ENABLED=1, otherwise its SQLite
#             driver is a stub ("go-sqlite3 requires cgo to work") and this
#             script fails at the tbls step. On Windows that means a gcc in PATH
#             (MinGW-w64) at install time.
#   - dot   — Graphviz, for the SVG diagrams

set -euo pipefail

OPEN=1
if [[ "${1:-}" == "--no-open" ]]; then
    OPEN=0
fi

# Windows: Graphviz and go binaries are usually outside the Git Bash PATH.
if [[ -d "/c/Program Files/Graphviz/bin" ]]; then
    export PATH="/c/Program Files/Graphviz/bin:$PATH"
fi
if [[ -d "$HOME/go/bin" ]]; then
    export PATH="$HOME/go/bin:$PATH"
fi
if [[ -d "$HOME/.cargo/bin" ]]; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

command -v tbls >/dev/null || { echo "tbls not found — see the header of this script" >&2; exit 1; }
command -v dot  >/dev/null || { echo "dot (Graphviz) not found — see the header of this script" >&2; exit 1; }

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
DB_PATH="$TMP_DIR/schema.db"

echo "==> building a fresh schema database from migrations"
(
    cd src-tauri
    SDH_SCHEMA_DB_PATH="$DB_PATH" \
        cargo test --lib db::migrations::tests::export_fresh_schema_db \
        -- --ignored --exact --nocapture
)

[[ -f "$DB_PATH" ]] || { echo "schema database was not created at $DB_PATH" >&2; exit 1; }

# tbls parses the DSN as a URL, so it needs a forward-slash absolute path.
# On Windows that is C:/... (cygpath -m), elsewhere the path is already fine.
if command -v cygpath >/dev/null; then
    DB_URL_PATH="$(cygpath -m "$DB_PATH")"
else
    DB_URL_PATH="$DB_PATH"
fi

echo "==> generating docs/schema/"
TBLS_DSN="sqlite:///$DB_URL_PATH" tbls doc --force --config .tbls.yml

echo "==> docs/schema/ regenerated — review the diff and commit it"

if [[ "$OPEN" == "1" && -f docs/schema/schema.svg ]]; then
    # The system browser gives real pan/zoom; the VS Code SVG preview does not.
    if command -v cygstart >/dev/null; then cygstart docs/schema/schema.svg
    elif command -v xdg-open >/dev/null; then xdg-open docs/schema/schema.svg
    elif command -v open     >/dev/null; then open docs/schema/schema.svg
    fi
fi
