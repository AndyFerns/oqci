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
//! - [`error`] — the [`IrError`] type returned by all fallible IR operations.
//! - [`mlir_compat`] — the (currently empty) Phase 2 MLIR integration seam.
//!
//! The design mirrors the `quantum` MLIR dialect one-to-one so that Phase 2 is a
//! mechanical translation; see `docs/mlir_dialect.md` and
//! `docs/architecture_decision_mlir_phase2.md`.

pub mod convert;
pub mod error;
pub mod mlir_compat;
pub mod qc;
pub mod qco;
pub mod qir;
pub mod types;

pub use convert::qc_to_qco;
pub use error::IrError;
pub use qc::{Circuit, CircuitBuilder, Instruction};
pub use qco::{DepKind, NodeKind, QcoCircuit, QcoNode, Wire};
pub use qir::emit_qir;
pub use types::{Angle, ClbitId, GateKind, QubitId};
