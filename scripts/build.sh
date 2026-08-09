#!/usr/bin/env bash
# OQCI full-pipeline build script (Linux / macOS / Git Bash).
#
# Runs every check the project cares about, in order:
#   fmt-check | clippy | test | rustdoc | mdbook
#
# All stages route through cargo/mdbook, which auto-discover modules and
# chapters — this script does not enumerate them, so adding a new module,
# integration test, or doc chapter never requires editing here.
#
# On failure it prints a single LLM-friendly summary block delimited by
# "=====", one section per failed stage, with the exit code and the full
# captured output. Copy the whole block into a chat to get help.
#
# Usage:
#   scripts/build.sh                # run everything
#   scripts/build.sh --fix          # auto-apply `cargo fmt` before checks
#   scripts/build.sh --fast         # skip mdbook (Rust checks only)
#   scripts/build.sh --docs-only    # just build the doc site
#   scripts/build.sh --serve        # after docs build, serve them locally
#   scripts/build.sh -h | --help    # show this help
#
# Exit codes:
#   0  every non-skipped stage passed
#   1  one or more stages failed (see summary block)
#   2  bad arguments or missing required tool

set -uo pipefail

# ------------------------------------------------------------------ config

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

FIX=0
FAST=0
DOCS_ONLY=0
SERVE=0

for arg in "$@"; do
    case "$arg" in
        --fix)        FIX=1 ;;
        --fast)       FAST=1 ;;
        --docs-only)  DOCS_ONLY=1 ;;
        --serve)      SERVE=1 ;;
        -h|--help)
            sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "error: unknown argument: $arg" >&2
            echo "run '$0 --help' for usage" >&2
            exit 2
            ;;
    esac
done

# Color output only when writing to a TTY, so log files stay clean.
if [ -t 1 ]; then
    C_BOLD=$'\033[1m'; C_DIM=$'\033[2m'; C_RED=$'\033[31m'
    C_GRN=$'\033[32m'; C_YEL=$'\033[33m'; C_CYA=$'\033[36m'; C_RST=$'\033[0m'
else
    C_BOLD=''; C_DIM=''; C_RED=''; C_GRN=''; C_YEL=''; C_CYA=''; C_RST=''
fi

# ------------------------------------------------------------------ state

STAGE_NAMES=()
STAGE_STATUS=()   # pass | fail:<rc> | skip:<reason>
STAGE_OUTPUT=()   # path to captured combined output (or "" if skipped)
STAGE_SECS=()

TMPDIR_STAGES="$(mktemp -d -t oqci-build.XXXXXX)"
trap 'rm -rf "$TMPDIR_STAGES"' EXIT

# ------------------------------------------------------------------ helpers

require_tool() {
    local tool="$1" hint="$2"
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "${C_RED}error:${C_RST} required tool '$tool' not found on PATH" >&2
        echo "  install hint: $hint" >&2
        exit 2
    fi
}

# run_stage <name> <optional?:0|1> <cmd> [args...]
#   optional=1 marks the stage as skippable if its prerequisite tool is missing.
run_stage() {
    local name="$1"; shift
    local optional="$1"; shift
    local out="$TMPDIR_STAGES/$(printf '%s' "$name" | tr -c 'A-Za-z0-9._-' '_').log"
    local tool="$1"

    if ! command -v "$tool" >/dev/null 2>&1; then
        if [ "$optional" = "1" ]; then
            printf '%s⏭  [%s] SKIP%s — tool not found: %s\n' \
                "$C_YEL" "$name" "$C_RST" "$tool"
            STAGE_NAMES+=("$name"); STAGE_STATUS+=("skip:tool-missing:$tool")
            STAGE_OUTPUT+=(""); STAGE_SECS+=("0")
            return 0
        fi
        printf '%s✗  [%s] FAILED%s — required tool not found: %s\n' \
            "$C_RED" "$name" "$C_RST" "$tool"
        STAGE_NAMES+=("$name"); STAGE_STATUS+=("fail:127")
        STAGE_OUTPUT+=(""); STAGE_SECS+=("0")
        return 1
    fi

    printf '%s▶  [%s]%s %s%s%s\n' \
        "$C_CYA" "$name" "$C_RST" "$C_DIM" "$*" "$C_RST"

    local start_ts end_ts dur rc
    start_ts=$SECONDS
    # Live-stream while capturing. PIPESTATUS[0] preserves the command's rc.
    "$@" 2>&1 | tee "$out"
    rc=${PIPESTATUS[0]}
    end_ts=$SECONDS
    dur=$((end_ts - start_ts))

    if [ "$rc" -eq 0 ]; then
        printf '%s✓  [%s] ok%s (%ds)\n' "$C_GRN" "$name" "$C_RST" "$dur"
        STAGE_NAMES+=("$name"); STAGE_STATUS+=("pass")
        STAGE_OUTPUT+=("$out"); STAGE_SECS+=("$dur")
        return 0
    fi

    printf '%s✗  [%s] FAILED%s (exit %d, %ds)\n' \
        "$C_RED" "$name" "$C_RST" "$rc" "$dur"
    STAGE_NAMES+=("$name"); STAGE_STATUS+=("fail:$rc")
    STAGE_OUTPUT+=("$out"); STAGE_SECS+=("$dur")
    return "$rc"
}

# ------------------------------------------------------------------ stages

require_tool cargo "install Rust via https://rustup.rs"

printf '%s%s== OQCI build pipeline ==%s\n' "$C_BOLD" "$C_CYA" "$C_RST"
printf '%srepo:%s %s\n' "$C_DIM" "$C_RST" "$ROOT"
printf '%stoolchain:%s %s\n\n' "$C_DIM" "$C_RST" "$(rustc --version 2>/dev/null || echo 'unknown')"

if [ "$DOCS_ONLY" -eq 0 ]; then
    if [ "$FIX" -eq 1 ]; then
        run_stage "fmt-apply" 0 cargo fmt --all || true
    fi
    run_stage "fmt-check"    0 cargo fmt --all -- --check                        || true
    run_stage "clippy"       0 cargo clippy --all-targets --all-features -- -D warnings || true
    run_stage "test"         0 cargo test --all-targets --all-features           || true
    run_stage "doctest"      0 cargo test --doc --all-features                   || true
    run_stage "rustdoc"      0 env RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features || true
fi

if [ "$FAST" -eq 0 ]; then
    run_stage "mdbook"       1 mdbook build docs                                 || true
    if [ "$SERVE" -eq 1 ]; then
        # `serve` blocks — don't capture it into the results table.
        if command -v mdbook >/dev/null 2>&1; then
            echo ""; echo "${C_CYA}▶  serving docs at http://localhost:3000${C_RST}"
            exec mdbook serve docs --open
        fi
    fi
fi

# ------------------------------------------------------------------ report

echo ""
printf '%s%s============================================================%s\n' "$C_BOLD" "$C_CYA" "$C_RST"
printf '%s%s BUILD SUMMARY%s\n' "$C_BOLD" "$C_CYA" "$C_RST"
printf '%s%s============================================================%s\n' "$C_BOLD" "$C_CYA" "$C_RST"

fail_count=0
skip_count=0
for i in "${!STAGE_NAMES[@]}"; do
    name="${STAGE_NAMES[$i]}"
    status="${STAGE_STATUS[$i]}"
    secs="${STAGE_SECS[$i]}"
    case "$status" in
        pass)         printf '  %s✓%s %-14s ok      %4ss\n' "$C_GRN" "$C_RST" "$name" "$secs" ;;
        fail:*)       rc="${status#fail:}"; fail_count=$((fail_count+1))
                      printf '  %s✗%s %-14s FAILED  %4ss  (exit %s)\n' "$C_RED" "$C_RST" "$name" "$secs" "$rc" ;;
        skip:*)       reason="${status#skip:}"; skip_count=$((skip_count+1))
                      printf '  %s⏭%s %-14s skipped        (%s)\n' "$C_YEL" "$C_RST" "$name" "$reason" ;;
    esac
done

if [ "$fail_count" -eq 0 ]; then
    echo ""
    printf '%s%s✓ all stages passed%s' "$C_BOLD" "$C_GRN" "$C_RST"
    if [ "$skip_count" -gt 0 ]; then printf ' (%d skipped)' "$skip_count"; fi
    echo ""
    exit 0
fi

# LLM-friendly failure block: one section per failed stage, easy to copy.
echo ""
echo "============================================================"
echo "BUILD FAILED — $fail_count stage(s) failed"
echo "toolchain: $(rustc --version 2>/dev/null || echo unknown)"
echo "platform:  $(uname -srm 2>/dev/null || echo unknown)"
echo "cwd:       $ROOT"
echo "============================================================"

for i in "${!STAGE_NAMES[@]}"; do
    status="${STAGE_STATUS[$i]}"
    case "$status" in
        fail:*)
            name="${STAGE_NAMES[$i]}"
            rc="${status#fail:}"
            out="${STAGE_OUTPUT[$i]}"
            echo ""
            echo "--- [$name] exit=$rc ---"
            if [ -n "$out" ] && [ -s "$out" ]; then
                cat "$out"
            else
                echo "(no captured output — prerequisite check failed)"
            fi
            ;;
    esac
done

echo ""
echo "============================================================"
echo "END BUILD FAILURE — copy from 'BUILD FAILED' through this line"
echo "============================================================"

exit 1
