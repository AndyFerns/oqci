# OQCI Documentation

Documentation for **OQCI** (Open Quantum Compiler Infrastructure). This index
covers the IR core, its conversions, QIR lowering, and the frontend layer
(OpenQASM 3 and Qiskit). There is still **no backend execution** and there are
**no optimization passes**; QIR emission remains the output boundary.

## Pipeline at a glance

```text
  parse / adapt          build            convert           lower
source ─────────▶ QC-IR ───────▶ Circuit ────────▶ QCO-IR ────────▶ QIR
(QASM 3, Qiskit)  (imperative)   (validated)       (DAG)      (Base Profile, textual)
                                      ▲
                                      │ bind_parameters
                              symbolic circuits (Stage F)
```

## Documents

| Document | What it covers |
|----------|----------------|
| [`ir_spec.md`](ir_spec.md) | **Normative IR reference.** Value types, parameters (`Param`) and binding, QC-IR + QCO-IR ops and invariants (I1–I8), operational semantics, and the semantics-preservation proof for QC-IR → QCO-IR. |
| [`gate_mapping.md`](gate_mapping.md) | **The shared source-name → `GateKind` table** used by both frontends, and the rule that unknown names become `Opaque` rather than new enum variants. |
| [`openqasm_frontend.md`](openqasm_frontend.md) | **The OpenQASM 3 supported subset**, precisely: what is accepted, what is refused and why, broadcast rules, and diagnostics. |
| [`qiskit_adapter.md`](qiskit_adapter.md) | **The Qiskit adapter.** The vendor-neutral `QiskitCircuitIr` handoff, the PyO3 boundary, the verified Qiskit version, and known limitations. |
| [`mlir_dialect.md`](mlir_dialect.md) | **The `quantum` MLIR dialect spec.** Types, ops, attribute-vs-operand rules, the complete op↔Rust correspondence table, the Phase 2 integration path, and the implied TableGen skeleton. |
| [`qir_lowering.md`](qir_lowering.md) | **Lowering rules.** Target QIR format, the op → QIR intrinsic mapping table, angle/qubit encoding, and the two documented conformance caveats (extended intrinsics, mid-circuit measurement). |
| [`architecture_decision_no_frontend.md`](architecture_decision_no_frontend.md) | **ADR.** Why no frontend is built before the IR is stable, and how to resist adding one early. |
| [`architecture_decision_mlir_phase2.md`](architecture_decision_mlir_phase2.md) | **ADR (binding constraints).** Why pure Rust now, what MLIR adds later, and the constraints C1–C8 that must not change to keep the Phase 2 port mechanical. |

## Source map

| Area | Crate module |
|------|--------------|
| Shared value types (`QubitId`, `ClbitId`, `Angle`, `GateKind`) | `src/ir/types.rs` |
| Gate parameters (`Param`) | `src/ir/param.rs` |
| Parameter binding (`bind_parameters`) | `src/ir/bind.rs` |
| QC-IR (`Circuit`, `Instruction`, `CircuitBuilder`) | `src/ir/qc.rs` |
| QCO-IR (DAG, deterministic toposort) | `src/ir/qco.rs` |
| QC-IR → QCO-IR conversion | `src/ir/convert.rs` |
| QCO-IR → QIR lowering | `src/ir/qir.rs` |
| Error type (`IrError`) | `src/ir/error.rs` |
| Phase 2 MLIR seam (empty by design) | `src/ir/mlir_compat.rs` |
| Frontend contract (`FrontendError`) | `src/frontend/error.rs` |
| Shared gate-name table | `src/frontend/gate_map.rs` |
| OpenQASM 3 frontend (lexer/parser/AST/translate) | `src/frontend/openqasm/` |
| Qiskit adapter core (no Python) | `src/frontend/qiskit/mod.rs` |
| Qiskit PyO3 boundary | `python/src/lib.rs` |
| End-to-end + error-path tests | `tests/pipeline.rs`, `tests/openqasm_pipeline.rs`, `tests/qiskit_adapter.rs` |

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
