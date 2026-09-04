# Stage F — Static Circuits with Parameterized Rotations

Status: LOCKED  
Decision: **F2 — Support static circuits plus symbolic/numeric parameterized rotations. Defer runtime-dependent dynamic circuit semantics.**

## 1. Objective

Support useful research workloads, especially parameterized variational circuits, without allowing dynamic-circuit semantics to expand the project beyond the intended scope.

The supported mainline model is:

**static circuit topology + fixed wire structure + symbolic/numeric gate parameters**

## 2. Required Parameter Support

The IR and relevant passes must support parameterized forms of rotation operations, including:

- `Rx(theta)`
- `Ry(theta)`
- `Rz(theta)`

where `theta` may be represented in the project's chosen symbolic/numeric parameter mechanism.

The exact symbolic expression representation must be selected during implementation, but it must preserve the distinction between:

- a concrete numeric angle;
- a parameter/symbol;
- an operation whose structure is fixed while a parameter changes.

Do not silently replace symbolic parameters with arbitrary runtime code.

## 3. Why Parameterized Circuits Are In Scope

This enables:

- VQE-style ansatzes;
- repeated compilation/execution experiments;
- parameter sweeps;
- rotation-merging research;
- symbolic optimization experiments;
- future just-in-time/partial compilation research.

The supplied project presentation explicitly identifies intelligent optimization, hardware-aware compilation, noise-aware optimization, scalable compilation and cross-platform support as future directions; dynamic circuits are also identified as future scope rather than a required foundation capability.

## 4. Static-Circuit Definition

For the main implementation, the following characteristics are considered static:

- qubit count;
- classical-bit count;
- operation topology;
- operation ordering/dependency structure;
- presence and location of measurements/reset;
- control structure represented in the supported IR.

Parameter values may vary without changing those structural properties.

## 5. Dynamic Circuits Are Out of Scope

The main deliverable does not require:

- measurement-conditioned branches;
- runtime-generated operations;
- arbitrary classical control flow;
- runtime-dependent circuit topology;
- dynamic allocation based on execution results;
- a full hybrid quantum-classical control-flow IR.

These may be future research directions.

Do not introduce them merely because a frontend or framework supports them.

## 6. Parameter-Aware Optimization

Passes must preserve parameter semantics.

For rotation merging, the implementation must distinguish cases such as:

```text
Rz(a) followed by Rz(b)
```

from incompatible symbolic expressions.

The optimizer may transform the expression only when the transformation is semantically valid under the project's chosen parameter algebra.

Do not evaluate symbolic parameters prematurely.

## 7. Parameter Serialization

Any serialized representation must preserve enough information to reconstruct the parameterized operation.

A benchmark/execution layer must record whether a circuit is:

- fully numeric;
- parameterized;
- instantiated from parameter values.

## 8. Hardware Lowering Interaction

Before physical execution, symbolic parameters must be resolved or lowered according to the backend's capabilities.

The IBM target-lowering layer must not receive ambiguous symbolic values where the execution API requires concrete values.

Parameter binding must therefore be an explicit compiler/backend step.

## 9. Research Boundary

The following is the intended boundary:

```text
Supported:
    static circuit
    + symbolic/numeric parameters
    + parameter binding

Deferred:
    runtime measurement
    → classical condition
    → dynamically generated quantum operations
```

## 10. Exit Criteria

Stage F is complete when:

1. QC-IR can represent supported static parameterized rotations.
2. Parameter values survive frontend → IR → optimization → lowering transformations correctly.
3. Core optimization passes do not corrupt symbolic parameters.
4. Parameter binding is explicit and testable.
5. VQE-style parameterized circuits can be represented.
6. Backend lowering can produce executable concrete values where required.
7. Dynamic control-flow semantics remain explicitly outside the main project scope.
