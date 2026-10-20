# Framing and Research Question

Status: LOCKED  
Date: 2026-09-30  
Decision: **OQCI is quantum-native compiler infrastructure first, and LLVM/QIR-interoperable second.**

## 1. Framing

> OQCI builds upon the broader LLVM/QIR quantum compilation ecosystem but
> explores a different architectural boundary: quantum-native compiler
> infrastructure first, LLVM interoperability second.

This replaces the earlier framing of OQCI as "a Q-LLVM-inspired,
vendor-neutral compiler architecture". It is a change of framing and scope,
not a rewrite: the code already has the shape this record describes (§3).

Vendor neutrality is kept as a *property* of the design — no frontend or
backend type reaches the IR, and no vendor branch appears in the compiler
core (Stage C §6, Stage D) — but it is no longer the project's headline claim.

## 2. Research question

> Can a quantum-first, strongly validated intermediate representation provide
> a clean foundation for deterministic quantum compiler transformations while
> remaining interoperable with established LLVM/QIR infrastructure?

Each part of the question maps onto something the repository must be able to
show evidence for:

| Part of the question | What it refers to | Where it is specified |
|---|---|---|
| quantum-first IR | QC-IR and QCO-IR are the authoritative representation; nothing downstream redefines their semantics | [`../ir_spec.md`](../ir_spec.md), Stage A |
| strongly validated | the IR contract: invariants I1–I8, checked totally at construction, and the constraints C1–C8 that keep them checkable | [`../architecture_decision_ir_contract.md`](../architecture_decision_ir_contract.md) |
| deterministic transformations | a defined determinism contract over the compiler's semantic outputs | to be specified (planned; see [`index.md`](index.md)) |
| interoperable with LLVM/QIR | QIR emitted at the output boundary, labeled honestly, and accepted by external tools | [`final-deliverables-spec.md`](final-deliverables-spec.md) §14 |

## 3. The hypothesis this framing rests on, and its verification

The framing assumes the Rust IR is already authoritative and QIR is already
only an emission boundary. That was checked against the code at
`0.4.1` (2026-09-30):

- **The Rust IR is authoritative.** `src/ir/mlir_compat.rs` contains no code,
  and the crate has no MLIR or LLVM dependency (`Cargo.toml`).
- **QIR is output-only.** Nothing in the repository parses QIR. `src/backend/`
  has no QIR code path; backends consume `Executable`, and the Qiskit Aer
  adapter reads `Executable.ops` (`python/oqci/backends/aer.py`).
- **QIR is off the orchestrated path.** `compile::compile_circuit` never emits
  QIR, and `CompilationArtifacts` carries none. QIR is emitted separately, by
  the CLI, the visualization server and the Python SDK, each from a different
  circuit ([`../qir_lowering.md`](../qir_lowering.md) §1.4).

The last point is recorded because the interop boundary is currently three
boundaries. Making QIR a single canonical compile output, emitted from the
most-lowered circuit available, is part of this decision (§5).

## 4. What OQCI claims, and what it does not

OQCI claims only what its tests and validators show.

- It does **not** claim QIR interoperability beyond what named external tools,
  at pinned versions, have been observed to accept.
- It does **not** claim that emitted QIR runs on any hardware.
- It does **not** claim hardware executability of any kind; live IBM
  submission is outside the Definition of Done
  ([`final-deliverables-spec.md`](final-deliverables-spec.md) §34).
- It does **not** claim performance or quality advantages over other
  compilers without Stage G evidence (§30.1 of the final spec).

## 5. Consequences

The decisions below follow from this framing. Each is recorded in the
document named; this list is an index, not a second copy.

| Decision | Recorded in |
|---|---|
| The IR contract (C1–C8, I1–I8) is justified as an IR-validity guarantee; MLIR portability is a secondary benefit. | [`../architecture_decision_ir_contract.md`](../architecture_decision_ir_contract.md) |
| MLIR remains optional enabling infrastructure; Stage B's contribution clause defers to this record. | [`stage-b-modular-mlir-boundary.md`](stage-b-modular-mlir-boundary.md), [`../architecture_decision_mlir_phase2.md`](../architecture_decision_mlir_phase2.md) |
| QIR is an interoperability artifact: one canonical output per compilation, emitted from the most-lowered circuit available, targeting QIR 1.0. | [`final-deliverables-spec.md`](final-deliverables-spec.md) §14 |
| A module is labeled `base_profile` only if its circuit is legal against a `qir-base` target profile; otherwise it is labeled `custom`. A strict mode refuses instead. | [`final-deliverables-spec.md`](final-deliverables-spec.md) §14 |
| External validation (PyQIR and `llvm-as`, pinned) runs in CI; interoperability evidence is consumer-side. | [`final-deliverables-spec.md`](final-deliverables-spec.md) §14, §26 |
| Ingesting QIR is future work, not a deliverable. | [`final-deliverables-spec.md`](final-deliverables-spec.md) §5.7 |
| Stage G gains validation-strength, determinism and interoperability metric definitions; hardware experiments become secondary. | [`stage-g-benchmarking-protocol.md`](stage-g-benchmarking-protocol.md) |
| `0.0.1` baselines in locked documents are historical snapshots; [`index.md`](index.md) is the only living baseline. | Stage A §2, final spec §2 |
| "Phase n" labels are historical and mapped to versions; they are not used for future work. | [`index.md`](index.md) |

## 6. Precedence

This record is a locked decision in the sense of the source hierarchy in
[`index.md`](index.md). Where an older locked document states the project's
identity or main contribution differently, this record takes precedence; the
older wording is kept only where it is marked as historical.
