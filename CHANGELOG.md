# Changelog

All notable changes to OQCI are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
The canonical version lives in the repo-root [`VERSION`](VERSION) file and is
mirrored in `Cargo.toml`; bump both with `scripts/bump-version.{sh,bat}`.

While the major version is `0`, the public API and IR contracts are unstable
and may change without a major bump (per SemVer §4).

## [Unreleased]

The **target model**: how a backend describes what it accepts and what it
finds expensive. Target *description* only — mapping, routing, basis
decomposition and execution remain absent, and are the next roadmap step.

### Added

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

### Changed

- `BasisProfile::supports_operation` answers for `"measure"`/`"reset"` from
  `MeasurementSupport` rather than the basis set, so a profile cannot
  contradict itself and have legality and cost disagree.
- `src/ir/qir.rs`'s doc comment now lists `sx`/`sxdg` among the extended
  intrinsics.

### Not included (deferred to the next roadmap step)

- Qubit mapping (§8.6), routing/SWAP insertion (§8.7) and basis decomposition
  (§8.8). The data they need now exists; the passes that consume it do not.
- Executable decomposition-rule data. Profiles record rule *identifiers*; the
  Stage D §5 model (source op, target sequence, parameter transformation,
  operand mapping, exactness) lands with the pass that executes it.
- Target context on the `Pass` trait, so Stage E §8 is not yet satisfied.
- Per-operation cost and error/noise metadata in profiles (Stage D §2), which
  has no legitimate value to hold until a real backend supplies it.
- Backend execution and result retrieval (§9, §10).

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

[Unreleased]: https://github.com/AndyFerns/oqci/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/AndyFerns/oqci/compare/v0.0.1...v0.2.0
[0.0.1]: https://github.com/AndyFerns/oqci/releases/tag/v0.0.1
