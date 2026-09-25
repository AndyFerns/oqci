# Changelog

All notable changes to OQCI are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
The canonical version lives in the repo-root [`VERSION`](VERSION) file and is
mirrored in `Cargo.toml`; bump both with `scripts/bump-version.{sh,bat}`.

While the major version is `0`, the public API and IR contracts are unstable
and may change without a major bump (per SemVer §4).

## [0.4.0] - 2026-09-25

A companion, not a new compiler: `server/` and `frontend/` add a live,
browser-based view of the pipeline this version's own binaries already
compute — nothing about compilation itself changes here.

### Added

#### Live visualization (`server/`, `frontend/`)

- **`oqci-server`** (`server/`, new workspace member): a local WebSocket
  server that watches a `.qasm` file and, on every save, runs the real,
  unmodified `compile_named` — the exact entry point the CLI and Python SDK
  already go through — and streams the result as JSON.
- **Ground truth + self-validating replay** (`server/src/compile.rs`,
  `server/src/replay.rs`): every push is either read directly from that one
  real compile call, or reconstructed by calling the compiler's own public
  functions again — `PassManager`, `decompose()`, `RoutingStrategy::route()`,
  `repair_orientation()` — for pass-by-pass and swap/rule-by-rule detail the
  coarse schema doesn't retain. Every reconstruction is cross-checked against
  the real call before being shown; a mismatch reports itself `degraded`
  rather than displaying a reconstruction that might not match what the
  compiler actually did. `replay.rs`'s own tests include a negative control
  that feeds a deliberately wrong ground truth and asserts the check actually
  fires.
- **The only change to the `oqci` crate itself**: `src/cli/mod.rs`'s
  `mod snapshot;` becomes `pub mod snapshot;`, so the server can read the
  same `PipelineReport`-building functions (`instructions_of`, `graph_of`,
  `target_report`, …) the CLI's `--json` flag already uses, through the same
  public API every other consumer goes through. No `src/pass/`,
  `src/lowering/`, `src/compile.rs`, `src/target/`, `src/backend/`,
  `src/ir/`, or `src/frontend/` file changed.
- **`frontend/`** (React + TypeScript + Vite): a piano-roll circuit diagram
  laid out from the compiler's own `QcoCircuit::layers()` data, a
  `react-flow` view of the QCO-IR dependency graph, a pass timeline
  (scrub through each optimization pass), a lowering timeline (scrub through
  layout/routing/decomposition down to individual SWAPs and individual
  decomposition-rule firings, with an animated device-topology overlay), and
  cost/legality and provenance panels.
- `docs/visualization.md` — the wire contract, the replay technique in full,
  and why it needs no compiler changes. `server/README.md` and
  `frontend/README.md`.
- Cross-platform dev-server launcher/cleanup scripts:
  `scripts/dev-visualization.{sh,ps1,bat}` and
  `scripts/stop-visualization.{sh,ps1,bat}`. The `.sh`/`.ps1` pair genuinely
  traps Ctrl+C to stop both process trees together; `.bat` does not pretend
  cmd.exe can do that reliably, and uses a titled-window-plus-keypress
  pattern instead.

### Changed

- `scripts/bump-version.{sh,bat}` now also sync `server/Cargo.toml`'s
  `[package]` version, alongside the root and `python/` manifests they
  already kept in step.

## [0.3.0] - 2026-09-21

Two phases of work land together here: the **target model**, which was
pending, and the **backend** built on top of it. The backend sections come
first because they are what the version is for; the target-model sections
follow, unchanged from when they were written.

### Added (backend)

#### Target lowering (`src/lowering/`)

- **Qubit mapping** (§8.6) — `Layout` is an injective logical-to-physical
  assignment with both directions kept in step, and `LayoutStrategy` has two
  implementations: `TrivialLayout` (logical *n* on physical *n*, the
  assumption `check` always made, now named) and `DenseLayout`, which seats
  interacting qubits near each other. Layout choice can only change the SWAP
  count, never whether a circuit is correct.
- **Routing** (§8.7) — `ShortestPathRouter` walks the instruction list in
  program order, deletes nothing, reorders nothing, and only *inserts* `Swap`s.
  That one sentence discharges §33.14: no operation can be optimized away
  across a measurement barrier if nothing ever moves past anything.
  Deterministic, with no lookahead; the extra SWAPs relative to a SABRE-style
  router are a documented quality gap rather than a hidden one.
- **Basis decomposition** (§8.8) — a rule model covering every field Stage D
  §5 requires, with 18 verified built-in rules. `RuleSet::new` proves the rule
  graph acyclic before any circuit is touched, which makes termination a
  theorem rather than an iteration cap.
- **The lowering schedule** — arity reduction, layout, routing, basis
  decomposition, orientation repair, single-qubit cleanup, verify. Two of
  those orderings are load-bearing, and `docs/lowering.md` explains both.
- `lower` re-runs `check` on its own output and refuses to return a circuit
  the target rejects. It either returns something the target model itself
  certifies, or an error naming what it could not fix.

#### Backend contract (`src/backend/`)

- The `Backend` trait, with target lowering, validation, execution
  preparation and execution as four separately-failable stages (Stage C §3,
  §10). Backend selection is a registry lookup, so the compiler core contains
  no vendor branching (Stage C §6).
- `Executable` — a structured operation list, deliberately **not** QIR
  (Stage C §5 forbids treating emitted QIR as an execution guarantee) and not
  OpenQASM 3 (its Qiskit loader is a separate package this project does not
  depend on, and a third-party parser between "verified" and "runs" is a
  trust-path problem).
- `Provenance` covering every field Stage C §9 and Stage E §9 list, including
  the git commit, captured at build time by a new `build.rs` that degrades to
  `"unknown"` outside a checkout. Compilation, submission and execution
  durations are three independent fields, never derived from one another
  (Stage C §8).
- Three backends: `simulator`, `simulator-nisq`, and `ibm-illustrative` — the
  last of which is **synthetic and describes no real device**.
- **No backend executes in this process.** Each returns a typed
  `ExecutionNotAvailableInProcess` naming where execution actually happens.
  For IBM that is because `qiskit-ibm-runtime` is absent, no credentials
  exist, and §33.4 forbids writing a vendor API from memory; **no claim of IBM
  hardware executability is made** (§33.12). For the simulator it is because
  the project's non-goals rule out writing one.

#### Compiler orchestration (`src/compile.rs`)

- `compile` performs §6's pipeline end to end, and is now the single path from
  source text to artifacts. The CLI and the Python SDK both go through it, so
  the numbers a user sees are the numbers the compiler computed.

#### CLI

- `oqci lower` (with `--backend`, `--layout`, `--no-route`, `--no-decompose`),
  `oqci prepare` (with `--shots`, `--seed`, `-o`), and `oqci backends`.

#### Python SDK (`python/`)

- A pure-Python `oqci` package beside the compiled extension, covering §17's
  list: circuit import, compiler invocation, configuration, backend selection,
  analysis and result access.
- `oqci.backends.aer` executes a prepared executable on Qiskit Aer (§15.1),
  replaying it by direct `QuantumCircuit` method call rather than through a
  text format. Noise models are accepted from the caller and **never
  invented** — noise policy belongs to Stage G (§33.15).

#### Verification

- `tests/lowering_equivalence.rs` — the central property. For generated
  circuits on generated devices, lowering either refuses with an error the
  test independently confirms, or produces a circuit that is semantically
  equivalent modulo the final layout, legal, and deterministic. Equivalence is
  an isometry comparison; comparing full unitaries would fail on *correct*
  routing, and the module docs give the counterexample.
- Coverage floors assert the generators actually reach SWAP insertion,
  orientation repair, non-involutive layouts and wide gates. Without them a
  property suite can be green while exercising nothing.
- `python/tests/test_rules.py` re-checks every decomposition rule against
  Qiskit's `quantum_info.Operator`, deriving the exactness column rather than
  trusting it. Two independent implementations of gate semantics now have to
  agree on every run.
- `python/tests/test_aer.py` compiles and executes on Aer, checking measured
  distributions — the first tests in the project that check OQCI's output
  against something other than OQCI.
- Evidence the suite can fail: dropping one `H` from the `Cx` orientation
  repair produced an overlap of `1.3e-15` where `8` was required. Restoring it
  returned the suite to green.

### Changed (backend)

- **`Pass::run` takes a `&PassContext`**, carrying an optional target profile
  and cost model. The five existing passes ignore it. This closes Stage E exit
  criterion 3 — optimization *can* consult backend-defined costs — while
  keeping Stage E §8's rule that target awareness must not make every pass
  backend-specific. A breaking change for anyone implementing `Pass`.
- `PassManager::run` takes the same context. Callers pass
  `&PassContext::none()` explicitly, so "ran without target information" is
  visible at the call site rather than implied by an absent argument.
- `BasisProfile` now derives `Deserialize` **through the builder**, so a
  target description read from outside the compiler is validated exactly as a
  constructed one is. This is what lets an IBM target be supplied rather than
  hard-coded (§9.2).
- `builtin::linear_nisq` names 17 decomposition rules instead of two. It could
  not previously be lowered to: the first `Swap` routing inserted would have
  had no rule, and nothing would have noticed until a circuit failed.
- The compiled extension moved to `oqci._native`, as maturin's mixed layout
  requires. `import oqci_native` still works, through a shim.
- `Topology` gained `CouplingMode` and path queries. "Are these two qubits
  connected?" turns out to be three different questions, and conflating them
  is the entire bug class the enum exists to prevent.

### Fixed (backend)

- **`check` reported a symbolic parameter as legal** whenever the target
  declared no domain for that operation. The `Param::Symbol` arm sat behind
  the `parameter_constraint` lookup, so `rz(theta)` against
  `builtin::linear_nisq` — which declares no constraints — passed validation
  despite being unexecutable on every backend. Found while making `check` the
  postcondition oracle for lowering: an oracle with a hole in it certifies
  circuits that cannot run.
- Routing now searches for a path *around* measured wires rather than
  filtering a single best path against them. On a ring two paths can be
  equally short, and discarding the first because it crossed a measured wire
  refused circuits the second handled perfectly well.

### Added (target model)

The **target model**: how a backend describes what it accepts and what it
finds expensive. Target *description* only — mapping, routing, basis
decomposition and execution remain absent, and are the next roadmap step.


#### `SX` / `SXdg` in the registered gate set

- Two new `GateKind` variants, under an explicit architecture decision
  ([`docs/architecture_decision_sx_basis_gate.md`](docs/architecture_decision_sx_basis_gate.md)),
  since Stage A §4 locks the enum as closed "until an explicit architecture
  decision changes this". `sx` is a native one-qubit operation on IBM-style
  hardware, and routing it through `Opaque` would have left exactly the
  circuits the target research depends on unoptimizable and unverifiable.
- `SX`↔`SXdg` join the cancellation pass's mutual-inverse table. `SX` is
  **not** self-inverse (`SX; SX` is `X`), and a test guards that.
- Both frontends resolve `sx`/`sxdg` through the shared gate table; both
  lower to declared extended QIR intrinsics; both are covered by the
  state-vector equivalence harness.

#### Target model (`src/target/`)

- `BasisProfile` — the formal target description, covering every field
  `final-deliverables-spec.md` §11 requires. Built through a validating
  builder and `Serialize`, so a profile used in an experiment can be
  snapshotted (Stage D §8).
- `Topology` — physical qubits and **directed** couplings. Stage D §7
  forbids assuming an undirected edge means both orderings are native, so
  symmetric links declare both directions explicitly.
- `check` — validates a circuit against a profile, reporting *every*
  violation rather than the first, and repairing nothing (Stage D §4 keeps
  description, lowering and routing separate).
- `Cost` / `CostModel` — structured cost that never collapses to a scalar
  (Stage E §7). `WeightedCostModel`'s weights are explicit configuration
  reported through `configuration()`, not literals buried in optimizer code
  (Stage E §6). `estimated_duration`/`estimated_error` stay `None` rather
  than becoming fabricated zeros.
- `builtin` — `ideal-simulator` and `linear-nisq`, both **synthetic**.
  Neither describes real hardware; §9.2 forbids hard-coding a device into the
  compiler core, and a profile asserting uninvented error rates would be a
  fabricated record.

#### CLI

- `oqci targets` lists the built-in profiles.
- `--target ID` on `compile`, `optimize` and `analyze` appends a legality
  report and a cost breakdown; on `optimize` the *optimized* circuit is
  checked, since that is what would be submitted. `--json` carries all of it.
- A scalar score is never printed without the weights that produced it.

### Changed (target model)

- `BasisProfile::supports_operation` answers for `"measure"`/`"reset"` from
  `MeasurementSupport` rather than the basis set, so a profile cannot
  contradict itself and have legality and cost disagree.
- `src/ir/qir.rs`'s doc comment now lists `sx`/`sxdg` among the extended
  intrinsics.

### Not included

Five items were deferred when the target model landed. Four of them are
implemented above — qubit mapping, routing, basis decomposition and target
context on the `Pass` trait — and the fifth remains open, along with what the
backend work deferred in turn.

- **Live IBM submission and result retrieval.** Everything up to a validated
  executable and its provenance is implemented and tested; the submission call
  is not. `qiskit-ibm-runtime` is not installed, no credentials exist, and
  §33.4 forbids implementing a vendor API from memory when the SDK cannot be
  checked. Writing untested code on the path between a verified circuit and
  real hardware would be worse than an explicit boundary, so `execute` returns
  a typed error naming it. **No claim of IBM hardware executability is made**
  (§33.12).
- **Per-operation cost and error/noise metadata in profiles** (Stage D §2).
  Still has no legitimate value to hold: both built-in profiles are synthetic,
  and a profile asserting error rates nobody measured would be a fabricated
  record. This lands with a real backend adapter.
- **Cirq and CUDA-Q execution adapters** (§15.2). The backend contract is
  SDK-agnostic and the executable representation is not Qiskit-specific, so
  these are adapter work rather than compiler work.
- **Lookahead routing.** The deterministic shortest-path router inserts more
  SWAPs than a SABRE-style one would. The gap is real and unmeasured;
  `docs/lowering.md` states it rather than leaving it to be discovered.
- **Cost-model-guided lowering.** The cost model evaluates lowered circuits
  but does not yet steer any decision inside lowering.
- **Benchmark infrastructure** (§20) and the Stage G experimental protocol,
  which remains unlocked.

## [0.2.0] - 2026-09-16

The **frontend layer** (OpenQASM 3 and Qiskit ingestion plus the Stage F
parameter work they depend on), the **pass manager and target-independent
optimization passes**, and the **`oqci` CLI** for inspecting every stage.
QIR emission remains the output boundary — there is still **no target model**
and **no backend execution**.

### Added

#### Pass manager and optimization passes (`src/pass/`)

- `Pass` / `PassManager`: explicit registration and ordering, enable/disable
  selection for ablation runs, deterministic execution, per-pass metadata,
  and error propagation naming the offending pass
  (`final-deliverables-spec.md` §7).
- Passes are `Circuit → Circuit` and replay their result through
  `CircuitBuilder`, so **a pass cannot emit a circuit that violates a QC-IR
  invariant** — it fails loudly instead.
- `canonicalize`: removes `I` gates and exact-zero rotations. Nothing that
  would require asserting a global-phase-sensitive identity.
- `gate-cancellation`: removes adjacent inverse pairs, where "adjacent" is a
  QCO-IR question (`H q0; H q1; H q0` cancels; `X q0; measure q0; X q0` does
  not). `U` and `Opaque` are deliberately never cancelled.
- `rotation-merge`: `Rz(a); Rz(b) → Rz(a+b)` for `Rx`/`Ry`/`Rz`/`P`, only
  when both parameters are concrete. This is the whole of OQCI's gate
  fusion; arbitrary unitary synthesis is not implemented and is not claimed.
- `schedule`: reports ASAP layering, depth and parallel width. Analysis
  only — it never reorders.
- Default pipeline `canonicalize → gate-cancellation → rotation-merge →
  canonicalize → schedule`, with the ordering rationale documented.

#### Analysis (`src/analysis/`)

- `analyze` → `ResourceReport`: gate counts, measurement/reset counts,
  depth, parallel width (§13.1–13.3). Target-native counts and routing
  overhead are absent rather than reported as zero — they need a target
  profile.
- `diff_circuits` → `CircuitDiff`: LCS alignment of two instruction lists,
  computed by comparing circuits rather than trusting a pass's own account.
- `QcoCircuit::layers` / `depth` / `max_parallel_width`.
- This module is the single source of truth for measurement: the pass
  manager's bookkeeping and the CLI's output call the same functions, so
  they cannot disagree.

#### Pass-correctness harness (`tests/pass_equivalence.rs`)

- `proptest`-generated circuits run through each pass, asserting the state
  vector is unchanged up to global phase.
- Semantics come from an independent dense-matrix simulator
  (`tests/support/statevector.rs`; test-only, `num-complex` and `proptest`
  are dev-dependencies), so a mis-signed rewrite cannot hide behind a
  matching mistake in the check.
- Verified to actually fail when a rule is broken — inverting the sign in
  rotation-merge's addition fails four tests including the randomized one.

#### The `oqci` CLI (`src/cli/`)

- `compile`, `optimize`, `analyze`, `watch`, `passes`. `watch` re-runs the
  pipeline on every save, watching the directory (so editor save-and-rename
  works) and staying alive through parse errors.
- `--diff` renders a before/after instruction diff; `--disable ID` runs an
  ablation; `--bind NAME=VALUE` supplies symbolic parameters.
- `--json` emits a `PipelineReport` — the schema a future dashboard
  consumes. IR types stay serde-free; the schema is CLI-layer view types.
- Per §19 the CLI duplicates no compiler logic: `src/cli/pipeline.rs` is the
  only module that calls the compiler, and every metric it prints came from
  `analysis`.
- `examples/{bell,ghz3,parameterized}.qasm` as runnable starting points.
- `src/main.rs` is now a thin entry point; the old hardcoded Bell demo is
  replaced by `oqci compile examples/bell.qasm`.

### Added — frontends (earlier in this cycle)

#### Symbolic gate parameters (`src/ir/param.rs`, `src/ir/bind.rs`)

Stage F (`docs/core_architecture/stage-f-static-parameterized-circuit-scope.md`)
requires QC-IR to express static circuits with **symbolic or numeric**
rotation parameters, so VQE-style ansatzes are representable without admitting
dynamic control flow.

- `Param`: a gate parameter that is either `Concrete(Angle)` or
  `Symbol(String)`. `GateKind`'s parameter-bearing variants (`Rx`, `Ry`, `Rz`,
  `P`, `U`, `Opaque`) now carry `Param`. `Angle` remains the concrete type and
  converts into `Param`, so existing concrete call sites are unchanged.
- A circuit with unbound symbols is **valid** QC-IR; only its symbol *names*
  are validated (new invariant I8, `IrError::EmptyParameterSymbol`).
- `bind_parameters(&circuit, &bindings)`: the explicit symbolic → concrete
  step Stage F §8 requires. Structure is preserved exactly; the result is
  re-validated, so a `NaN` binding is rejected like a literal one.
- `emit_qir` now reports `IrError::UnboundParameter` rather than lowering a
  circuit whose angles are not yet known — it never invents a value.
- `Circuit::parameters()` / `Circuit::is_concrete()`.

#### Frontend contract (`src/frontend/`)

- `FrontendError`: one error type for every frontend, distinguishing syntax
  (with line/column), semantic, unsupported-construct, parameter-arity, and
  propagated `IrError` failures.
- `map_gate`: a **single** source-name → `GateKind` table shared by both
  frontends, so they agree on what `rz` or `u2` means by construction.
  Unrecognised names become `GateKind::Opaque`; the enum is never grown to
  accommodate a source language.

#### OpenQASM 3 frontend (`src/frontend/openqasm/`)

- `parse_openqasm3` / `parse_openqasm3_named`, built from a hand-written
  lexer, recursive-descent parser, and a translator resolving OpenQASM's
  named/indexed registers onto QC-IR's flat space. No new dependencies.
- Supported subset documented precisely in `docs/openqasm_frontend.md`:
  declarations, `input` parameters, gate calls with broadcast, both `measure`
  spellings, `reset`, comments, and constant-folded angle arithmetic.
- Out-of-subset constructs — `if`/`for`/`while`, `def`/`gate` blocks,
  `output`, `barrier`, OpenQASM 2 `qreg`/`creg`, compound symbolic
  expressions — are **refused by name**, never skipped.

#### Qiskit adapter (`src/frontend/qiskit/`, `python/`)

- `QiskitCircuitIr`: a vendor-neutral handoff struct, keeping Qiskit types out
  of QC-IR entirely.
- `translate`: all adapter logic — gate mapping, operand order, measurement
  destinations, parameters — in pure Rust, tested with no Python present.
- A thin PyO3 boundary (`oqci-python`, module `oqci_native`) exposing
  `qiskit_to_qir`, `qiskit_parameters` and `qasm3_to_qir`, verified against
  **Qiskit 2.5.2** via `python/tests/test_adapter.py`. The repo is now a Cargo
  workspace so this crate builds alongside the core.
- Documented limitations: compound `ParameterExpression`s and Qiskit control
  flow are refused; `barrier`/`delay` are dropped (recorded, not silent).

#### Tests

- 330 Rust tests (218 unit + 112 integration across `tests/*.rs`), 10
  doctests, and 15 Python tests against real Qiskit circuits.
- The frontends are pinned by **equivalence**: a parsed Bell state, GHZ-3 and
  folded-angle rotation must emit byte-identical QIR to the hand-built
  circuit, and both frontends must agree on the same program. That is what
  makes them a mapping onto the existing IR rather than a second definition
  of it.

#### Documentation

- `docs/gate_mapping.md`, `docs/openqasm_frontend.md`,
  `docs/qiskit_adapter.md`, `python/README.md`; `docs/ir_spec.md` extended
  with `Param`, invariant I8, and parameter binding.

### Changed

- `GateKind`'s parameter-bearing variants take `Param` instead of `Angle`, and
  `GateKind::params()` returns `Vec<Param>`. Call sites constructing these
  variants directly need `Param::concrete(x)` in place of `Angle::new(x)`;
  `CircuitBuilder::rx/ry/rz` accept both.
- `VERSION` reconciled with `Cargo.toml` at `0.1.0` (they had drifted).
- New dependencies: `clap`, `notify`, `serde`, `serde_json` for the CLI;
  `proptest` and `num-complex` as dev-dependencies for the equivalence
  harness only.
- The QIR module header no longer claims "Phase 0", which stopped being true.

### Not included (planned for later phases)

- Target/basis profiles, qubit mapping, routing, and basis decomposition
  (`final-deliverables-spec.md` §8.6–8.8, §11) — blocked on the Stage D
  target model, which does not exist yet.
- Compiler orchestration layer, backend execution (simulators, hardware),
  and cost models — Phase 3+.
- General gate fusion beyond additive-parameter merging; a fixed-point pass
  scheduler; pass plugins.
- A Python compiler/analysis/backend API — the current bindings cover only
  the frontend → QC-IR → QIR path (`python/README.md`).

## [0.0.1] - 2026-08-09

First substantive drop: the **Phase 0 IR core**. This establishes the
intermediate representation, the conversions between its two levels, and QIR
emission. There are deliberately **no frontends**, **no backend execution**, and
**no optimization passes** yet — the scope boundary is the IR itself. The
version is `0.0.1` (not `0.1.0`) because Phase 0 is the foundation only, before
the first usable milestone; Phase 1 will bump the minor.

### Added

#### QC-IR — imperative circuit IR (`src/ir/qc.rs`, `src/ir/types.rs`)

- `Circuit`: an immutable, validated circuit over a fixed qubit register and a
  fixed classical register, holding an ordered instruction list.
- `Instruction`: three variants — `Gate { kind, qubits }`, `Measure { qubit,
  target }`, `Reset { qubit }` — with `control()`/`target()` role accessors for
  controlled gates.
- `CircuitBuilder`: the sole construction path (mirrors MLIR's `OpBuilder`).
  Register allocation returns typed ids; instruction methods are infallible and
  chainable; **all** validation is deferred to `build()`.
- `GateKind`: a closed enum of registered gates — `I, X, Y, Z, H, S, Sdg, T,
  Tdg, Rx, Ry, Rz, P, U, Cx, Cy, Cz, Swap, Ccx` — plus an `Opaque { name,
  params }` escape hatch. Parameters (angles) live here; qubit operands live on
  the instruction, matching the MLIR attribute-vs-operand split.
- Newtype value types: `QubitId(u32)`, `ClbitId(u32)`, and a dedicated
  `Angle(f64)` (radians, not normalized), so the eventual MLIR mapping is
  mechanical.
- Seven validation invariants enforced by `build()`, each with a dedicated error
  variant: qubit/clbit range, gate arity, no duplicate operand within a gate,
  non-empty opaque name and operands, and finite angles.

#### QCO-IR — optimization IR / dependency graph (`src/ir/qco.rs`)

- `QcoCircuit`: the circuit as a directed acyclic graph (backed by `petgraph`),
  with explicit `Input`/`Output` boundary nodes threading every qubit and
  classical wire.
- Dependency edges classified as `DepKind::Data` (value flow between unitaries)
  or `DepKind::Control` (ordering barrier out of a state-collapsing measure or
  reset).
- Deterministic topological traversal (`topological_ops`) via Kahn's algorithm
  with ties broken by program index, plus `linearize()` for canonical program
  order.

#### QC-IR → QCO-IR conversion (`src/ir/convert.rs`)

- `qc_to_qco`: a single deterministic wire-threading pass that extracts data and
  control dependencies. Documented as semantics-preserving (proof sketch in
  `docs/ir_spec.md`): every topological order is an equivalent execution, and
  the canonical order reproduces the original program exactly.

#### QCO-IR → QIR lowering (`src/ir/qir.rs`)

- `emit_qir`: emits textual, LLVM-compatible QIR in the classic typed-pointer
  form (opaque `%Qubit`/`%Result`, `__quantum__qis__*` intrinsics,
  `__quantum__rt__result_record_output`, a single `entry_point` function with
  Base-Profile module flags).
- Gate → intrinsic mapping table; angles emitted as exact hexadecimal `double`
  literals; static qubit/result addressing via `inttoptr`.
- Gates outside the QIR standard set (`id, p, u, cy, swap, ccx`, and `Opaque`)
  are emitted as declared **extended** intrinsics (documented; decomposition is
  a future pass). Mid-circuit measurement is emitted in program order.

#### Error model (`src/ir/error.rs`)

- `IrError`: a single `thiserror`-based, `#[non_exhaustive]` error type covering
  validation, conversion, and lowering. No IR routine panics or `unwrap`s on
  malformed input.

#### MLIR readiness (`src/ir/mlir_compat.rs`)

- An intentionally empty module marking the Phase 2 MLIR integration seam, so the
  boundary exists before the port rather than being bolted on later.

#### Tests

- 39 tests total: 25 unit + 12 end-to-end integration (`tests/pipeline.rs`,
  covering identity/empty, Bell, GHZ-3, mid-circuit measurement, and one
  malformed circuit per validation rule) + 2 doctests.

#### Documentation (`docs/`)

- `ir_spec.md` — normative QC-IR + QCO-IR spec, invariants, operational
  semantics, and the conversion's semantics-preservation proof.
- `mlir_dialect.md` — the `quantum` MLIR dialect spec, complete op↔Rust
  correspondence table, Phase 2 integration path, and implied TableGen skeleton.
- `qir_lowering.md` — lowering rules, op→intrinsic table, and conformance
  caveats.
- `architecture_decision_no_frontend.md` and
  `architecture_decision_mlir_phase2.md` — the two ADRs (why no frontend yet;
  the binding constraints C1–C8 that keep the MLIR port cheap).
- `README.md` — docs index.
- mdBook static site (`book.toml`, `SUMMARY.md`, `theme/custom.css`) publishing
  the above, with a GitHub Pages deploy workflow.

#### Tooling & project setup

- Cross-platform build scripts (`scripts/build.{sh,bat}`) running the full check
  suite (fmt, clippy, test, doctest, rustdoc, mdbook) with copy-pasteable,
  LLM-friendly failure output.
- Version bump scripts (`scripts/bump-version.{sh,bat}`) driving the `VERSION`
  file and keeping `Cargo.toml` in sync.
- Cargo manifest with `thiserror` + `petgraph`, library + binary targets, and a
  thin demo binary; `.gitignore` and `.gitattributes` hygiene.

### Not included (planned for later phases)

- Frontend parsers / SDK adapters (OpenQASM, Qiskit, Cirq, CUDA-Q) — Phase 1.
- Optimization passes (cancellation, fusion, rotation merging, scheduling,
  routing) — Phase 3.
- MLIR dialect implementation (TableGen/C++, pass manager, conversion
  framework) — Phase 2.
- Backend execution (simulators, hardware) — beyond the QIR emission boundary.
- Python bindings — only empty PyO3 placeholders exist under `python/`.

[Unreleased]: https://github.com/AndyFerns/oqci/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/AndyFerns/oqci/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/AndyFerns/oqci/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/AndyFerns/oqci/compare/v0.0.1...v0.2.0
[0.0.1]: https://github.com/AndyFerns/oqci/releases/tag/v0.0.1
