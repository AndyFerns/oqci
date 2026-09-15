# OQCI Source-of-Truth Documentation Index

Status: Active planning baseline  
Last verified against repository: 2026-09-04  
Repository: `https://github.com/AndyFerns/oqci`  
Branch reviewed: `master`  
Current repository version: `0.0.1`

## Purpose

This directory is the authoritative implementation-planning index for the Open Quantum Compiler Infrastructure (OQCI) project.

The documents are intentionally split into architecture/design decisions and one consolidated final-deliverables specification. They are written to be usable by a human developer or an LLM coding agent without silently inventing architecture.

**Stage G is intentionally not finalized.** Its purpose is the experimental benchmark matrix/protocol and hardware-test methodology. It requires additional research and discussion before becoming a locked specification.

## Locked Architecture Decisions

| Stage | File | Decision | Status |
|---|---|---|---|
| A | [`stage-a-rust-native-ir-foundation.md`](stage-a-rust-native-ir-foundation.md) | Rust-native IR first; stabilize QC-IR/QCO-IR before introducing heavy MLIR infrastructure. | LOCKED |
| B | [`stage-b-modular-mlir-boundary.md`](stage-b-modular-mlir-boundary.md) | MLIR is a modular/enabling infrastructure layer, not the project's primary research contribution. Integrate progressively after the Rust core is stable. | LOCKED |
| C | [`stage-c-explicit-backend-contract.md`](stage-c-explicit-backend-contract.md) | Backend execution is governed by an explicit OQCI backend contract; IBM gets a dedicated target-lowering stage. | LOCKED |
| D | [`stage-d-backend-specific-basis-profiles.md`](stage-d-backend-specific-basis-profiles.md) | Target-independent abstract gates are lowered through backend-specific basis profiles. | LOCKED |
| E | [`stage-e-backend-defined-cost-model.md`](stage-e-backend-defined-cost-model.md) | Optimization cost is supplied by the selected backend/target cost model. Preserve raw metrics; do not collapse the research evaluation to one scalar. | LOCKED |
| F | [`stage-f-static-parameterized-circuit-scope.md`](stage-f-static-parameterized-circuit-scope.md) | Main project supports static circuits plus symbolic/numeric parameterized rotations. Runtime-dependent dynamic control flow is deferred. | LOCKED |
| G | `stage-g-benchmarking-protocol.md` | Benchmark corpus, repetitions, noise policy, hardware selection, statistical protocol, and IBM experiment design. | PENDING — NOT LOCKED |

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

At the verified `master` state:

- the repository is at version `0.1.0`;
- the implemented core is the Phase 0 IR foundation plus the frontend layer;
- QC-IR exists;
- QCO-IR exists;
- deterministic QC-IR → QCO-IR conversion exists;
- QCO-IR → textual LLVM-compatible QIR emission exists;
- validation/error infrastructure exists;
- test coverage exists for the current IR/pipeline;
- symbolic/numeric gate parameters (`Param`) and explicit parameter binding exist, satisfying the Stage F IR requirement;
- an OpenQASM 3 frontend exists over a documented subset (`docs/openqasm_frontend.md`);
- a Qiskit adapter exists, split into a pure-Rust translation core and a PyO3 boundary (`docs/qiskit_adapter.md`);
- a pass manager and four target-independent passes exist — canonicalization, gate cancellation, rotation merging, and scheduling-as-analysis (`docs/pass_manager.md`);
- pass correctness is verified by property-based state-vector equivalence testing (`tests/pass_equivalence.rs`);
- an analysis module provides gate counts, depth and circuit diffing (`src/analysis/`);
- an `oqci` CLI exposes every pipeline stage, including a live `watch` mode and a JSON schema (`docs/cli.md`);
- `src/` contains the IR, frontend, pass, analysis and CLI subsystems;
- **target model, qubit mapping, routing, basis decomposition, execution-backend and compiler-orchestration subsystems are not yet implemented** — mapping/routing/decomposition are blocked on the Stage D target profile;
- `python/` is a real PyO3 crate covering the frontend → QIR path only; the compiler/pass/backend/analysis APIs of §17 do not exist yet;
- `benchmarks/` is not yet a populated benchmark suite;
- `mlir_compat.rs` exists as the future MLIR boundary but the full MLIR integration is not implemented.

The repository's changelog defines `0.0.1` as the Phase 0 IR core, and the
unreleased `0.1.0` entry as the frontend layer, the Stage F parameter work,
the pass manager with its target-independent passes, and the CLI. Target
modelling and backend execution remain deliberately absent.

## How to Use These Documents

Implement stages in dependency order rather than alphabetic-letter order when the engineering dependency differs.

The practical high-level progression is:

`A → frontend/contracts → pass infrastructure → optimization → target model → routing/lowering → backend execution → analysis/benchmark infrastructure → B MLIR boundary expansion → hardware-specific integration`

The exact phase decomposition belongs to the final implementation roadmap; these documents define the constraints that roadmap must obey.

## Critical Rule

No coding agent may treat a feature as implemented merely because:

- its directory exists;
- an interface has been stubbed;
- a diagram contains the feature;
- a README names the feature;
- or a TODO mentions the feature.

A feature is implemented only when its implementation, tests, documentation, and acceptance criteria defined in the relevant stage/specification are satisfied.
