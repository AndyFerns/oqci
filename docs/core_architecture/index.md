# OQCI Source-of-Truth Documentation Index

Status: Active planning baseline
Last verified against repository: 2026-10-21
Repository: `https://github.com/AndyFerns/oqci`  
Branch reviewed: `feat/completion` (0.7.1)  
Current repository version: `0.7.1`

## Purpose

This directory is the authoritative implementation-planning index for the Open Quantum Compiler Infrastructure (OQCI) project.

The documents are intentionally split into architecture/design decisions and one consolidated final-deliverables specification. They are written to be usable by a human developer or an LLM coding agent without silently inventing architecture.

**Stage G is locked** (2026-10-20, by the project owner's approval of the 0.6.0 completion plan). Its purpose is the experimental benchmark matrix/protocol and hardware-test methodology; its values are recorded in its own document and mirrored in `benchmarks/configs/stage_g.toml`.

## Locked Architecture Decisions

| Stage | File | Decision | Status |
|---|---|---|---|
| — | [`framing-and-research-question.md`](framing-and-research-question.md) | OQCI is quantum-native compiler infrastructure first and LLVM/QIR-interoperable second; the research question every other decision serves. Takes precedence over older identity statements. | LOCKED |
| A | [`stage-a-rust-native-ir-foundation.md`](stage-a-rust-native-ir-foundation.md) | Rust-native IR first; stabilize QC-IR/QCO-IR before introducing heavy MLIR infrastructure. | LOCKED |
| B | [`stage-b-modular-mlir-boundary.md`](stage-b-modular-mlir-boundary.md) | MLIR is a modular/enabling infrastructure layer, not the project's primary research contribution. Integrate progressively after the Rust core is stable. | LOCKED |
| C | [`stage-c-explicit-backend-contract.md`](stage-c-explicit-backend-contract.md) | Backend execution is governed by an explicit OQCI backend contract; IBM gets a dedicated target-lowering stage. | LOCKED |
| D | [`stage-d-backend-specific-basis-profiles.md`](stage-d-backend-specific-basis-profiles.md) | Target-independent abstract gates are lowered through backend-specific basis profiles. | LOCKED |
| E | [`stage-e-backend-defined-cost-model.md`](stage-e-backend-defined-cost-model.md) | Optimization cost is supplied by the selected backend/target cost model. Preserve raw metrics; do not collapse the research evaluation to one scalar. | LOCKED |
| F | [`stage-f-static-parameterized-circuit-scope.md`](stage-f-static-parameterized-circuit-scope.md) | Main project supports static circuits plus symbolic/numeric parameterized rotations. Runtime-dependent dynamic control flow is deferred. | LOCKED |
| G | [`stage-g-benchmarking-protocol.md`](stage-g-benchmarking-protocol.md) | Experimental protocol. Primary axes: validation strength, determinism, interoperability; hardware experiments secondary. Every numeric parameter is decided in its §5. | LOCKED (2026-10-20) |

The binding IR constraints C1–C8 are recorded in the IR Contract ADR,
[`../architecture_decision_ir_contract.md`](../architecture_decision_ir_contract.md),
and carry the same weight as a locked decision: they may only be changed by an
explicit architecture decision that amends that ADR.

## Consolidated Deliverables Specification

[`final-deliverables-spec.md`](final-deliverables-spec.md)

This is the complete definition of what must eventually exist in the repository for the non-G project deliverables.

It covers:

- the current-state baseline;
- compiler architecture;
- IR requirements;
- frontend ingestion;
- pass infrastructure;
- core optimization passes;
- scheduling;
- target topology and mapping/routing;
- backend-specific lowering;
- QIR/LLVM emission;
- simulator integration;
- backend abstraction;
- Python integration;
- plugin/extension architecture;
- analysis and validation;
- parameterized-circuit semantics;
- testing and quality requirements;
- packaging and CI;
- documentation;
- benchmark infrastructure;
- research artifacts;
- final acceptance criteria;
- explicit non-goals and anti-hallucination constraints.

G-specific experimental values are intentionally omitted from the locked specification. The benchmark framework may be implemented before G is finalized, but exact experimental parameters must not be invented by an implementation agent.

## Source Hierarchy

When sources disagree, use this precedence order:

1. Explicitly locked decisions in this directory.
2. The current repository implementation and its existing ADRs/specifications.
3. The supplied OQCI presentation PDF.
4. The supplied OQCI architecture/technology-stack presentation material.
5. The supplied OQCI complete development guide and project proposal.
6. Literature/research sources used for rationale.
7. General model knowledge only when the higher-priority sources do not specify an implementation detail.

An implementation agent must not silently override a locked decision because another framework uses a different architecture.

## Current Repository Baseline

At version `0.7.1` (branch `feat/completion`), the non-G Definition of Done
(`final-deliverables-spec.md` §34) is met, and Stage G is locked and
implemented. In detail:

- **IR core.** QC-IR, QCO-IR, deterministic conversion, validation (I1–I8)
  and symbolic/numeric parameters with explicit binding. Untrusted register
  widths are refused before allocation (`ir::MAX_REGISTER_WIDTH`).
- **QIR.** QIR 1.0 emission is the interoperability boundary (§14): one
  canonical module per compilation, honest `base_profile`/`custom` labeling,
  and acceptance by PyQIR 0.12.5, `llvm-as` 18 and `llvm-as` 14, observed in
  CI (`docs/qir_lowering.md` §4).
- **Frontends.** OpenQASM 3 and CUDA-Q (documented subsets) and Qiskit, all
  into the same QC-IR. **Cirq is out of scope** by the owner's decision for
  `0.6.0`: §5.4 reserves it "to the committed scope", and the committed scope
  is none.
- **Optimization.** The pass manager runs canonicalization, gate
  cancellation, rotation merging, **single-qubit gate fusion** (§8.3,
  target-aware) and scheduling. All are property-tested against an
  independent state-vector model (`docs/pass_manager.md`).
- **Semantic verification in the compiler** (Stage G §3.1), `--verify`. It
  checks binding, every pass (extension passes included) and lowering, and
  stops compilation on a mismatch (`docs/verification.md`).
- **Lint** (§13.5): five rules behind a `LintRule` boundary.
- **Targets and lowering.**
  - Basis profiles, directed topology, legality and a target-supplied cost
    model.
  - Profiles can be loaded at runtime: `backend::from_target_json`,
    `--target-file`, `target=`.
  - Layout, routing and 22 decomposition rules covering CX- and CZ-native
    bases, with lowering re-checking its own output.
- **Backend contract and orchestration.** Lowering, validation, preparation
  and execution are distinct stages with full provenance. One orchestrator
  serves the CLI, the SDK and library callers, and reports per-stage
  `Timings` (§24). Adapter results convert back into `ExecutionResult`
  (§10).
- **Extensions** (§18, DoD item 9). A source-level `Registry` of frontends,
  passes, backends and lint rules. The demonstration crate
  `examples/plugin-demo` is tested end to end (`docs/extensions.md`).
- **Execution adapters.**
  - **Aer.**
  - **CUDA-Q**, checked against a real, pinned CUDA-Q in a Linux CI job.
  - **IBM Quantum hardware** (CZ-native Heron), `oqci.backends.ibm`. It
    builds the profile from the device's live target, checks the ISA (never
    transpiling), and submits verbatim with no twirling or dynamical
    decoupling. Queue and execution time are kept separately. It is tested
    end to end offline through IBM's local testing mode on a Heron
    snapshot, and verified against `qiskit-ibm-runtime` 0.50.0.
  - Live runs are the operator's, with their own account
    (`docs/hardware.md`). Live submission remains outside the Definition of
    Done (§34); it is implemented anyway.
  - `python -m oqci run` compiles and executes in one command.
- **Benchmarks** (§20–§22, §24, §30): `benchmarks/`, under the locked
  Stage G. It includes:
  - a 64-circuit corpus on three targets;
  - OQCI against Qiskit O3/O0 and t|ket⟩;
  - an exhaustive correctness check of every compiled output;
  - determinism across fresh processes, interop and mutation evidence;
  - a validated checker coverage matrix;
  - a report generated only from serialized data, and a reference run
    (`docs/benchmarks.md`), being regenerated on `0.7.1` as of 2026-10-21:
    the `0.6.0` run stopped on a t|ket⟩ defect and was dropped.
- **Determinism contract**, specified in `docs/determinism.md` and checked
  across processes on ubuntu and windows.
- **Engineering.**
  - Unit, integration and property tests.
  - Five cargo-fuzz targets, one of which hunts miscompilations under
    `--verify`.
  - `cargo-deny` and `cargo-audit`.
  - Wheel builds, a `Dockerfile`, and an mdBook documentation build — all in
    CI.
- **Live visualization companion** (`server/`, `frontend/`): a local,
  read-only dev tool (`docs/visualization.md`).

Deliberately absent:

- **MLIR integration.** Optional by Stage B, and out of scope for `0.6.0`;
  `mlir_compat.rs` is the empty seam.
- **Calibration and error data on profiles** (Stage D §2). Layout and routing
  minimize hop count, not error.
- **ECR-native IBM devices.** ECR is not in the gate set.
- **A dynamic (binary) plugin mechanism** (§18.4).

The changelog maps the versions:

| Version | Contents |
|---|---|
| `0.0.1` | The Phase 0 IR core. |
| `0.2.0` | Frontends, Stage F parameters, the pass manager and the CLI. |
| `0.3.0` | The target model, lowering, the backend contract and the orchestrator. |
| `0.4.0` | The visualization companion. |
| `0.4.1` | A lowering fix. |
| `0.5.0` | The QIR interoperability boundary, and the CUDA-Q frontend and adapter. |
| `0.6.0` | Completion: extensions, verification, lint, fusion, runtime targets and the IBM adapter, timing and result retrieval, the benchmark suite under the locked Stage G, and fuzzing and CI hardening. |
| `0.7.1` | Fixes: a pyo3 security upgrade, five defects found by fuzzing (numerical and structural limits), the t\|ket⟩ permutation fallback, and CI repairs. |

## How to Use These Documents

Implement stages in dependency order rather than alphabetic-letter order when the engineering dependency differs.

The practical high-level progression is:

`A → frontend/contracts → pass infrastructure → optimization → target model → routing/lowering → backend execution → interop hardening (QIR conformance and validation) → determinism contract → semantic validation → analysis/benchmark infrastructure → B MLIR boundary expansion (optional) → hardware-specific integration`

These documents define the constraints any roadmap must obey.

## Phases and Versions

Older documents use "Phase n" labels. They are historical: each names a
completed step and maps to a released version. Future work is named by what it
does, not by a phase number.

| Label in older documents | Version | What it was |
|---|---|---|
| Phase 0 | `0.0.1` | The IR core: QC-IR, QCO-IR, conversion, textual QIR emission, validation. |
| Phase 1 | `0.2.0` | The frontend layer (OpenQASM 3, Qiskit), Stage F parameters, the pass manager with its passes, the CLI. |
| "Phase 3" (optimization passes) | `0.2.0` | Not a separate release: the passes shipped with the frontend layer, in Rust rather than MLIR. |
| — | `0.3.0` | Target model, target lowering, backend contract, orchestrator. |
| — | `0.4.0` | Live visualization companion. |
| — | `0.4.1` | A lowering fix (declared wide gates). |
| "Phase 2" (MLIR integration) | — | Not done; MLIR is optional (Stage B). |
| — | `0.5.0` | QIR interoperability boundary: `final-deliverables-spec.md` §14. |
| — | `0.6.0` | Completion of the non-G deliverables and of Stage G (see the baseline above). |
| — | `0.7.1` | Fixes to `0.6.0`: security, fuzzing findings, CI. |

## Critical Rule

No coding agent may treat a feature as implemented merely because:

- its directory exists;
- an interface has been stubbed;
- a diagram contains the feature;
- a README names the feature;
- or a TODO mentions the feature.

A feature is implemented only when its implementation, tests, documentation, and acceptance criteria defined in the relevant stage/specification are satisfied.
