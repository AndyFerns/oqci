#!/usr/bin/env bash
# Force-stops oqci-server and the visualization frontend dev server, by port
# and by process name — a safety net for when they weren't stopped through
# dev-visualization.sh (terminal closed out from under them, a crash, a
# manually-started instance, etc.).
#
# Tries lsof, then ss, then netstat, whichever is available — one of the
# three is present on essentially every Linux, macOS, or Git-Bash-on-Windows
# install. On native Windows, scripts/stop-visualization.bat or
# scripts/stop-visualization.ps1 are more reliable: `taskkill /T` kills a
# whole process tree, which plain `kill` on a translated PID is not
# guaranteed to do for a native (non-MSYS) process under Git Bash.
#
# Usage:
#   scripts/stop-visualization.sh [SERVER_PORT] [FRONTEND_PORT]

set -uo pipefail

SERVER_PORT="${1:-4173}"
FRONTEND_PORT="${2:-5173}"

pids_on_port() {
    local port="$1"
    if command -v lsof >/dev/null 2>&1; then
        lsof -ti tcp:"$port" -sTCP:LISTEN 2>/dev/null
    elif command -v ss >/dev/null 2>&1; then
        ss -ltnp "sport = :$port" 2>/dev/null | grep -oE 'pid=[0-9]+' | cut -d= -f2 | sort -u
    elif command -v netstat >/dev/null 2>&1; then
        # Covers both GNU netstat (Linux) and Windows' netstat.exe as seen
        # from Git Bash — the PID is always the last whitespace-separated
        # field on a LISTENING/LISTEN line for that port.
        netstat -ano 2>/dev/null | grep -i listen | grep -E "[:.]${port}[[:space:]]" | awk '{print $NF}' | sort -u
    fi
}

kill_port() {
    local port="$1"
    local pids
    pids="$(pids_on_port "$port")"
    if [[ -z "$pids" ]]; then
        echo "nothing listening on port $port"
        return
    fi
    for pid in $pids; do
        echo "killing pid $pid listening on port $port"
        kill -TERM "$pid" 2>/dev/null || true
    done
    sleep 1
    for pid in $pids; do
        if kill -0 "$pid" 2>/dev/null; then
            kill -KILL "$pid" 2>/dev/null || true
        fi
    done
}

kill_port "$SERVER_PORT"
kill_port "$FRONTEND_PORT"

if command -v pkill >/dev/null 2>&1; then
    echo "backstop: killing any oqci-server process by name..."
    pkill -f "oqci-server" 2>/dev/null || true
fi

echo "done."
