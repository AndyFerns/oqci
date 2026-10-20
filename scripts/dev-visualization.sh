#!/usr/bin/env bash
# Runs oqci-server and the visualization frontend together, and guarantees
# both process trees are stopped together on exit — including Ctrl+C.
#
# `set -m` gives each backgrounded job its own process group, which is what
# lets the cleanup trap kill an entire tree (`npm run dev` spawns its own
# node/vite child that killing only the npm PID would leave running) with
# one `kill -- -$pid`.
#
# See docs/visualization.md and server/README.md.
#
# Usage:
#   scripts/dev-visualization.sh [--root DIR] [--backend ID] \
#       [--server-port PORT] [--frontend-port PORT]

set -euo pipefail
set -m

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
FRONTEND_DIR="$REPO_ROOT/frontend"

ROOT="."
BACKEND="simulator-nisq"
SERVER_PORT=4173
FRONTEND_PORT=5173

usage() {
    cat <<EOF
Usage: $0 [--root DIR] [--backend ID] [--server-port PORT] [--frontend-port PORT]

Starts oqci-server and the visualization frontend together. Ctrl+C stops
both, including their child processes.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --root) ROOT="$2"; shift 2 ;;
        --backend) BACKEND="$2"; shift 2 ;;
        --server-port) SERVER_PORT="$2"; shift 2 ;;
        --frontend-port) FRONTEND_PORT="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; usage; exit 1 ;;
    esac
done

if [[ ! -f "$FRONTEND_DIR/package.json" ]]; then
    echo "error: $FRONTEND_DIR/package.json not found — run 'npm install' in frontend/ first" >&2
    exit 1
fi

SERVER_PID=""
FRONTEND_PID=""
CLEANED_UP=0

cleanup() {
    if [[ "$CLEANED_UP" -eq 1 ]]; then
        return
    fi
    CLEANED_UP=1
    echo ""
    for pid in "$SERVER_PID" "$FRONTEND_PID"; do
        if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
            echo "stopping pgid $pid and its children..."
            kill -TERM -"$pid" 2>/dev/null || true
        fi
    done
    sleep 1
    for pid in "$SERVER_PID" "$FRONTEND_PID"; do
        if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
            kill -KILL -"$pid" 2>/dev/null || true
        fi
    done
    echo "both stopped."
}
trap cleanup EXIT INT TERM

echo "starting oqci-server on port $SERVER_PORT (backend: $BACKEND)..."
(cd "$REPO_ROOT" && exec cargo run -p oqci-server -- --root "$ROOT" --backend "$BACKEND" --port "$SERVER_PORT") &
SERVER_PID=$!

echo "starting frontend dev server on port $FRONTEND_PORT..."
(cd "$FRONTEND_DIR" && exec npm run dev -- --port "$FRONTEND_PORT") &
FRONTEND_PID=$!

echo ""
echo "  server:   http://localhost:$SERVER_PORT"
echo "  frontend: http://localhost:$FRONTEND_PORT"
echo ""
echo "Press Ctrl+C to stop both."
echo ""

while kill -0 "$SERVER_PID" 2>/dev/null && kill -0 "$FRONTEND_PID" 2>/dev/null; do
    sleep 1
done

if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "warning: oqci-server exited on its own" >&2
fi
if ! kill -0 "$FRONTEND_PID" 2>/dev/null; then
    echo "warning: frontend dev server exited on its own" >&2
fi
