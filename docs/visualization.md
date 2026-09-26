# Live Visualization — A Read-Only Companion, Not a Second Compiler

Status: normative
Implemented by: `server/`, `frontend/`
Verified by: `server/src/replay.rs`'s own test module (cross-validation
against real `compile_named`/`lower()` calls)
Entry points: `oqci-server` (binary), `oqci::cli::snapshot` (the JSON
contract it reads)

A browser-based, live-updating view of what the compiler actually did, for a
real program on disk — the same information `oqci --json` prints, plus a
step-by-step replay of the pass pipeline and target lowering, animated.

## The one rule

**The visualization server never changes what the compiler does, and never
contains a second implementation of any compiler decision.** Every fact it
shows is either read directly from a real `compile_named`/`lower()` call
(the *ground truth*), or reconstructed by calling the compiler's own public
functions — `PassManager`, `decompose()`, `RoutingStrategy::route()`,
`repair_orientation()` — one more time each, from outside, and then
cross-checked against that ground truth before being shown at all.

This is not a convention someone has to remember. It is structural:

- `server/` is a separate Cargo workspace member that depends on the `oqci`
  library crate like any other caller (`Cargo.toml`'s `[workspace] members`,
  the same pattern `python/` already uses). It cannot reach a private item.
- The **only** change made to the compiler crate itself to build this
  feature is a single visibility flip — `src/cli/mod.rs`: `mod snapshot;` →
  `pub mod snapshot;`. Every type and function in `src/cli/snapshot.rs` was
  already `pub`; only the module wrapping it was private. Nothing in
  `src/pass/`, `src/lowering/`, `src/compile.rs`, `src/target/`,
  `src/backend/`, `src/ir/`, `src/frontend/`, or `src/analysis/` was
  touched.
- `server/src/replay.rs` documents, for every reconstruction it performs,
  which real public function produced the underlying decision and why the
  reconstruction is provably equivalent to calling that function once on the
  whole input (see [Fine-grained replay](#fine-grained-replay-and-why-its-safe)
  below). It is not a parallel pass manager or a parallel rule engine; it is
  the same one, invoked more times.

## Architecture

```text
oqci (root lib crate)                     — unmodified except the one
  compile_named, lower, decompose,          visibility flip above
  repair_orientation, route, PassManager,
  analyze, diff_circuits, target::check
        │
        │ public API, called as-is
        ▼
server/ (oqci-server, axum + tokio)
  compile.rs  — ground truth: one real compile_named call per request,
                assembled into a PipelineReport via oqci::cli::snapshot,
                mirroring src/cli/pipeline.rs's own assembly rather than
                reimplementing it.
  replay.rs   — fine-grained reconstruction: calls the same pass/lowering
                functions again, more finely, then cross-validates the
                result against the ground truth call above. Reports itself
                `degraded` rather than showing a possibly-wrong replay if
                the cross-check ever fails.
  watch.rs    — independent, server-owned file watcher (notify crate,
                debounced, directory-not-file — the same technique
                `oqci watch` uses, reimplemented standalone here since
                watching a filesystem path is not compiler logic).
  ws.rs       — the /api/watch WebSocket: recompiles on every change,
                pushes ground truth + replay as JSON.
        │
        │ WebSocket (JSON)
        ▼
frontend/ (React + TypeScript, Vite)
  Circuit diagram, QCO-IR dependency graph, pass timeline (scrub through
  each optimization pass), lowering timeline (scrub through layout/routing/
  decomposition, down to individual SWAPs and individual rule firings),
  device topology, cost/legality, provenance.
```

## Fine-grained replay, and why it's safe

`PassRecord` (per-pass) and `Lowered.steps` (the seven-step lowering
schedule) already report *metrics* for every pass and stage — that part of
`PipelineReport` needs no reconstruction at all. What the coarse schema does
not retain is the *circuit* at each intermediate point, or which specific
SWAP or decomposition rule fired where — those are discarded as soon as the
next pass or lowering phase overwrites `current` (see `src/pass/mod.rs`'s
`PassManager::run` and `src/lowering/mod.rs`'s `lower`).

Rather than changing those functions to retain more, `server/src/replay.rs`
calls the same underlying, already-public building blocks again, once per
intermediate point wanted, entirely from outside the compiler crate:

| Reconstruction | Real function(s) called, unmodified | Why it's equivalent to the real call |
|---|---|---|
| Circuit after pass *i* | `PassManager::new().register(...)` for a *prefix* of `PassManager::default_pipeline()`'s own published, tested order, then `.run()` | `PassManager::run` is already the function that produces `current`; running a shorter, identical prefix of the same registered `Pass` implementations reproduces exactly the state the real run passed through, and is not a different algorithm. |
| Each SWAP inserted during routing | `RoutingStrategy::route()` once (the real router), then a lock-step walk against its input | Routing is documented and tested as *insertion-only, in program order* (`docs/lowering.md`) — it never deletes or reorders an original instruction. That invariant is exactly what makes a simple lock-step walk correct: every point where the two lists disagree **must** be an inserted `Swap`, not something the walk could misread as one. |
| Each decomposition rule firing | `decompose()` (the real rule engine) called once per *source instruction* instead of once for the whole list | A decomposition rule may only permute the operands it was given and can never introduce a new one (`RuleSet`'s own tested I3 invariant, `docs/lowering.md`) — so decomposition has no cross-instruction interaction, and splitting one call into many produces byte-identical output. This equivalence is not merely argued; `replay.rs` checks it on every call by concatenating the per-instruction results and comparing them to one whole-list call. |

Every one of these reconstructions is then compared, in full, against the
one real `compile_named`/`lower()` call the request also made — final
circuit, swap count, rule set, legality. If anything disagrees, the replay
reports `degraded: true` with a reason, and the frontend falls back to the
always-correct coarse summary rather than displaying a reconstruction that
might not match what the compiler actually did. `server/src/replay.rs`'s own
test module includes a negative control
(`a_deliberately_wrong_ground_truth_is_reported_degraded_not_silently_accepted`)
proving this check can actually fail, not just always pass.

## Wire contract

`WS /api/watch`. The first message from the client selects a mode:

```json
{ "kind": "watch_file", "path": "examples/bell.qasm" }
{ "kind": "compile_source", "source": "OPENQASM 3.0; ...", "name": "inline" }
```

`watch_file` resolves `path` relative to (and confined within) the server's
`--root`, starts a filesystem watcher, and recompiles on every save.
`compile_source` compiles once immediately and again each time the client
sends another `compile_source` message on the same connection — the shape a
future hosted, no-filesystem playground would use; it needed no
implementation beyond what `watch_file`'s compile step already does, so it
is implemented now rather than stubbed.

The server pushes one message per compile cycle, tagged and sequence-numbered
so a client that keeps editing can discard a late reply to a stale compile:

```json
{ "kind": "pipeline_report",  "sequence": 3, "report": { ... PipelineReport, unmodified from src/cli/snapshot.rs ... } }
{ "kind": "pass_replay",      "sequence": 3, "replay": { "steps": [...], "degraded": false } }
{ "kind": "lowering_replay",  "sequence": 3, "replay": { "steps": [...], "degraded": false } }
{ "kind": "compile_error",    "sequence": 3, "message": "..." }
```

`pipeline_report.report` is byte-for-byte the same schema `oqci ... --json`
prints (`docs/cli.md`) — nothing about the ground-truth JSON shape is
server-specific. `pass_replay`/`lowering_replay` are server-owned additions
(`server/src/events.rs`): `PassStepEvent` (per pass: id, before/after
metrics and circuits), and `LoweringStepReplay` (per lowering step: the
circuit at that point, plus `swap_events`/`rule_firings` inside the routing
and decomposition steps).

## Running it locally

```bash
cargo run -p oqci-server -- --root . --backend simulator-nisq
```

```bash
cd frontend && npm install && npm run dev
```

Open the dev server's URL, enter a path relative to `--root` (e.g.
`examples/ghz3.qasm`), and click "watch". Editing and saving the file
updates the browser live — the same directory-watch, debounce, and
rename-tolerant behavior `oqci watch` already has, reimplemented
independently in `server/src/watch.rs` rather than by depending on or
modifying `src/cli/watch.rs`.

`--backend` selects which target every watched file is lowered against
(anything `oqci backends` lists). `--static-dir` optionally serves a built
frontend (`npm run build`'s `dist/`) from the same process, so the whole
tool can run as one binary.

## What is explicitly not here

Per the project-wide rule that a feature is not implemented merely because a
module or a plan names it:

- **No authentication, rate-limiting, or sandboxing.** This is a local
  dev-companion tool: `--root` confines `watch_file` requests to one
  directory tree, and that is the only input validation performed. A hosted,
  multi-user playground built on the `compile_source` arm above would need
  all three before being exposed to untrusted input, and none of them exist.
- **No caching or incremental compilation.** Every push is a full,
  independent `compile_named` call plus independent replay calls, exactly as
  `oqci watch` is a full re-run per save (`docs/cli.md`). This compiler
  compiles fast enough on real hardware that this has not needed optimizing.
- **No claim about IBM hardware.** The visualization shows exactly what the
  compiler itself can verify — see `docs/backend_contract.md` for what
  "prepared, not submitted" means and why.
