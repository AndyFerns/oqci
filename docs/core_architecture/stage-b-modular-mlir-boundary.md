# Stage B — Modular MLIR Integration Boundary

Status: LOCKED  
Decision: **B2/B3 — MLIR is enabling/modular infrastructure; OQCI's main contribution is the Q-LLVM-inspired, vendor-neutral compiler architecture and reusable optimization/backend ecosystem.**

## 1. Objective

Integrate MLIR where it provides real compiler infrastructure value without turning OQCI into an "MLIR project".

The purpose of MLIR in OQCI is to support:

- standardized compiler infrastructure;
- dialect definitions;
- verification hooks;
- reusable transformation infrastructure;
- a future scalable lowering path;
- interoperability with LLVM/QIR;
- modular pass infrastructure where justified.

The purpose is not to rewrite stable Rust IR code simply to satisfy a diagram.

## 2. Architectural Position

The target architecture is conceptually:

`External Frontends`
→ `QC-IR`
→ `QCO-IR`
→ `optimization infrastructure`
→ `target lowering`
→ `QIR/LLVM`
→ `backend`

MLIR may occupy the optimization/lowering layer, but it must not create an incompatible second semantic universe.

## 3. Phased Strategy

### Phase B-Initial: Rust-only core

Keep the current Rust IR and Rust-native orchestration while frontend and pass contracts are still evolving.

Use the existing `src/ir/mlir_compat.rs` seam for future marshalling/interoperability.

Do not force every contributor to install/build the complete LLVM/MLIR stack while the IR semantics are still changing.

### Phase B-Expansion: MLIR boundary

Introduce actual MLIR integration only when it provides a concrete benefit.

The minimum useful integration should establish:

- a formally described quantum dialect;
- a stable mapping from relevant QC-IR/QCO-IR structures to MLIR;
- operation verification corresponding to OQCI IR invariants;
- a documented marshalling boundary;
- a controlled route from the MLIR representation toward LLVM/QIR lowering.

### Phase B-Later: Native MLIR toolchain where justified

The presentation permits a native C++ MLIR layer later.

Do not implement a C++ MLIR stack merely because it was listed as an optional technology.

Use it where one or more of the following is demonstrably valuable:

- MLIR-native transformations;
- dialect conversion;
- established MLIR analysis infrastructure;
- integration with LLVM tooling;
- research experiments that need MLIR pass infrastructure.

## 4. What Must Remain OQCI-Owned

OQCI must continue to own:

- its frontend abstraction;
- canonical quantum IR semantics;
- pass contracts;
- backend contract;
- target/basis-profile abstraction;
- cost-model abstraction;
- compiler orchestration;
- benchmark metadata and result schema;
- reproducibility rules.

Third-party compiler infrastructure should be used as implementation infrastructure, not allowed to redefine OQCI semantics implicitly.

## 5. Reuse Instead of Reinvention

The project explicitly favors modular reuse.

Prefer established components when they satisfy the requirement and can be isolated behind a stable OQCI boundary.

Examples include:

- LLVM/QIR tooling for LLVM/QIR mechanics;
- MLIR for generic compiler infrastructure;
- Qiskit/Cirq/CUDA-Q for frontend/backend integration where appropriate;
- existing graph libraries for dependency structures;
- established visualization/data libraries for research tooling.

Reimplement a component only when:

- OQCI needs semantics the component cannot provide;
- the external component cannot be cleanly integrated;
- the implementation itself is a deliberate research contribution;
- or licensing/availability constraints require another solution.

## 6. No Accidental Dual-IR Architecture

Do not allow:

`Rust QCO-IR A`

and

`MLIR QCO-IR B`

to diverge.

There must be a clearly defined semantic correspondence.

Every MLIR operation used for OQCI quantum operations must have:

- an OQCI semantic definition;
- operand/attribute mapping;
- verification requirements;
- lowering behavior;
- tests.

## 7. MLIR Scope Constraints

Do not expand the project into:

- general-purpose MLIR compiler research;
- arbitrary classical dialect ecosystems;
- dynamic quantum control-flow support solely to showcase MLIR;
- an independent quantum programming language.

Those are outside the main project contribution.

## 8. Exit Criteria

Stage B is complete for the main deliverable when:

1. The Rust core remains authoritative and regression-tested.
2. The MLIR boundary is documented.
3. Relevant QC-IR/QCO-IR constructs have a deterministic mapping to the chosen MLIR representation.
4. MLIR verification corresponds to OQCI semantic invariants.
5. LLVM/QIR lowering can be connected without changing the backend contract.
6. MLIR integration is modular and optional wherever practical.
7. Documentation clearly distinguishes OQCI-owned abstractions from external compiler infrastructure.
