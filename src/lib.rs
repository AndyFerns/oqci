//! # OQCI — Open Quantum Compiler Infrastructure
//!
//! A modular, vendor-neutral quantum compiler framework. This crate is the
//! **Phase 0** deliverable: the IR core, its conversions, and QIR lowering. It
//! deliberately contains **no frontend parsers** and **no backend execution**
//! (see `docs/architecture_decision_no_frontend.md`).
//!
//! ## Pipeline
//!
//! ```text
//! QC-IR  ──convert──▶  QCO-IR  ──lower──▶  QIR (LLVM-compatible text)
//! ```
//!
//! ```
//! use oqci::ir::{CircuitBuilder, qc_to_qco, emit_qir};
//!
//! // Build a Bell state in QC-IR.
//! let mut b = CircuitBuilder::new("bell");
//! let q0 = b.alloc_qubit();
//! let q1 = b.alloc_qubit();
//! let c0 = b.alloc_clbit();
//! let c1 = b.alloc_clbit();
//! b.h(q0).cx(q0, q1).measure(q0, c0).measure(q1, c1);
//! let circuit = b.build().unwrap();
//!
//! // Convert to the optimization IR and lower to QIR.
//! let dag = qc_to_qco(&circuit).unwrap();
//! let qir = emit_qir(&dag).unwrap();
//! assert!(qir.contains("__quantum__qis__h__body"));
//! ```
//!
//! ## Layout
//!
//! Everything lives under [`ir`]. The type design mirrors the `quantum` MLIR
//! dialect (`docs/mlir_dialect.md`) so the Phase 2 MLIR port is mechanical.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod ir;
