# Changelog

All notable changes to OQCI are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
The canonical version lives in the repo-root [`VERSION`](VERSION) file and is
mirrored in `Cargo.toml`; bump both with `scripts/bump-version.{sh,bat}`.

While the major version is `0`, the public API and IR contracts are unstable
and may change without a major bump (per SemVer §4).

## [Unreleased]

_Nothing yet._

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

[Unreleased]: https://github.com/AndyFerns/oqci/compare/v0.0.1...HEAD
[0.0.1]: https://github.com/AndyFerns/oqci/releases/tag/v0.0.1
