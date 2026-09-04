# Stage E — Backend-Defined Cost Model

Status: LOCKED  
Decision: **E3 — The selected backend supplies the cost model used by target-aware optimization.**

## 1. Objective

Define optimization cost in terms of the target being compiled for rather than embedding one universal set of weights into OQCI.

The central principle is:

**The compiler optimizer chooses among transformations; the target describes what is expensive.**

## 2. Why the Backend Owns Cost

Quantum hardware architectures differ.

The relative importance of:

- two-qubit operations;
- circuit depth;
- routing overhead;
- operation duration;
- error rates;
- calibration information;
- target-native operation counts

can vary by backend.

A universal hard-coded weighted score would therefore become a hidden assumption.

## 3. Required Cost-Model Interface

The OQCI cost-model interface should support target-aware evaluation.

Conceptually:

```text
CostModel
    evaluate(circuit_or_candidate) -> Cost
    compare(a, b) -> ordering or relation
    explain(circuit_or_candidate) -> structured breakdown
```

The exact Rust trait/API is to be finalized during implementation, but it must support structured rather than opaque results.

## 4. Cost Object

Do not represent cost only as one `f64`.

A cost result should retain component metrics such as:

- total gate count;
- one-qubit gate count;
- two-qubit gate count;
- depth;
- routing/SWAP overhead;
- target-native gate counts;
- estimated execution duration when available;
- estimated error/noise contribution when the backend can supply it.

A scalar score may be derived for optimization decisions, but the component values must remain accessible.

## 5. Initial IBM/NISQ Cost Model

The initial production research target is IBM-style gate-model NISQ hardware.

The model should therefore be capable of considering, at minimum:

- two-qubit operation burden;
- circuit depth;
- routing overhead;
- target-native operation count;
- optional backend-provided error information.

Do not hard-code numeric weights in the architecture document unless a later research decision establishes and documents them.

## 6. Why E2 Is Not the Primary Architecture

A generic weighted objective is still possible as an implementation mechanism.

However, E2 becomes problematic if the weights are arbitrary.

For example:

```text
0.6 * depth + 0.3 * CX_count + 0.1 * gate_count
```

has no inherent scientific legitimacy unless the experiment establishes why those coefficients are appropriate.

Therefore:

- E3 is the architectural contract;
- an E2-style weighted score may be one implementation of a backend's cost model;
- the weights must come from explicit backend/model configuration;
- the experiment must preserve the unweighted component metrics.

## 7. Avoid Metric Collapse

The benchmarking system must not report only:

`OQCI cost = 123.4`

That would hide why the optimizer made its decision.

Instead, retain the cost breakdown.

Example:

```text
Cost
    two_qubit_count: ...
    depth: ...
    swap_count: ...
    native_gate_count: ...
    estimated_duration: ...
    estimated_error: ...
    scalar_score: ...
```

The exact field names may evolve, but the separation of raw metrics and derived scalar score is required.

## 8. Cost Model and Pass Ordering

The pass manager should be capable of allowing a pass to consult target cost information where appropriate.

However, target-aware behavior must not make every pass backend-specific.

Prefer:

- generic transformations in generic passes;
- target-aware passes where target data is genuinely needed;
- backend cost evaluation through the cost-model interface.

## 9. Cost Model Reproducibility

A compiled result must be attributable to:

- backend;
- target profile;
- cost-model identifier/version;
- cost-model configuration;
- compiler version;
- pass pipeline.

This is essential for comparing optimization experiments.

## 10. Exit Criteria

Stage E is complete when:

1. The backend contract exposes a cost model.
2. The cost model returns structured components plus an optional scalar.
3. Optimization can consult backend-defined costs.
4. Raw metrics are never discarded in favor of the scalar.
5. Cost-model identity/configuration is recorded in experiment metadata.
6. The initial IBM/NISQ cost model is documented without arbitrary undocumented weights.
