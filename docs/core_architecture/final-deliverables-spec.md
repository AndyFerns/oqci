# OQCI Final Deliverables Specification — Source of Truth

Status: LOCKED for non-G benchmarking-design scope  
Stage G: intentionally excluded from the locked experimental protocol  
Repository baseline verified: `master`, version `0.0.1`, 2026-09-04 (historical; see §2 and [`index.md`](index.md))  
Repository: `https://github.com/AndyFerns/oqci`

## 0. Purpose

This document defines the complete set of software, architecture, testing, documentation, tooling, and research-package deliverables required to take OQCI from its current Phase 0 IR foundation to the intended non-G project deliverable.

It is written as an implementation contract.

An LLM coding agent must treat the requirements here as constraints, not suggestions.

Where this document says "must", the implementation must satisfy it or explicitly raise a design issue before proceeding.

Where this document says "may", the implementation can vary without violating the locked architecture.

Where this document says "deferred", do not implement the feature as part of the current locked scope.

---

# 1. Project Definition

*Amended 2026-09-30 per [`framing-and-research-question.md`](framing-and-research-question.md).*

OQCI is a modular, open-source quantum compiler infrastructure that builds upon the broader LLVM/QIR quantum compilation ecosystem but draws a different architectural boundary: quantum-native compiler infrastructure first, LLVM interoperability second. A strongly validated, Rust-native IR is the core; QIR is an interoperability artifact emitted at the output boundary.

Research question: *can a quantum-first, strongly validated intermediate representation provide a clean foundation for deterministic quantum compiler transformations while remaining interoperable with established LLVM/QIR infrastructure?*

The previous identity statement — "a vendor-neutral quantum compiler infrastructure inspired by the decoupling principles of LLVM/Q-LLVM-style compiler ecosystems" — is superseded. Vendor neutrality remains a design property (no vendor types in the IR, no vendor branching in the core).

The core purpose is to separate:

- source-language/frontend integration;
- intermediate representation;
- optimization;
- target-specific lowering;
- backend execution;
- analysis;
- benchmarking;
- plugin/research extensions.

The main research value is the validated IR and the infrastructure built on it — the ability to compose, check and compare compiler transformations across target backends — not the construction of a new quantum programming language.

The supplied project presentation describes the intended architecture as:

`OpenQASM 3 / Qiskit / Cirq / CUDA-Q`
→ frontend adapters
→ `QC-IR`
→ `QCO-IR`
→ pass manager/optimization
→ `QIR / LLVM IR`
→ execution backends.

The locked architecture refines this by making backend targeting explicit:

`...`
→ target selection
→ backend-specific basis profile
→ target lowering
→ executable backend representation
→ execution.

---

# 2. Baseline at `0.0.1` (historical snapshot)

> **Historical snapshot — the repository at version `0.0.1`, verified
> 2026-09-04.** Kept unchanged as the starting point this specification was
> written against. Much of the "does not yet provide" list below has since been
> implemented. It does not describe the current repository; the only living
> baseline is [`index.md`](index.md).

The current repository is not a completed compiler.

The verified `0.0.1` state already contains:

- Rust project infrastructure;
- QC-IR;
- QCO-IR;
- QC-IR → QCO-IR conversion;
- QCO-IR → textual LLVM-compatible QIR emission;
- validation/error handling;
- IR/pipeline tests;
- MLIR compatibility seam.

The current repository does not yet provide the final:

- OpenQASM 3 frontend;
- Qiskit adapter;
- Cirq adapter;
- CUDA-Q adapter;
- pass manager;
- optimization passes;
- target topology/mapping;
- routing;
- basis decomposition;
- generic backend execution abstraction;
- simulator execution stack;
- IBM target-lowering backend;
- complete Python SDK;
- populated benchmark suite;
- full analysis framework;
- complete plugin ecosystem;
- research-result generation pipeline.

Do not represent any of those as existing merely because folders or diagrams exist.

---

# 3. Final System Architecture

The implementation must converge toward the following conceptual structure.

```text
                     ┌──────────────────────┐
                     │  External Programs   │
                     │ QASM / Qiskit / ...  │
                     └──────────┬───────────┘
                                │
                                ▼
                     ┌──────────────────────┐
                     │ Frontend Adapters    │
                     │ parse / translate /  │
                     │ validate             │
                     └──────────┬───────────┘
                                │
                                ▼
                     ┌──────────────────────┐
                     │ QC-IR                │
                     │ imperative IR        │
                     └──────────┬───────────┘
                                │
                                ▼
                     ┌──────────────────────┐
                     │ QCO-IR               │
                     │ optimization DAG     │
                     └──────────┬───────────┘
                                │
                                ▼
                     ┌──────────────────────┐
                     │ Pass Manager         │
                     │ generic + target     │
                     │ aware passes         │
                     └──────────┬───────────┘
                                │
                                ▼
                     ┌──────────────────────┐
                     │ Backend/Target       │
                     │ selection            │
                     └──────────┬───────────┘
                                │
                                ▼
                     ┌──────────────────────┐
                     │ Basis Profile        │
                     │ topology + ops +     │
                     │ constraints + cost   │
                     └──────────┬───────────┘
                                │
                                ▼
                     ┌──────────────────────┐
                     │ Target Lowering      │
                     │ mapping / routing /  │
                     │ decomposition        │
                     └──────────┬───────────┘
                                │
                    ┌───────────┴───────────┐
                    ▼                       ▼
          QIR 1.0 module             Backend-specific
       (interop artifact;            executable form
        honestly labeled)                   │
                    │                       ▼
                    ▼               Simulators / IBM /
         external QIR tools          future hardware
         (PyQIR, llvm-as, …)
```

*Amended 2026-09-30 per [`framing-and-research-question.md`](framing-and-research-question.md).* Execution runs through the right-hand branch only: backends consume the backend-specific executable, never QIR. QIR is an interoperability artifact — one canonical module per compilation, emitted from the most-lowered circuit available — whose evidence of interoperability is that external QIR tools accept it (§14). Since `0.5.0` the orchestrator emits that canonical module and every tool shows it ([`../qir_lowering.md`](../qir_lowering.md) §1.4).

MLIR may appear within the IR/pass/lowering layers as an enabling infrastructure component, but it is not the sole definition of the OQCI architecture.

---

# 4. Core IR Deliverable

## 4.1 QC-IR

QC-IR is the imperative, source-neutral representation.

It must support:

- fixed quantum register size;
- fixed classical register size;
- ordered instructions;
- single-qubit operations;
- multi-qubit operations;
- measurements;
- reset;
- controlled-gate role information;
- parameterized rotations;
- explicit qubit/classical operand identities.

The IR must remain language-neutral.

## 4.2 QCO-IR

QCO-IR is the optimization-oriented dependency representation.

It must support:

- operation nodes;
- input/output wire boundaries;
- data dependencies;
- control/barrier dependencies;
- deterministic topological ordering;
- canonical linearization;
- sufficient metadata for later analysis and optimization.

The dependency graph must be acyclic for supported static circuits.

## 4.3 Conversion

`QC-IR → QCO-IR` must:

- be deterministic;
- preserve circuit semantics;
- preserve operation identity/order as required;
- produce correct data/control dependency edges;
- have tests that expose dependency structure.

If a later pass transforms QCO-IR, it must preserve the invariants necessary to lower the result safely.

## 4.4 Parameter Semantics

The mainline project supports:

- numeric rotations;
- symbolic/named parameters;
- static circuit structure.

The mainline project does not require arbitrary runtime-dependent control flow.

---

# 5. Frontend Deliverables

The frontend layer must be plugin-oriented.

## 5.1 Frontend Contract

Define an OQCI frontend interface with a stable output contract:

`external program`
→ `validated QC-IR`

The frontend boundary must not expose vendor-specific types to downstream Rust optimization code.

## 5.2 OpenQASM 3

Implement a documented supported subset sufficient for the project's intended circuits.

The parser must have:

- lexical/syntactic errors;
- semantic validation;
- useful diagnostics;
- mapping into QC-IR;
- tests;
- round-trip or equivalent structural tests where meaningful.

Do not claim full OpenQASM 3 language coverage unless the implementation actually supports it.

The supported subset must be documented precisely.

## 5.3 Qiskit

Implement translation from `QuantumCircuit` to QC-IR.

Required concerns include:

- qubit mapping;
- classical measurement destinations;
- supported gates;
- supported parameterized rotations;
- unsupported-operation errors;
- preservation of operation ordering and semantics.

The adapter must live at the integration boundary rather than contaminating QC-IR with Qiskit types.

## 5.4 Cirq

The design reserves a Cirq adapter.

It should be implemented only to the level explicitly committed to by the final project scope and should map into the same QC-IR contract.

*Decision, 2026-10-20:* the committed scope for `0.6.0` is **none**. The project owner placed Cirq out of scope. [`../adapters.md`](../adapters.md) keeps the checklist for adding it later.

## 5.5 CUDA-Q

The design reserves a CUDA-Q adapter.

It should be implemented only to the level explicitly committed to by the final project scope and should map into the same QC-IR contract.

## 5.6 Frontend Validation

Validation must reject or report:

- invalid wire references;
- unsupported gates;
- invalid arity;
- invalid parameters;
- unsupported dynamic behavior under Stage F;
- malformed source programs;
- semantic inconsistencies that QC-IR cannot represent safely.

Do not let frontends bypass QC-IR validation.

## 5.7 QIR Ingestion (future work, not a deliverable)

*Amended 2026-09-30 per [`framing-and-research-question.md`](framing-and-research-question.md).* Reading QIR back into QC-IR is **not** part of the locked deliverable. If it is taken up, the intended first scope is a hand-written Rust reader for a strict Base Profile subset, reviewed as a mapping onto the existing IR under the no-frontend ADR's principle. It must refuse dynamic qubit allocation, Adaptive-profile branching and result reads (Stage F §5), and controlled (`__ctl`) forms (no control modifier exists in `GateKind`). Round-tripping OQCI's own output is self-consistency only; interoperability evidence would need QIR produced by other tools at pinned versions.

---

# 6. Compiler Orchestration Deliverable

Implement a compiler orchestration layer that can perform:

`Frontend → QC-IR → QCO-IR → pass pipeline → target lowering → backend preparation`

The compiler orchestrator must:

- accept an explicit configuration;
- select frontend;
- validate;
- construct IR;
- run passes in explicit order;
- record pass execution metadata;
- select backend;
- apply target-aware lowering;
- return structured artifacts/results.

The orchestrator must not hard-code IBM-specific logic.

---

# 7. Pass Manager Deliverable

Implement a reusable pass manager.

Required properties:

- explicit pass registration;
- explicit pass ordering;
- enable/disable capability;
- deterministic execution;
- pass metadata;
- before/after analysis hooks where required;
- error propagation;
- ability to run an ablation with selected passes disabled;
- compatibility with plugin-defined passes later.

The pass manager must treat pass ordering as meaningful.

Do not assume that running every available optimization is automatically optimal.

---

# 8. Optimization Pass Deliverables

The main optimization suite must include the core transformations identified by the project proposal/presentation.

## 8.1 Canonicalization

Normalize equivalent structural representations before deeper optimization.

Requirements:

- deterministic;
- semantics preserving;
- documented rules;
- regression tests.

## 8.2 Gate Cancellation

Implement provably safe cancellation for supported inverse gate pairs.

Examples include patterns such as:

`H ; H → identity`

`X ; X → identity`

and analogous supported inverses.

The implementation must not cancel across an intervening operation that changes the relevant wire semantics.

## 8.3 Gate Fusion

Fuse compatible consecutive operations according to a defined gate-fusion policy.

The implementation must document:

- supported fusion families;
- parameter composition;
- operand compatibility;
- maximum fusion scope;
- conditions that prevent unsafe fusion.

Do not claim arbitrary unitary synthesis unless it is actually implemented.

## 8.4 Rotation Merging

Merge compatible rotation sequences, especially for `Rx`, `Ry`, and `Rz`.

Parameter handling must preserve symbolic semantics under Stage F.

Examples may include:

`Rz(a); Rz(b) → Rz(a+b)`

provided the representation and operation semantics make this transformation valid.

## 8.5 DAG Scheduling

Use QCO-IR dependencies to expose parallelism while respecting:

- wire dependencies;
- control barriers;
- measurement/reset semantics;
- target constraints where scheduling is target-aware.

Do not reorder operations solely because they touch different qubits if an explicit dependency/barrier forbids it.

## 8.6 Qubit Mapping

Implement logical-to-physical mapping.

Required concepts:

- logical qubits;
- physical qubits;
- initial layout;
- mapping representation;
- mapping validity.

The mapper must consult the selected target profile.

## 8.7 Routing

Insert connectivity-repair operations where required.

At minimum:

- detect non-local two-qubit operations;
- select a routing strategy;
- insert SWAP operations or equivalent transformations;
- update logical/physical mapping;
- maintain correctness;
- report routing overhead.

Routing must not assume a universal device topology.

## 8.8 Basis Decomposition

Convert abstract operations into a target-supported basis.

The decomposition rules are supplied by the selected basis profile/target.

Do not encode an IBM basis into generic QC-IR.

---

# 9. IBM Target Deliverable

*Amended 2026-09-30 per [`framing-and-research-question.md`](framing-and-research-question.md).* Execution submission and result retrieval against a live IBM device are **outside the Definition of Done** (§34). Everything else in this section remains required.

*Status at `0.6.0`:* submission and result retrieval are implemented anyway, in the execution adapter `oqci.backends.ibm`. It was written against `qiskit-ibm-runtime` 0.50.0's installed source (§33.4), uses only the operator's own account, and is tested offline on a Heron device snapshot. CZ-native (Heron) devices are supported; ECR-native ones are refused. See [`../hardware.md`](../hardware.md).

IBM is the first real hardware target.

The implementation must have a distinct IBM backend/target module.

It must provide:

- backend configuration;
- target discovery or explicitly supplied target data;
- basis profile;
- connectivity;
- target legality checking;
- target lowering;
- execution preparation;
- execution submission;
- result retrieval;
- structured result metadata.

## 9.1 IBM Target Lowering

This stage is mandatory.

It exists because:

`QIR-valid`
does not necessarily mean
`IBM-executable`.

The IBM lowering stage is responsible for producing an executable representation that satisfies the current selected IBM backend constraints.

Do not bypass this stage merely because QIR or LLVM IR has already been produced.

## 9.2 Backend Availability

The selected IBM backend must be determined by configuration/runtime availability.

Do not hard-code one physical IBM device name into the compiler core.

## 9.3 Hardware Metadata

Record sufficient metadata to associate an execution with:

- backend name/identifier;
- target profile;
- compiler version;
- source circuit;
- optimization configuration;
- execution settings;
- raw result data.

Exact benchmarking policy remains Stage G.

---

# 10. Backend Abstraction Deliverable

At least the following backend classes/concepts should exist:

- simulator backend;
- IBM hardware backend;
- future-backend extension boundary.

The backend abstraction must distinguish:

- compilation/target preparation;
- execution;
- result retrieval.

Queue time and compiler transformation time must not be conflated.

---

# 11. Basis Profile Deliverable

Implement a first-class target profile.

Required conceptual fields:

```text
profile_id
profile_version
backend_id
physical_qubit_count
topology
supported_operations
parameter_constraints
decomposition_rules
measurement_constraints
reset_constraints
capabilities
cost_model_reference
```

The final Rust schema may differ in exact naming, but all required semantics must be representable.

Target profile data must be versionable/reproducible.

---

# 12. Cost Model Deliverable

Implement backend-defined target costs.

## 12.1 Contract

The cost model must support:

- candidate evaluation;
- structured cost breakdown;
- ordering/comparison;
- optional scalar objective.

## 12.2 Metrics

Retain at minimum:

- total gate count;
- one-qubit gate count;
- two-qubit gate count;
- depth;
- routing/SWAP overhead;
- target-native gate counts.

Where available, also support:

- execution-duration estimates;
- error/noise estimates;
- other backend-specific resource values.

## 12.3 Scalar Objective

A scalar objective may be used by an optimizer, but its derivation must be backend-defined and documented.

Do not bury arbitrary weights in optimization code.

## 12.4 Reproducibility

Record:

- backend ID;
- profile ID/version;
- cost-model ID/version;
- cost-model configuration.

---

# 13. Analysis and Validation Deliverable

The project presentation explicitly identifies analysis/tooling as a core component.

## 13.1 Gate Count

Provide:

- total count;
- one-qubit count;
- two-qubit count;
- target-native gate counts where available;
- per-wire metrics where practical.

## 13.2 Depth

Provide DAG-aware depth.

The implementation must define precisely what constitutes one scheduling layer and how measurements/reset are treated.

## 13.3 Resource Estimation

Provide a structured report covering:

- qubit count;
- classical-bit count;
- operation counts;
- depth;
- two-qubit burden;
- routing overhead;
- target-specific resources where supported.

## 13.4 Validation

Validate:

- IR invariants;
- frontend semantics;
- target legality;
- post-optimization structure;
- post-lowering backend compatibility.

## 13.5 Static Analysis/Linting

The main deliverable should support a modular analysis boundary even if the initial lint rule set is small.

Potential rules include:

- repeated/no-op patterns;
- suspicious qubit reuse;
- unsupported operation patterns;
- invalid target constructs;
- redundant operations.

Do not claim a full static-analysis research system unless the implemented rule set supports it.

---

# 14. QIR/LLVM Deliverable

The project must retain a well-defined lowering/output path into QIR/LLVM-compatible artifacts.

*Amended 2026-09-30 per [`framing-and-research-question.md`](framing-and-research-question.md).* QIR is OQCI's interoperability artifact, not its execution path (Stage C §5).

## 14.1 Current Emitter

The textual QIR emitter is part of the foundation. Up to `0.4.1` it wrote a `base_profile` label unconditionally and was never validated; since `0.5.0` it implements §14.2 and is validated as §14.3 describes. Its remaining limitations are documented in [`../qir_lowering.md`](../qir_lowering.md) and must not be hidden.

## 14.2 Target Specification

- **Version.** QIR 1.0, as published in `qir-alliance/qir-spec` at tag `1.0` (commit `e872897d107516d889718b5b6f5fee909dd64238`). Every claim of Base Profile conformance cites this commit.
- **Canonical output.** Each compilation produces at most one QIR module, emitted from the most-lowered circuit available (lowered if a backend was selected and lowering ran, otherwise optimized). It records which stage it came from. CLI, Python SDK and visualization server all use it; none emits QIR separately.
- **Profile labeling.** A module is labeled `"qir_profiles"="base_profile"` if and only if its circuit is legal against the `qir-base` target profile and the module follows the Base Profile structure. Otherwise it is labeled `"custom"`. A strict mode refuses to produce a non-Base module, with a typed error naming the violations.
- **The `qir-base` profile.** QIR 1.0 defines no normative quantum instruction set (`specification/Instruction_Set.md`: each backend declares its own). The `qir-base` basis is therefore pinned to the instruction set PyQIR 0.12.5 exposes (`pyqir/qis.py`), excluding `reset` and `barrier`. It is a registered compile target, so any supported circuit can be lowered onto it.
- **Pointer style.** Opaque pointers (`ptr`) by default; typed pointers (`%Qubit*`) on request.
- **Abstraction layers.** The path distinguishes abstract OQCI operations, target lowering (which may target `qir-base`), and QIR emission, which performs no decomposition of its own.

Where mature QIR/LLVM infrastructure can replace custom mechanics, prefer reuse.

## 14.3 Validation

QIR output must be validated to the extent supported by the chosen QIR tooling.

- External validators, pinned: PyQIR 0.12.5 (`Module.from_ir` and `verify`) and `llvm-as` from a pinned LLVM version for each pointer style. They run in CI.
- What each validator proves must be stated precisely. Parsing and verification by LLVM-based tools prove well-formed IR and readable QIR metadata; they do not prove Base Profile conformance. Base Profile structure checks written by OQCI itself are not independent evidence and must be described as such.
- Interoperability evidence is consumer-side: a named external tool, at a pinned version, accepted (and where available executed) the artifact.

Do not call arbitrary text "fully QIR compliant" without validation against the relevant specification/tooling.

---

# 15. Simulator Execution Deliverables

The project presentation names Qiskit Aer as an execution target and also identifies Cirq and CUDA-Q simulators.

## 15.1 Qiskit Aer

Implement a usable simulator execution path.

It must support:

- optimized circuit execution;
- optional noise-model execution where supported;
- measurement result retrieval;
- execution metadata;
- integration through the backend abstraction.

## 15.2 Cirq/CUDA-Q

Implement as resources allow and according to the project scope, but preserve the same backend contract.

The internal compiler should not need to know which SDK implements the simulator.

---

# 16. Parameterized-Circuit Deliverable

Support:

- `Rx(theta)`;
- `Ry(theta)`;
- `Rz(theta)`;
- concrete values;
- symbolic parameters;
- explicit parameter binding.

The supported circuit topology must remain static.

Dynamic control flow remains deferred.

---

# 17. Python SDK Deliverable

The presentation specifies Python as the SDK/integration/benchmarking layer.

Implement Python bindings only over stable Rust contracts.

The SDK should expose:

- circuit construction/import;
- compiler invocation;
- configuration;
- backend selection;
- analysis;
- optimization configuration;
- result/artifact access.

PyO3/maturin are suitable technologies already identified in project materials.

Do not expose internal Rust implementation details unnecessarily.

The Python API must have its own tests and documentation.

---

# 18. Plugin and Extension Deliverable

The final project must demonstrate that new components can be added without modifying the compiler core.

## 18.1 Frontend Plugin Boundary

A new source language should be able to map into QC-IR through a defined adapter contract.

## 18.2 Pass Plugin Boundary

A custom optimization/analysis pass must implement a stable interface and register with the pass manager.

The supplied presentation explicitly depicts:

`Third-party pass`
→ `Plugin SDK/interface`
→ `Pass Manager`

without core compiler modification.

## 18.3 Backend Plugin Boundary

A new backend should supply:

- target profile;
- lowering;
- execution interface;
- cost model.

The generic compiler should not contain vendor-specific branches.

## 18.4 Dynamic Libraries

A dynamic plugin mechanism may use a technology such as `libloading`, but only adopt this if the safety/versioning/ABI contract is actually specified.

Do not claim binary compatibility across arbitrary Rust compiler versions.

A process-local or source-level plugin API is acceptable during the first research deliverable if binary stability is not guaranteed.

---

# 19. CLI Deliverable

The presentation lists a future CLI concept.

A practical OQCI CLI should support commands conceptually equivalent to:

```text
oqci compile
oqci optimize
oqci analyze
oqci run
```

The exact commands/options can evolve.

The CLI must not duplicate compiler logic that belongs in the Rust library.

It should invoke the same public compiler APIs.

---

# 20. Benchmark Infrastructure Deliverable

Stage G is excluded from the locked experiment protocol, but the benchmark software infrastructure itself is part of the project.

Build:

- benchmark circuit loading/generation;
- deterministic metadata representation;
- compiler runner;
- backend runner abstraction;
- metric collection;
- result serialization;
- baseline runner;
- report generation;
- plotting/data-export utilities.

Do not hard-code the final Stage G numeric experiment matrix until Stage G is finalized.

## 20.1 Planned Circuit Families

The project materials identify:

- QFT;
- Grover;
- VQE/variational circuits;
- random/QASMBench-style circuits.

The exact final sizes, counts, seeds and repetition counts belong to Stage G.

## 20.2 Planned Baselines

The materials identify:

- Qiskit transpiler at optimization level 3;
- t|ket|;
- OQCI with selected optimizations disabled for ablation.

A Qiskit level-0 reference may be added as a control, but that is an experimental-design choice rather than a locked requirement in this document.

## 20.3 Result Schema

Each benchmark record should retain enough information to reproduce the run:

```text
benchmark_id
circuit_id
circuit_family
circuit_size
source_format
compiler
compiler_version
git_commit
frontend
pass_pipeline
backend
target_profile
cost_model
compilation_time
metrics
execution_metadata
result_artifact_paths
```

The final schema can be expanded.

---

# 21. Visualization Deliverable

Provide programmatic visualization of:

- circuit structure where practical;
- DAG/dependency structure;
- before/after optimization metrics;
- pass-by-pass changes;
- benchmark distributions;
- baseline comparisons.

Plots must be generated from serialized benchmark data, not manually edited.

---

# 22. Reproducibility Deliverable

A complete result should be traceable to source.

Record:

- repository commit;
- compiler version;
- configuration;
- frontend;
- target profile;
- cost model;
- optimization pipeline;
- circuit identifier;
- parameter values;
- backend information;
- artifact versions.

Use deterministic seeds where randomness is required.

The exact benchmark seed/count policy remains Stage G.

---

# 23. Testing Deliverable

Testing is not optional.

## 23.1 Rust Tests

Maintain unit/integration coverage across:

- IR;
- conversion;
- frontend adapters;
- pass manager;
- every optimization pass;
- routing;
- target lowering;
- backend validation;
- QIR emission;
- parameter binding;
- compiler orchestration.

## 23.2 Property Tests

Use property-based testing where it gives clear value, particularly for:

- transformations preserving structure/semantics;
- parser edge cases;
- routing correctness;
- parameter transformations.

## 23.3 Fuzzing

Parser fuzzing should be considered part of robustness work.

Do not claim fuzzing coverage until actual fuzz targets exist and have been exercised.

## 23.4 Snapshot Tests

Snapshot tests may be used for:

- QIR;
- diagnostics;
- serialized IR;
- benchmark reports.

## 23.5 Undefined-Behavior/Correctness Tooling

The presentation names tools such as:

- `miri`;
- `cargo-fuzz`;
- `proptest`;
- `cargo-audit`;
- `cargo-deny`.

Use them when compatible with the implemented subsystem and project toolchain.

---

# 24. Performance Deliverable

The project guide gives a target of compilation below roughly 100 ms for circuits up to 16 qubits.

Treat that as a performance objective, not an unconditional guarantee.

Benchmark:

- frontend time;
- IR construction;
- optimization;
- target lowering;
- total compilation.

Do not include backend queue time in compiler timing.

A performance regression must be measurable from benchmark artifacts.

---

# 25. Documentation Deliverable

At minimum the repository should contain accurate documentation for:

- architecture;
- QC-IR;
- QCO-IR;
- frontend contracts;
- pass interface;
- pass guide;
- backend contract;
- basis profiles;
- target lowering;
- cost model;
- plugin SDK;
- Python API;
- CLI;
- benchmark infrastructure;
- reproducibility;
- examples;
- limitations.

Any "supported operation" list must correspond to actual code.

Do not document future functionality as though it already exists.

---

# 26. CI/CD Deliverable

CI must execute the project's core quality gates.

At minimum:

- Rust build;
- Rust tests;
- formatting;
- Clippy/lints;
- Python tests when Python is present;
- Python formatting/type checks where configured;
- documentation build where practical.

Later hardware tests must not run automatically in ordinary CI unless credentials and resource policy make this explicitly safe.

Hardware experiments should be manually triggered or separately configured.

---

# 27. Packaging Deliverable

The project materials identify:

- Cargo;
- maturin/PyPI;
- Docker;
- reproducible development environments.

The release design should distinguish:

- Rust crate;
- Python wheel/package;
- optional CLI artifact;
- documentation build;
- benchmark/research package.

Do not claim multi-platform packaging until CI/release workflows actually support it.

---

# 28. Logging and Diagnostics

Compiler errors should be structured and actionable.

The project materials mention:

- `thiserror`;
- `miette`;
- `log`/`env_logger`;
- `tracing`.

Choose an appropriate combination based on actual implementation.

Do not introduce multiple logging systems without a clear reason.

Diagnostics should identify:

- stage;
- operation;
- qubit/classical operand;
- backend/target where relevant;
- source location where the frontend provides one.

---

# 29. Security and Dependency Hygiene

Maintain:

- dependency auditing;
- denied dependency/licensing policies where practical;
- no secrets committed to repository;
- backend credentials supplied through environment/configuration, not source;
- safe parsing of untrusted input.

IBM credentials must never be embedded in benchmark fixtures or committed configuration.

---

# 30. Research Package Deliverable

The final research package must be capable of producing:

- benchmark tables;
- figures;
- raw machine-readable results;
- benchmark configuration;
- compiler configuration;
- comparison outputs;
- ablation results;
- documentation;
- paper-ready artifacts.

The package must make it possible to reconstruct how a result was obtained from a repository commit.

## 30.1 Research Claims Must Be Evidence-Bound

Do not make claims such as:

- "OQCI is faster";
- "OQCI is more accurate";
- "OQCI gives higher fidelity";
- "OQCI outperforms Qiskit";

unless the implemented benchmark evidence supports the specific claim.

The experiment protocol itself will be finalized separately as Stage G.

---

# 31. Final Deliverables Checklist

The final non-G deliverable should contain, in a working and tested state, the
items below. Each box was ticked at `0.6.0` (2026-10-20) with the evidence
that backs it. An item that is out of scope says so, and by whose decision.

### Compiler core
- [x] Stable Rust compiler library — `src/`, `cargo test --workspace`.
- [x] QC-IR — `src/ir/qc.rs`, `tests/validation.rs`.
- [x] QCO-IR — `src/ir/qco.rs`, `tests/qco_structure.rs`.
- [x] deterministic conversions — `src/ir/convert.rs`, `tests/roundtrip.rs`, `docs/determinism.md`.
- [x] compiler orchestrator — `src/compile.rs`, `docs/compiler.md`.
- [x] configuration model — `CompilerConfig`, `benchmarks/configs/`.

### Frontends
- [x] OpenQASM 3 supported subset documented and implemented — `docs/openqasm_frontend.md`, `tests/openqasm_pipeline.rs`.
- [x] Qiskit adapter — `src/frontend/qiskit/`, `python/src/lib.rs`, `tests/qiskit_adapter.rs`.
- [x] Cirq adapter, to the committed scope — **the committed scope is none**, by the project owner's decision for `0.6.0` (§5.4).
- [x] CUDA-Q adapter, to the committed scope — `src/frontend/cudaq/`, `docs/cudaq_frontend.md`.
- [x] frontend plugin boundary — `extension::FrontendPlugin`, `docs/extensions.md`.
- [x] validation — I1–I8 at construction, `ir::MAX_REGISTER_WIDTH`, `tests/limits.rs`, fuzzing (`fuzz/`).

### Optimization
- [x] pass interface — `pass::Pass`.
- [x] pass manager — `pass::PassManager`, `docs/pass_manager.md`.
- [x] canonicalization — `pass::canonicalize`.
- [x] gate cancellation — `pass::cancellation`.
- [x] gate fusion — `pass::fusion` (single-qubit runs; policy in `docs/pass_manager.md`).
- [x] rotation merging — `pass::rotation_merge`.
- [x] DAG scheduling — `pass::schedule`.
- [x] qubit mapping — `lowering::layout`.
- [x] routing/SWAP insertion — `lowering::routing`.
- [x] basis decomposition — `lowering::rules`, `docs/lowering.md`.
- [x] pass enable/disable configuration — `PassSelection`, `--passes`/`--disable`.
- [x] pass metadata — `PassRecord`.

### Target abstraction
- [x] backend interface — `backend::Backend`, `docs/backend_contract.md`.
- [x] target/basis profile — `target::BasisProfile`, `docs/target_model.md`.
- [x] topology model — `target::Topology`.
- [x] target legality validation — `target::check`.
- [x] target-specific lowering — `lowering::lower`, re-checked by its own verification step.
- [x] backend-defined cost model — `target::cost`.

### IBM
- [x] IBM backend module — `src/backend/ibm.rs`, `python/oqci/backends/ibm.py`.
- [x] IBM target profile — from the device's live target (`target_from_backend`) or a file (`from_target_json`); `cx-to-cz` for Heron.
- [x] IBM target lowering — tested against Heron snapshots (`python/tests/test_backend_ibm.py`).
- [x] execution preparation — `Backend::prepare`, ISA-checked against the device (`check_isa`).
- [x] provenance metadata — `Provenance`, carried into every result.
- Live submission and result retrieval: outside the Definition of Done (§9, §34), and **implemented** in `0.6.0` (`oqci.backends.ibm`, `docs/hardware.md`), tested offline; live runs are operator-run.

### Execution
- [x] QIR/LLVM artifact generation — `src/ir/qir.rs`, `docs/qir_lowering.md`, external validation in CI.
- [x] Qiskit Aer backend — `oqci.backends.aer`, `python/tests/test_aer.py`.
- [x] Cirq simulator backend, to committed scope — none committed (see Frontends).
- [x] CUDA-Q simulator backend, to committed scope — `oqci.backends.cudaq`, checked against a real CUDA-Q in CI.
- [x] backend abstraction shared by simulation/hardware — one `Backend` trait; one adapter layout (`docs/adapters.md`).

### Parameters
- [x] symbolic/numeric `Rx` — `ir::Param`.
- [x] symbolic/numeric `Ry`.
- [x] symbolic/numeric `Rz`.
- [x] explicit parameter binding — `ir::bind_parameters`, checked by `verify::check_binding`.
- [x] static topology guarantee — Stage F; binding preserves structure exactly.

### Analysis
- [x] total gate count — `analysis::ResourceReport`.
- [x] one-qubit count.
- [x] two-qubit count.
- [x] depth.
- [x] routing overhead — `Lowered::swaps_inserted`, `Cost::swap_count`; uniformly derived in the benchmark report.
- [x] target-native gate counts — `Cost::native_gate_count`.
- [x] resource report — `oqci analyze`, the SDK's metrics.
- [x] validation/lint boundary — `analysis::lint`, `oqci lint`, `docs/verification.md`.

### Python/API
- [x] PyO3 bindings — `python/src/lib.rs`.
- [x] Python packaging — `python/pyproject.toml` (extras `aer`, `cudaq`, `ibm`), wheels built in CI.
- [x] compiler API — `oqci.compile`, `oqci.lint`, `oqci.load_target`.
- [x] analysis API — metrics, lint, verification in the result.
- [x] backend API — `oqci.backends`, `to_execution_result`, `python -m oqci run`.
- [x] documentation — `python/README.md`, `docs/adapters.md`, `docs/hardware.md`.

### Plugins
- [x] pass extension contract — `Registry::register_pass`, `PassPlacement`.
- [x] frontend extension contract — `extension::FrontendPlugin`.
- [x] backend extension contract — `Registry::register_backend`, `register_target_json`.
- [x] configuration mechanism — `CompilerConfig::extensions`, `cli::run_with`.
- [x] at least one demonstration extension — `examples/plugin-demo`, `examples/plugin-demo/tests/no_core_modification.rs`.

### Benchmark infrastructure
- [x] benchmark runner — `benchmarks/oqci_bench/runner.py`.
- [x] deterministic benchmark metadata — seeded items, `environment.json`, the record schema.
- [x] metric collector — `oqci_bench/metrics.py`.
- [x] baseline runner — Qiskit O3/O0 and t|ket⟩ (`oqci_bench/compilers/`).
- [x] result serializer — `records.jsonl`, `schema/result.schema.json`.
- [x] plotting/report generation — `oqci_bench/report.py`.
- [x] reproducible artifact structure — one run directory per run, `docs/benchmarks.md`.

### Engineering quality
- [x] unit tests.
- [x] integration tests.
- [x] property tests where useful — `pass_equivalence`, `lowering_equivalence`, `verifier`.
- [x] parser fuzzing where applicable — `fuzz/` (five targets; coverage-guided and mutation), in CI. See `docs/fuzzing.md`.
- [x] snapshots where useful — CLI and QIR output tests.
- [x] formatting/linting — `cargo fmt`, `clippy -D warnings`.
- [x] dependency/security checks — `deny.toml`, cargo-deny and cargo-audit in CI.
- [x] CI — `.github/workflows/ci.yml`.
- [x] documentation build — `.github/workflows/docs.yml` (mdBook).

### Research package
- [ ] raw benchmark data — `records.jsonl` of the reference run. *Status 2026-10-21:* the `0.6.0` run stopped on a t\|ket⟩ defect and was dropped (`d42b413`). It is being rerun in full on `0.7.1`, and this box and the three below are re-ticked when it is committed.
- [x] machine-readable result schema — `benchmarks/schema/result.schema.json`.
- [ ] generated figures — `report/figures/` of the reference run (the generator is tested by the `benchmarks (smoke)` CI job).
- [ ] generated tables — `report/tables/` (CSV, Markdown, LaTeX) of the reference run.
- [x] experiment configurations — `benchmarks/configs/stage_g.toml`.
- [x] ablation framework — the OQCI ablation configurations.
- [x] reproducibility instructions — `docs/benchmarks.md`, `Dockerfile`.
- [ ] paper-supporting artifacts — the reference run's `report/`.

---

# 32. Explicit Non-Goals for the Main Locked Scope

Do not implement these as required deliverables unless the project explicitly reopens scope:

- arbitrary runtime dynamic circuits;
- measurement-conditioned quantum control flow;
- a new quantum programming language;
- a complete general-purpose classical compiler;
- distributed quantum computing runtime;
- HPC orchestration;
- neutral-atom-specific optimization;
- photonic-specific optimization;
- full fault-tolerant T-count/T-depth optimization as the primary target;
- ML/RL-based optimization as a required first implementation;
- automatic pass selection as a required first implementation;
- full AI-driven compiler optimization;
- arbitrary pulse-level compilation;
- a custom quantum simulator replacing established simulator frameworks.

These can appear in future research documentation but must not be allowed to destabilize the main compiler deliverable.

---

# 33. Anti-Hallucination Rules for Coding Agents

Any LLM implementing this repository must follow these rules.

1. Inspect the actual current repository before modifying it.
2. Do not infer that a named file/module exists because a planning document names it.
3. Do not assume an API from Qiskit/Cirq/CUDA-Q without checking the currently supported dependency version.
4. Do not implement a vendor-specific API from memory when the current SDK documentation can be checked.
5. Do not silently expand the supported gate set.
6. Do not silently add dynamic-circuit semantics.
7. Do not embed IBM-specific assumptions into generic IR or generic passes.
8. Do not replace the backend contract with direct SDK calls throughout the compiler.
9. Do not introduce MLIR as a rewrite of the Rust core unless Stage B is explicitly being implemented.
10. Do not delete or weaken existing IR invariants to make an adapter easier.
11. Do not claim QIR compliance without validation.
12. Do not claim hardware executability without target-specific lowering/validation.
13. Do not hard-code arbitrary optimization weights.
14. Do not optimize away operations across measurement/reset/control barriers without proving the transformation safe.
15. Do not add benchmark numbers, random seeds, repetitions, hardware-selection rules, or experimental conclusions that belong to Stage G.
16. When an implementation requirement is genuinely ambiguous, stop and ask for a design decision rather than inventing one.
17. Any claim about QIR conformance or interoperability must cite the QIR specification commit and the validator names and versions it rests on.

---

# 34. Definition of Done

The non-G project is complete only when the implementation is demonstrably able to take supported external/static parameterized circuits through the compiler stack:

`frontend`
→ `QC-IR`
→ `QCO-IR`
→ `configured pass pipeline`
→ `backend/target selection`
→ `basis-profile validation`
→ `target lowering`
→ `backend executable representation` (plus the canonical QIR interop artifact)
→ `simulator execution adapter`

and produce:

1. a valid optimized circuit;
2. structured analysis metrics;
3. target-specific lowering artifacts;
4. executable simulator output;
5. an IBM target path through lowering, validation and execution preparation — live IBM submission and result retrieval are outside this definition (amended 2026-09-30), and have been implemented anyway since `0.6.0` (§9);
6. reproducible machine-readable metadata;
7. passing automated tests;
8. documentation matching actual implementation;
9. extension points that demonstrate the claimed modular architecture.

The benchmark experiment design and IBM experimental matrix are outside this definition. They belong to Stage G, locked on 2026-10-20 ([`stage-g-benchmarking-protocol.md`](stage-g-benchmarking-protocol.md)).

---

# 35. Source Basis

Primary project sources used for this specification:

- Current repository: `https://github.com/AndyFerns/oqci`
- Current repository changelog: `https://github.com/AndyFerns/oqci/blob/master/CHANGELOG.md`
- Current repository IR tree: `https://github.com/AndyFerns/oqci/tree/master/src/ir`
- Existing no-frontend ADR: `https://github.com/AndyFerns/oqci/blob/master/docs/architecture_decision_no_frontend.md`
- Existing MLIR ADR: `https://github.com/AndyFerns/oqci/blob/master/docs/architecture_decision_mlir_phase2.md`
- Supplied OQCI presentation PDF: `Open Quantum Compiler Infrastructure.pdf`
- OQCI project proposal/development guide and related architecture presentations supplied with the project materials.

The supplied presentation's page 8 is the main visual architecture reference: it shows the frontend-adapter layer, QC-IR, QCO-IR, pass manager, QIR/LLVM lowering, execution backends, analysis/tooling, plugin infrastructure, and final deliverables. Its page 11 provides the technology-stack decomposition, including Rust, Python/PyO3/maturin, MLIR/LLVM as a phased layer, backend abstraction, simulators, IBM hardware, testing, benchmarking infrastructure, documentation, CI, packaging, and plugin mechanisms.

The supplied presentation's pages 12–13 treat dynamic circuits and other larger capabilities as future scope, which is consistent with the locked Stage F boundary.
