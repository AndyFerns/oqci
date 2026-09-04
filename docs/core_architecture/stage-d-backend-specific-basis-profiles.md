# Stage D — Backend-Specific Basis Profiles and Target-Aware Lowering

Status: LOCKED  
Decision: **D2 — Abstract gate set → backend-specific basis profile.**

## 1. Objective

Preserve a vendor-neutral abstract circuit model while allowing each target backend to define how abstract operations become executable operations.

The core rule is:

**The abstract OQCI gate vocabulary must not be rewritten every time a hardware backend changes.**

Instead:

`Abstract Gate Set`
→ `Target/Basis Profile`
→ `Target Lowering`
→ `Executable Circuit`

## 2. What a Basis Profile Is

A basis profile is a formal target description containing the operations and constraints that the backend can accept or efficiently realize.

It is more than a list of gate names.

The profile should be able to describe:

- supported one-qubit operations;
- supported two-qubit operations;
- supported multi-qubit operations where applicable;
- parameterized operations;
- parameter domains/constraints;
- operation decomposition rules;
- connectivity;
- directed versus undirected coupling constraints;
- measurement constraints;
- reset constraints;
- operation-specific costs;
- optional error/noise metadata;
- target-specific legality requirements.

## 3. Example Conceptual Profile

Do not hard-code the following as final IBM values; this illustrates the abstraction.

```text
BackendProfile
    id
    version
    qubit_count
    topology
    basis_operations
    decomposition_rules
    measurement_constraints
    parameter_constraints
    cost_model
    capabilities
```

An IBM profile would then provide IBM-specific target information.

A future backend can provide a different profile without changing QC-IR.

## 4. Separation of Responsibilities

### Abstract IR

Defines what the quantum program means.

Examples:

- H
- Rx(theta)
- Ry(theta)
- Rz(theta)
- CX
- SWAP
- measurement

### Basis profile

Defines what the chosen backend accepts or prefers.

### Target lowering

Decides how to turn the abstract operation into legal target operations.

### Routing/mapping

Decides where logical qubits live physically and how connectivity constraints are satisfied.

These must not be conflated.

## 5. Decomposition Rules

Every non-native abstract operation that reaches target lowering must have a documented strategy.

For each decomposition rule define:

- source operation;
- target operation sequence;
- parameter transformation;
- qubit operand mapping;
- classical/result behavior if relevant;
- semantic-preservation expectation;
- whether the rule is exact or approximate;
- target-specific cost implications.

Do not implement an ad hoc decomposition inside the backend executor.

## 6. Mapping and Routing

The target profile must expose topology information so mapping/routing can use it.

Required concepts include:

- logical qubit IDs;
- physical qubit IDs;
- initial layout;
- layout updates;
- connectivity edges;
- two-qubit operation legality;
- SWAP insertion;
- final layout/reporting.

Routing must preserve semantic wire relationships.

## 7. Directionality

If a backend treats a two-qubit interaction as directed, the target model must represent that explicitly.

Do not assume that an undirected edge means both ordered interactions are equally native.

If a reverse interaction can be implemented by basis changes or conjugation, encode the transformation in the target-specific lowering rules rather than silently reversing operands.

## 8. Basis Profile Versioning

Target profiles can change as hardware/backends evolve.

Therefore every profile used for research experiments should have a stable identifier/version or serialized snapshot.

A benchmark result must record which target profile was used.

## 9. Why This Matters to OQCI

This design provides:

- vendor neutrality;
- retargetability;
- reproducibility;
- cleaner optimization code;
- backend-specific research experiments;
- the ability to compare different cost models;
- a path to future non-IBM hardware.

It also directly supports the project's long-term vision of expanding beyond one hardware family.

## 10. Exit Criteria

Stage D is complete when:

1. Backend target information is represented explicitly.
2. At least one backend profile is implemented end-to-end.
3. Abstract operations can be lowered to the target profile.
4. Unsupported operations are detected before execution.
5. Decomposition rules are tested.
6. Topology constraints are represented explicitly.
7. Mapping/routing can consume target topology without embedding vendor logic in the core optimizer.
8. Target-profile identity is included in execution/benchmark metadata.
