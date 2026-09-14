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
//! Both current frontends resolve gate names through one shared table
//! ([`map_gate`]), so they agree on what `rz` or `u2` means by construction.
//!
//! - [`openqasm`] — parses OpenQASM 3 source text.
//! - [`qiskit`] — translates a Qiskit `QuantumCircuit`, via the vendor-neutral
//!   [`qiskit::QiskitCircuitIr`] handoff struct.

pub mod error;
pub mod gate_map;
pub mod openqasm;
pub mod qiskit;

pub use error::FrontendError;
pub use gate_map::map_gate;
pub use openqasm::{parse_openqasm3, parse_openqasm3_named};
pub use qiskit::{QiskitCircuitIr, QiskitInstruction, QiskitParam};
