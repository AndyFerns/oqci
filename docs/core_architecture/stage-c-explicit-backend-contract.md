# Stage C — Explicit Backend Contract and IBM Target-Lowering Boundary

Status: LOCKED  
Decision: **C — Define an explicit backend contract and add a dedicated IBM target-lowering stage. Never assume that arbitrary emitted QIR is directly executable on IBM hardware.**

## 1. Objective

Separate compiler output from physical execution.

The compiler must distinguish:

1. a target-independent optimized quantum program;
2. a backend target description;
3. target-specific lowering;
4. execution submission;
5. execution results.

The IBM backend is a first real-hardware target, not a special case buried inside generic optimization code.

## 2. Required Conceptual Pipeline

The final architecture must allow this flow:

`Frontend`
→ `QC-IR`
→ `QCO-IR`
→ `target-independent optimization`
→ `target/backend selection`
→ `IBM target lowering`
→ `IBM-valid executable representation`
→ `IBM execution adapter`

The same compiler core must remain usable for simulation and future backends.

## 3. Backend Contract

Define an explicit OQCI backend abstraction with responsibilities separated from generic compiler logic.

At minimum, the backend contract must be able to provide or expose:

- target identity;
- supported qubit count/physical resources;
- connectivity/topology;
- supported operation/basis information;
- operation constraints;
- measurement constraints;
- reset constraints where relevant;
- parameter constraints;
- target-specific decomposition information;
- cost-model access;
- circuit validation;
- target lowering entry point;
- executable representation type;
- execution interface;
- structured execution result.

Avoid embedding vendor-specific API calls inside optimization passes.

## 4. Target Lowering

Target lowering is the stage that transforms a compiler-internal circuit into something that satisfies a specific backend.

For IBM, this stage may need to account for:

- physical qubit topology;
- target-supported gates;
- layout;
- SWAP insertion;
- basis decomposition;
- gate direction/orientation constraints;
- measurement constraints;
- backend-specific parameter/operation restrictions;
- backend error/cost information;
- execution API requirements.

The exact IBM operation set must not be hard-coded into the generic IR. It belongs to the IBM backend profile.

## 5. QIR Is an Intermediate Output, Not a Universal Execution Guarantee

The existing repository's textual QIR emitter is useful as a lowering/output artifact, but it currently declares several extended intrinsics and does not itself guarantee IBM execution compatibility.

Therefore:

- do not document "QIR emitted = executable on IBM";
- do not write backend tests that assume this;
- do not allow the hardware layer to consume arbitrary high-level QIR without validating the target contract;
- explicitly separate QIR generation from IBM target preparation.

## 6. Backend Independence

The generic compiler must not contain:

```text
if IBM
    ...
else if Cirq
    ...
else if CUDA-Q
    ...
```

throughout optimization logic.

Backend-specific behavior belongs behind backend/target interfaces.

## 7. Simulation Backends

Simulator execution should use the same backend contract where practical.

The project presentation names:

- Qiskit Aer;
- Cirq simulator;
- CUDA-Q simulator

as execution targets.

They may share common backend interfaces but can expose backend-specific capabilities.

## 8. IBM Execution Adapter

The IBM execution adapter is responsible for:

- authentication/configuration;
- backend discovery;
- target retrieval;
- target profile construction;
- executable submission;
- shot configuration;
- result retrieval;
- structured metadata capture.

Queue/wait behavior must not be mistaken for compiler execution time.

## 9. Required Result Provenance

Every execution result should be attributable to:

- input circuit identifier;
- compiler version;
- Git commit;
- selected optimization pipeline;
- backend identity;
- target profile/configuration identifier;
- compilation metadata;
- execution settings;
- shot count where relevant;
- raw backend result;
- derived metrics.

## 10. Exit Criteria

Stage C is complete when:

1. A backend contract exists and is documented.
2. Simulator backends use the contract.
3. Target-independent compilation does not contain IBM-specific branching.
4. IBM target lowering is a distinct, testable component.
5. IBM target validity is checked before submission.
6. QIR generation and hardware execution are explicitly separated.
7. Backend results preserve sufficient provenance for later benchmarking.
