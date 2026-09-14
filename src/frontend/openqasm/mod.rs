//! The OpenQASM 3 frontend.
//!
//! Parses a documented subset of OpenQASM 3 into QC-IR. The subset — and,
//! just as importantly, everything deliberately left out of it — is specified
//! in `docs/openqasm_frontend.md`. Constructs outside it are refused with
//! [`FrontendError::Unsupported`] rather than approximated, because the
//! alternative (compiling `if` as though the branch were unconditional, say)
//! would silently change a program's meaning.
//!
//! The pipeline is the conventional three stages, each independently testable:
//! [`lexer`] → [`parser`] (producing [`ast`]) → [`translate`].
//!
//! ```
//! use oqci::frontend::parse_openqasm3;
//!
//! let circuit = parse_openqasm3(
//!     r#"
//!     OPENQASM 3.0;
//!     include "stdgates.inc";
//!     qubit[2] q;
//!     bit[2] c;
//!     h q[0];
//!     cx q[0], q[1];
//!     c = measure q;
//!     "#,
//! )
//! .unwrap();
//!
//! assert_eq!(circuit.num_qubits(), 2);
//! assert_eq!(circuit.len(), 4);
//! ```

pub mod ast;
pub mod lexer;
pub mod parser;
pub mod translate;

use crate::frontend::error::FrontendError;
use crate::ir::Circuit;

/// Parses OpenQASM 3 source into a validated [`Circuit`] named `main`.
///
/// # Errors
///
/// See [`parse_openqasm3_named`].
pub fn parse_openqasm3(source: &str) -> Result<Circuit, FrontendError> {
    parse_openqasm3_named(source, "main")
}

/// Parses OpenQASM 3 source into a validated [`Circuit`] with the given name.
///
/// # Errors
///
/// - [`FrontendError::Syntax`] — malformed source, with a line/column.
/// - [`FrontendError::Unsupported`] — a construct outside the documented
///   subset (control flow, subroutines, compound symbolic expressions, …).
/// - [`FrontendError::Semantic`] — undeclared or out-of-range registers,
///   mismatched broadcast widths.
/// - [`FrontendError::ParamArity`] — wrong number of gate parameters.
/// - [`FrontendError::Ir`] — the resulting circuit violates a QC-IR invariant.
pub fn parse_openqasm3_named(source: &str, name: &str) -> Result<Circuit, FrontendError> {
    let program = parser::parse(source)?;
    translate::translate(&program, name)
}
