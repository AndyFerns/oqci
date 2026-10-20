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
//! - [`mlir_compat`] — the (empty) MLIR integration seam; MLIR is optional.
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
pub use param::{ANGLE_ROUNDING_LIMIT, MAX_EVALUATED_ANGLE, Param, exact_affine};
pub use qc::{Circuit, CircuitBuilder, Instruction};
pub use qco::{DepKind, NodeKind, QcoCircuit, QcoNode, Wire};
pub use qir::{PointerStyle, QirOptions, QirProfile, emit_qir, emit_qir_with};
pub use types::{Angle, ClbitId, GateKind, QubitId};

/// The widest qubit or classical register any untrusted input may declare.
///
/// A structural safety limit (`final-deliverables-spec.md` §29: safe parsing
/// of untrusted input), not a statement about hardware: it is far above any
/// device that exists (IBM's largest current processors have a few hundred
/// qubits), and far below what would exhaust memory. Every boundary that reads
/// a width from outside the process — the OpenQASM 3 and CUDA-Q frontends,
/// the Qiskit handoff, and a target profile's topology — refuses anything
/// wider *before* allocating, so a hostile `qubit[4294967295]` is a typed
/// error rather than an allocation failure. Found by the fuzz corpus.
pub const MAX_REGISTER_WIDTH: u32 = 1 << 16;
