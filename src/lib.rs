//! OQCI — the Open Quantum Compiler Infrastructure.
//!
//! A quantum compiler with a Rust-native IR at its core. A program comes in
//! through a frontend, becomes QC-IR, is optimized, is lowered onto a
//! specific device, and comes out as something that device could run.
//!
//! # Pipeline
//!
//! ```text
//! source ──frontend──▶ QC-IR ──convert──▶ QCO-IR ──lower──▶ QIR
//!                        │
//!                        ├──passes──▶ optimized QC-IR
//!                        │
//!                        └──lowering──▶ target-legal circuit ──prepare──▶ Executable
//! ```
//!
//! [`compile`] runs all of it. The CLI and the Python SDK both go through
//! that one entry point, so the numbers a user sees are the numbers the
//! compiler computed.
//!
//! ```
//! use oqci::ir::CircuitBuilder;
//!
//! let mut b = CircuitBuilder::new("bell");
//! let q0 = b.alloc_qubit();
//! let q1 = b.alloc_qubit();
//! b.h(q0).cx(q0, q1);
//!
//! let circuit = b.build().unwrap();
//! assert_eq!(circuit.num_qubits(), 2);
//! ```
//!
//! # Layout
//!
//! - [`ir`] — QC-IR, QCO-IR, validation, and QIR emission. Everything is
//!   built on this, and reaches it only through [`ir::CircuitBuilder`], so
//!   every `Circuit` in existence has already passed validation.
//! - [`frontend`] — OpenQASM 3 and Qiskit, into QC-IR.
//! - [`pass`] — the pass manager and the target-independent optimizations.
//! - [`analysis`] — the single place any metric is computed.
//! - [`target`] — what a device accepts, and what it finds expensive. It
//!   *describes* targets; it does not apply them.
//! - [`lowering`] — applying one: mapping, routing, basis decomposition.
//! - [`backend`] — the backend contract, the executable representation, and
//!   the provenance a result is cited by.
//! - [`compile`] — the orchestrator.
//! - [`cli`] — the `oqci` binary, which contains no compiler logic of its own.
//!
//! # What this crate does not do
//!
//! **It does not execute circuits.** Every shipped backend's `execute`
//! returns a typed "not available in this process" error naming where
//! execution actually happens: Qiskit Aer, through the Python adapter. The
//! project's non-goals rule out writing a simulator here, and live hardware
//! submission needs an SDK this crate does not depend on and credentials it
//! does not have. No claim of hardware executability is made.
//!
//! See `docs/` for the normative specifications, which are the source of
//! truth wherever they and a doc comment disagree.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod analysis;
pub mod backend;
pub mod cli;
pub mod compile;
pub mod frontend;
pub mod ir;
pub mod lowering;
pub mod pass;
pub mod target;
