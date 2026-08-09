#!/usr/bin/env bash
# OQCI version bumper (Linux / macOS / Git Bash).
#
# The repo-root `VERSION` file is the single source of truth for the project
# version (semantic versioning, MAJOR.MINOR.PATCH). Run this whenever you cut a
# change worth versioning; it bumps `VERSION` and keeps `Cargo.toml`'s package
# version in sync so the two can never drift.
#
# When to bump what (semantic versioning):
#   major  — breaking change to a public API or IR contract (0.x: still allowed)
#   minor  — new backwards-compatible capability / milestone (e.g. a new phase)
#   patch  — fixes, docs, internal changes with no API surface change
#
# Usage:
#   scripts/bump-version.sh patch          # 0.0.1 -> 0.0.2
#   scripts/bump-version.sh minor          # 0.0.1 -> 0.1.0
#   scripts/bump-version.sh major          # 0.9.3 -> 1.0.0
#   scripts/bump-version.sh set 1.2.3      # set an explicit version
#   scripts/bump-version.sh --show         # print the current version
#   scripts/bump-version.sh -h | --help
#
# On success it prints `old -> new` and a suggested tag command (it does NOT
# create tags or commits — that stays your decision).
#
# Exit codes: 0 ok · 2 bad arguments · 3 malformed VERSION / file error

set -uo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION_FILE="$ROOT/VERSION"
CARGO_FILE="$ROOT/Cargo.toml"

die() { echo "error: $*" >&2; exit "${2:-3}"; }

usage() { sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'; }

[ $# -ge 1 ] || { echo "error: missing argument" >&2; usage; exit 2; }

case "$1" in
    -h|--help) usage; exit 0 ;;
esac

# --- read + validate current version ------------------------------------------

[ -f "$VERSION_FILE" ] || die "VERSION file not found at $VERSION_FILE"
current="$(tr -d ' \t\r\n' < "$VERSION_FILE")"
[ -n "$current" ] || die "VERSION file is empty"

if [[ ! "$current" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    die "VERSION '$current' is not valid semver (MAJOR.MINOR.PATCH)"
fi

IFS='.' read -r MAJOR MINOR PATCH <<< "$current"

# --- compute the new version --------------------------------------------------

case "$1" in
    --show)
        echo "$current"
        exit 0
        ;;
    major) new="$((MAJOR + 1)).0.0" ;;
    minor) new="${MAJOR}.$((MINOR + 1)).0" ;;
    patch) new="${MAJOR}.${MINOR}.$((PATCH + 1))" ;;
    set)
        [ $# -ge 2 ] || die "'set' needs a version, e.g. 'set 1.2.3'" 2
        new="$2"
        [[ "$new" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "'$new' is not valid semver" 2
        ;;
    *)
        echo "error: unknown bump kind: $1" >&2
        usage
        exit 2
        ;;
esac

# --- write VERSION, then sync Cargo.toml [package] version --------------------

printf '%s\n' "$new" > "$VERSION_FILE" || die "could not write $VERSION_FILE"

if [ -f "$CARGO_FILE" ]; then
    # Replace `version = "..."` only inside the [package] table, so dependency
    # version strings are never touched. Portable awk (no in-place needed).
    tmp="$(mktemp)"
    awk -v newver="$new" '
        /^\[/ { in_pkg = ($0 == "[package]") }
        in_pkg && /^[[:space:]]*version[[:space:]]*=/ {
            sub(/version[[:space:]]*=[[:space:]]*"[^"]*"/, "version = \"" newver "\"")
        }
        { print }
    ' "$CARGO_FILE" > "$tmp" && mv "$tmp" "$CARGO_FILE" || die "could not update $CARGO_FILE"
fi

echo "version: $current -> $new"
echo "updated: VERSION, Cargo.toml"
echo "next (optional): git commit -am \"chore: bump version to $new\" && git tag v$new"
