# `scripts/` — OQCI build pipeline

Cross-platform launchers for the full OQCI check suite. Both scripts implement
the **same stage set** in the **same order** and emit the **same
LLM-friendly failure format**, so you can move between machines without
retraining your eye.

| Platform | Script | Shell |
|----------|--------|-------|
| Linux, macOS, Git Bash on Windows | [`build.sh`](build.sh) | `bash` |
| Windows | [`build.bat`](build.bat) | `cmd.exe` |

## Stages

Order is fixed; every stage runs even if an earlier one fails, so one run
surfaces every problem instead of one at a time.

| # | Stage | Command | Notes |
|---|-------|---------|-------|
| 1 | `fmt-apply`  | `cargo fmt --all` | only with `--fix` |
| 2 | `fmt-check`  | `cargo fmt --all -- --check` | style gate |
| 3 | `clippy`     | `cargo clippy --all-targets --all-features -- -D warnings` | lints as errors |
| 4 | `test`       | `cargo test --all-targets --all-features` | unit + integration |
| 5 | `doctest`    | `cargo test --doc --all-features` | doc examples |
| 6 | `rustdoc`    | `cargo doc --no-deps` with `RUSTDOCFLAGS=-D warnings` | API docs, warnings deny |
| 7 | `mdbook`     | `mdbook build docs` | doc **site** — skipped gracefully if `mdbook` isn't installed |

All commands go through `cargo`/`mdbook`, which auto-discover crate modules,
integration tests, and mdBook chapters. **Adding a new module, test, or doc
chapter never requires editing these scripts.**

## Usage

```bash
scripts/build.sh                # or scripts\build.bat  — run everything
scripts/build.sh --fix          # auto-apply cargo fmt first
scripts/build.sh --fast         # Rust checks only (skip mdbook)
scripts/build.sh --docs-only    # just build the docs site
scripts/build.sh --serve        # build docs, then `mdbook serve` on :3000
scripts/build.sh --help
```

Exit codes: `0` = every non-skipped stage passed; `1` = one or more stages
failed (see failure block); `2` = bad arguments or missing required tool.

## Failure output — copy/paste to an LLM

On any failure both scripts print a single delimited block:

```
============================================================
BUILD FAILED — 2 stage(s) failed
toolchain: rustc 1.96.0 (…)
platform:  Linux 6.5.0 x86_64
cwd:       /path/to/oqci
============================================================

--- [clippy] exit=101 ---
   Compiling oqci v0.2.0 (…)
error: something specific here
…

--- [test] exit=101 ---
…

============================================================
END BUILD FAILURE — copy from 'BUILD FAILED' through this line
============================================================
```

Copy from `BUILD FAILED` through the closing line and paste into a chat —
every failed stage's full output is inside, with the toolchain and platform
context an assistant needs to diagnose.

## Installing `mdbook`

Optional; the docs site stage skips gracefully without it.

```bash
cargo install mdbook          # any platform with Rust
# or on macOS:
brew install mdbook
# or on Windows (Scoop/Chocolatey):
scoop install mdbook
```

## Visualization dev servers

`dev-visualization.*` starts `oqci-server` and the `frontend/` dev server
together and guarantees both are stopped together; `stop-visualization.*` is
a standalone safety net for when they weren't. See
[`docs/visualization.md`](../docs/visualization.md).

| Platform | Start | Stop | Shutdown mechanism |
|----------|-------|------|---------------------|
| Linux, macOS, Git Bash | [`dev-visualization.sh`](dev-visualization.sh) | [`stop-visualization.sh`](stop-visualization.sh) | Ctrl+C — `trap` kills both process groups |
| PowerShell 7+ | [`dev-visualization.ps1`](dev-visualization.ps1) | [`stop-visualization.ps1`](stop-visualization.ps1) | Ctrl+C — `try/finally` runs `taskkill /T` on both |
| `cmd.exe` | [`dev-visualization.bat`](dev-visualization.bat) | [`stop-visualization.bat`](stop-visualization.bat) | a keypress in the launcher window — batch cannot reliably trap Ctrl+C to run cleanup code, so this script doesn't pretend to; each server gets its own titled window instead |

```bash
scripts/dev-visualization.sh --backend simulator-nisq
scripts/stop-visualization.sh                          # emergency cleanup, any time
```

```powershell
scripts\dev-visualization.ps1 -Backend simulator-nisq
scripts\stop-visualization.ps1
```

```bat
scripts\dev-visualization.bat --backend simulator-nisq
scripts\stop-visualization.bat
```

All six accept the server and frontend ports as arguments (`--server-port`/
`--frontend-port`, or positionally for the `stop-*` scripts) if you're not
using the defaults (4173 / 5173).

## CI

The GitHub Actions workflow at
[`.github/workflows/docs.yml`](../.github/workflows/docs.yml) builds and
publishes the docs site to GitHub Pages on every push to `master`. It uses
the same `mdbook build docs` command these scripts do, so local and CI output
match exactly.
