# `oqci-server`

A local, read-only visualization companion for the compiler — a WebSocket
server that watches a `.qasm` file, recompiles it on every save through the
real, unmodified `oqci` library, and streams the result (plus a step-by-step
replay of the pass pipeline and target lowering) to the `frontend/` app.

See [`docs/visualization.md`](../docs/visualization.md) for the full
contract, the replay technique, and why it needs no changes to the compiler
crate beyond one visibility flip. The short version: this crate is a caller
of `oqci`, exactly like `python/` or the `oqci` CLI binary — it never decides
anything about compilation.

## Layout

| Path | What it is |
|---|---|
| `src/compile.rs` | The ground-truth path: one real `compile_named` call per request, assembled into a `PipelineReport` via `oqci::cli::snapshot`. |
| `src/replay.rs` | Fine-grained, self-validating reconstruction of per-pass and per-lowering-step state, by calling the compiler's own public functions again. Its own test module cross-validates every reconstruction against a real compile. |
| `src/watch.rs` | An independent file watcher (directory-not-file, debounced) — the same technique `oqci watch` uses, reimplemented standalone here. |
| `src/ws.rs` | The `/api/watch` WebSocket handler: wiring only, no compiler logic. |
| `src/events.rs` | The server-owned JSON types (`PassReplay`, `LoweringReplay`, the message envelope) — additive to, never a fork of, `oqci::cli::snapshot`. |
| `src/handlers.rs` | Trivial REST endpoints (`/api/health`, `/api/backends`, `/api/targets`). |

## Running

```bash
cargo run -p oqci-server -- --root . --backend simulator-nisq
```

| Flag | Default | Meaning |
|---|---|---|
| `--root` | `.` | Directory `watch_file` requests are resolved and confined within. |
| `--port` | `4173` | Port to listen on. |
| `--backend` | `simulator-nisq` | Backend every watched file is compiled for. Must be one `oqci backends` lists. |
| `--static-dir` | (none) | Serves a built frontend (`frontend/dist`) from the same process. |

## Testing

```bash
cargo test -p oqci-server
```

`replay.rs`'s tests are the load-bearing ones: they compile a real circuit,
run the replay, and assert it is not `degraded` — plus one negative control
that deliberately feeds a wrong ground truth and asserts the cross-validation
actually notices.
