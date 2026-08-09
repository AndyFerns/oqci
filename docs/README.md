# OQCI Documentation

Documentation for **OQCI** (Open Quantum Compiler Infrastructure). This index
covers the **Phase 0** deliverable: the IR core, its conversions, and QIR
lowering. There are deliberately **no frontends** and **no backend execution** in
this phase (see the no-frontend ADR).

## Pipeline at a glance

```text
        build              convert            lower
QC-IR  ───────▶  Circuit ─────────▶  QCO-IR ────────▶  QIR (LLVM-compatible text)
(imperative)     (validated)         (DAG)             (Base Profile, textual)
```

## Documents

| Document | What it covers |
|----------|----------------|
| [`ir_spec.md`](ir_spec.md) | **Normative IR reference.** Value types, QC-IR + QCO-IR ops and invariants (I1–I7), operational semantics, and the semantics-preservation proof for QC-IR → QCO-IR. |
| [`mlir_dialect.md`](mlir_dialect.md) | **The `quantum` MLIR dialect spec.** Types, ops, attribute-vs-operand rules, the complete op↔Rust correspondence table, the Phase 2 integration path, and the implied TableGen skeleton. |
| [`qir_lowering.md`](qir_lowering.md) | **Lowering rules.** Target QIR format, the op → QIR intrinsic mapping table, angle/qubit encoding, and the two documented conformance caveats (extended intrinsics, mid-circuit measurement). |
| [`architecture_decision_no_frontend.md`](architecture_decision_no_frontend.md) | **ADR.** Why no frontend is built before the IR is stable, and how to resist adding one early. |
| [`architecture_decision_mlir_phase2.md`](architecture_decision_mlir_phase2.md) | **ADR (binding constraints).** Why pure Rust now, what MLIR adds later, and the constraints C1–C8 that must not change to keep the Phase 2 port mechanical. |

## Source map

| Area | Crate module |
|------|--------------|
| Shared value types (`QubitId`, `ClbitId`, `Angle`, `GateKind`) | `src/ir/types.rs` |
| QC-IR (`Circuit`, `Instruction`, `CircuitBuilder`) | `src/ir/qc.rs` |
| QCO-IR (DAG, deterministic toposort) | `src/ir/qco.rs` |
| QC-IR → QCO-IR conversion | `src/ir/convert.rs` |
| QCO-IR → QIR lowering | `src/ir/qir.rs` |
| Error type (`IrError`) | `src/ir/error.rs` |
| Phase 2 MLIR seam (empty by design) | `src/ir/mlir_compat.rs` |
| End-to-end + error-path tests | `tests/pipeline.rs` |

## Verifying Phase 0

```bash
cargo build
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test
```

## Reading order

New contributors: start with [`ir_spec.md`](ir_spec.md) for the IR itself, then
[`qir_lowering.md`](qir_lowering.md) for the output boundary. Before touching any
core IR type, read
[`architecture_decision_mlir_phase2.md`](architecture_decision_mlir_phase2.md) —
its constraints C1–C8 are what keep the future MLIR port cheap.
