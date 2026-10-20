//! Frontend adapters: external programs → QC-IR.
//!
//! Every frontend implements the same contract required by
//! `docs/core_architecture/final-deliverables-spec.md` §5.1:
//!
//! ```text
//! external program → validated QC-IR
//! ```
//!
//! Two properties make that contract load-bearing rather than decorative:
//!
//! - **No vendor types escape.** A caller downstream of a frontend receives a
//!   [`crate::ir::Circuit`] and a [`FrontendError`] — never an OpenQASM AST, a
//!   Qiskit object, or anything else that would leak one source language into
//!   the optimizer.
//! - **No frontend defines IR semantics.** Frontends build through
//!   [`crate::ir::CircuitBuilder`] and let its validation speak. A source
//!   construct QC-IR cannot represent is reported
//!   ([`FrontendError::Unsupported`]), never accommodated by bending the IR —
//!   the rule set out in `docs/architecture_decision_no_frontend.md` and
//!   Stage A §3.2.
//!
//! Every frontend resolves gate names through one shared table
//! ([`map_gate`]), so they agree on what `rz` or `u2` means by construction.
//!
//! - [`openqasm`] — parses OpenQASM 3 source text.
//! - [`cudaq`] — parses the `@cudaq.kernel` function in CUDA-Q Python source.
//! - [`qiskit`] — translates a Qiskit `QuantumCircuit`, via the vendor-neutral
//!   [`qiskit::QiskitCircuitIr`] handoff struct.

pub mod cudaq;
pub mod error;
pub mod gate_map;
pub mod openqasm;
pub mod qiskit;

pub use cudaq::{parse_cudaq, parse_cudaq_named, parse_cudaq_timed};
pub use error::FrontendError;
pub use gate_map::map_gate;
pub use openqasm::{parse_openqasm3, parse_openqasm3_named, parse_openqasm3_timed};
pub use qiskit::{QiskitCircuitIr, QiskitInstruction, QiskitParam};

/// The deepest nesting a parameter expression may have — parentheses,
/// unary signs, indexing — before a frontend refuses it.
///
/// A structural safety limit (`final-deliverables-spec.md` §29), like
/// [`crate::ir::MAX_REGISTER_WIDTH`]: the expression parsers are recursive,
/// so unbounded nesting such as `rz((((…1…))))` overflowed the stack and
/// aborted the process. Found by the fuzz targets. Real angle expressions
/// (`2*pi/3 + theta`) are a few levels deep.
pub const MAX_EXPRESSION_DEPTH: usize = 32;

/// The most nodes one parameter expression may have.
///
/// Bounds the tree's depth whatever its shape: a long `1+1+…+1` chain parses
/// iteratively but builds a left-deep tree that evaluation and dropping then
/// walk recursively.
pub const MAX_EXPRESSION_NODES: usize = 256;

/// How long a frontend took, split where the frontend can split it.
///
/// Both built-in frontends run in two stages — source text to a syntax tree,
/// then the tree to validated QC-IR — and are timed per stage. A frontend
/// that cannot split reports its whole time as `parse` and no
/// `ir_construction`, rather than inventing a division.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrontendTiming {
    /// Reading the source into the frontend's own representation.
    pub parse: std::time::Duration,
    /// Building validated QC-IR from it, when measured separately.
    pub ir_construction: Option<std::time::Duration>,
}
