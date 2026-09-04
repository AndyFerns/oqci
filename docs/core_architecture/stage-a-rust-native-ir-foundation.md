# Stage A — Rust-Native Compiler Core and IR Stabilization

Status: LOCKED  
Decision: **A1 — Rust-first implementation, with MLIR introduced progressively after the Rust IR/core is stable.**

## 1. Objective

Establish OQCI's compiler semantics in Rust before introducing the full MLIR toolchain.

The authoritative core representation for the initial implementation is:

`QC-IR → QCO-IR → lowering/output`

The current repository already implements the first version of this foundation. Stage A is therefore not a request to rebuild the existing IR; it is the requirement that the existing IR become a stable contract that all later frontends, passes, backends, and MLIR integrations must respect.

## 2. Current Verified Baseline

The current `master` repository at version `0.0.1` implements:

- `src/ir/qc.rs`
  - `Circuit`
  - `CircuitBuilder`
  - ordered instructions
  - fixed quantum/classical register sizes
- `src/ir/types.rs`
  - `QubitId`
  - `ClbitId`
  - `Angle`
  - `GateKind`
- `src/ir/qco.rs`
  - `QcoCircuit`
  - explicit input/output boundary nodes
  - data/control dependency edges
  - deterministic topological ordering
  - canonical linearization
- `src/ir/convert.rs`
  - deterministic QC-IR → QCO-IR conversion
- `src/ir/qir.rs`
  - textual LLVM-compatible QIR emission
- `src/ir/error.rs`
  - centralized validation/error types
- `src/ir/mlir_compat.rs`
  - reserved future MLIR marshalling boundary

The repository changelog states that frontends, optimization passes, and backend execution are intentionally absent from this Phase 0 baseline.

## 3. Architectural Principles

### 3.1 Rust owns the initial semantic core

Do not move the canonical semantics into MLIR merely because MLIR exists in the long-term architecture.

The Rust representation must remain understandable, testable, deterministic, and usable independently of surface-language parsers.

### 3.2 Frontends consume the IR; they do not define it

A frontend must translate external programs into the established QC-IR contract.

If an input language exposes a semantic feature that QC-IR cannot represent, that is an explicit IR-design decision. Do not silently mutate QC-IR to make one parser easier.

### 3.3 Optimizers operate on an explicit optimization representation

QCO-IR exists to expose dependency structure.

The QC-IR → QCO-IR conversion must remain deterministic and semantics-preserving.

### 3.4 Lowering is explicit

A later backend must never be assumed to understand arbitrary high-level OQCI operations merely because OQCI can serialize them.

## 4. Binding IR Invariants

The following design constraints are load-bearing because they preserve the future MLIR mapping:

- `GateKind` remains a closed enum plus one `Opaque` escape hatch until an explicit architecture decision changes this.
- Gate parameters remain attached to `GateKind`; qubit/classical operands remain on instructions/nodes.
- `QubitId` and `ClbitId` remain distinct types.
- `Angle` remains a dedicated parameter/value type and uses radians.
- Construction remains centralized through `CircuitBuilder`.
- Validation remains centralized and returns explicit errors rather than panicking on malformed user input.
- The IR keeps quantum and classical wire identities explicit.
- Control/target role information for controlled gates remains recoverable.
- QCO-IR dependency edges retain the distinction between value/data dependencies and control/barrier dependencies.
- Deterministic traversal remains a required property.

Do not alter any of these constraints casually. An alteration requires an explicit architecture decision and corresponding specification/test updates.

## 5. QC-IR Required Final Scope for This Project

QC-IR must support, at minimum:

- single-qubit gates;
- parameterized rotations;
- multi-qubit gates represented by the project's abstract gate set;
- measurement;
- reset;
- fixed qubit register allocation;
- classical result targets;
- symbolic/numeric rotation parameters under the Stage F rules.

The current project does not require arbitrary dynamic classical control flow.

## 6. QCO-IR Required Final Scope

QCO-IR must provide:

- circuit operation nodes;
- explicit wire/dependency structure;
- deterministic ordering;
- dependency classification;
- a reliable representation suitable for optimization passes;
- a canonical conversion back to an executable ordered form when required by downstream lowering.

The optimization representation must make parallelism visible without destroying program semantics.

## 7. Required Tests Before Dependent Work Is Considered Stable

At minimum:

- valid construction tests;
- invalid qubit/clbit reference tests;
- gate-arity tests;
- duplicate-operand tests;
- invalid parameter tests;
- opaque-operation validation tests;
- measurement/reset tests;
- controlled-gate role tests;
- QC-IR → QCO-IR structure tests;
- dependency classification tests;
- deterministic topological-order tests;
- canonical linearization tests;
- QCO-IR/QC-IR semantic round-trip tests;
- QIR emission smoke tests;
- regression tests for all existing invariants.

A future contributor must extend tests before changing an invariant.

## 8. What Stage A Does Not Mean

Stage A does not mean:

- implementing OpenQASM;
- implementing Qiskit/Cirq/CUDA-Q adapters;
- implementing optimization passes;
- implementing IBM hardware;
- converting all optimization logic into MLIR;
- making dynamic circuits fully supported.

Those are later stages.

## 9. Exit Criteria

Stage A is complete only when:

1. QC-IR and QCO-IR are documented as stable contracts.
2. Existing IR tests remain green.
3. Later components can consume the IR without changing its basic semantics.
4. The IR can represent the static + parameterized circuit subset required by Stage F.
5. Any required IR extension has an explicit documented rationale.
6. The current QIR emitter remains covered by regression tests.
7. The IR boundary is sufficiently stable to support frontend development without frontend syntax becoming the source of IR semantics.
