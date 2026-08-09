//! MLIR compatibility boundary — **intentionally empty in Phase 0.**
//!
//! This module exists so that the module boundary for MLIR integration is
//! established *now*, before Phase 2, rather than bolted on later. It is the
//! single seam through which the future MLIR port will flow, keeping the rest of
//! the IR (`qc`, `qco`, `convert`, `qir`) free of any MLIR dependency today.
//!
//! # Phase 2 role
//!
//! When the MLIR integration lands (see
//! `docs/architecture_decision_mlir_phase2.md`), this module will host:
//!
//! - Conversions between OQCI's pure-Rust IR types and the C++/TableGen-defined
//!   `quantum` dialect ops, types, and attributes described in
//!   `docs/mlir_dialect.md`.
//! - The `melior`/FFI glue that materialises [`crate::ir::GateKind`],
//!   [`crate::ir::Instruction`], and [`crate::ir::QubitId`] as `quantum.gate`,
//!   `quantum.measure`, `quantum.reset`, and typed SSA values.
//! - Round-trip verification that the Rust IR and the MLIR module denote the
//!   same circuit.
//!
//! # Binding constraint
//!
//! The mapping is designed to be *mechanical*. The Rust type boundaries that
//! must remain stable for that to hold (the `GateKind` closed-enum + `Opaque`
//! escape hatch, `QubitId`/`ClbitId` newtypes, the `Angle` parameter type, and
//! parameters-as-attributes / qubits-as-operands separation) are enumerated in
//! `docs/architecture_decision_mlir_phase2.md`. Do not change them without
//! updating that ADR.
