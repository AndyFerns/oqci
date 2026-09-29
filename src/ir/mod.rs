//! The OQCI intermediate representation.
//!
//! This module houses the two IR levels and the transformations between them:
//!
//! - [`qc`] — **QC-IR**, the imperative circuit IR ([`Circuit`], [`Instruction`],
//!   built via [`CircuitBuilder`]).
//! - [`qco`] — **QCO-IR**, the optimization IR: a directed acyclic dependency
//!   graph ([`QcoCircuit`]) supporting topological traversal.
//! - [`convert`] — deterministic, semantics-preserving QC-IR → QCO-IR
//!   conversion.
//! - [`qir`] — lowering from QCO-IR to QIR (LLVM-compatible textual IR).
//! - [`types`] — shared value types: [`QubitId`], [`ClbitId`], [`Angle`],
//!   [`GateKind`].
//! - [`param`] — [`Param`], a gate parameter that is either a concrete angle
//!   or a symbolic placeholder (Stage F).
//! - [`bind`] — [`bind_parameters`], the explicit symbolic → concrete step
//!   that must precede lowering.
//! - [`error`] — the [`IrError`] type returned by all fallible IR operations.
//! - [`mlir_compat`] — the (currently empty) Phase 2 MLIR integration seam.
//!
//! The types obey the IR contract (C1–C8,
//! `docs/architecture_decision_ir_contract.md`), which makes every `Circuit` a
//! witness of invariants I1–I8. They also mirror the `quantum` MLIR dialect
//! one-to-one, so a future MLIR port would be a mechanical translation; see
//! `docs/mlir_dialect.md`.

pub mod bind;
pub mod convert;
pub mod error;
pub mod mlir_compat;
pub mod param;
pub mod qc;
pub mod qco;
pub mod qir;
pub mod types;

pub use bind::bind_parameters;
pub use convert::qc_to_qco;
pub use error::IrError;
pub use param::Param;
pub use qc::{Circuit, CircuitBuilder, Instruction};
pub use qco::{DepKind, NodeKind, QcoCircuit, QcoNode, Wire};
pub use qir::emit_qir;
pub use types::{Angle, ClbitId, GateKind, QubitId};
