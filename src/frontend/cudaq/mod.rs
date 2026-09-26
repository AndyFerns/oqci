//! The CUDA-Q frontend.
//!
//! Reads the source text of a Python file containing one `@cudaq.kernel`
//! function and translates a documented subset of CUDA-Q's kernel syntax into
//! QC-IR — the same [`Circuit`] every other frontend produces, built through
//! the same [`crate::ir::CircuitBuilder`] with gate names resolved through the
//! same shared table. `final-deliverables-spec.md` §5.5 asks exactly this of a
//! CUDA-Q adapter: that it "map into the same QC-IR contract".
//!
//! It reads source *text*, the way the OpenQASM frontend does, rather than
//! introspecting a live kernel object. That keeps it a pure-Rust, deterministic
//! compiler boundary: it needs neither Python nor CUDA-Q installed, is tested
//! by ordinary `cargo test`, and a kernel reaches the whole pipeline —
//! optimization, lowering, execution preparation, the CLI and the visualizer —
//! through [`crate::compile::Frontend::CudaQ`].
//!
//! The subset, what is refused, and the documentation it was verified against
//! are in `docs/cudaq_frontend.md`. The pipeline is [`parser`] (kernel
//! extraction, then [`lexer`] and a recursive-descent parse) → [`translate`].
//!
//! ```
//! use oqci::frontend::parse_cudaq;
//!
//! let circuit = parse_cudaq(
//!     r#"
//! import cudaq
//!
//! @cudaq.kernel
//! def bell():
//!     q = cudaq.qvector(2)
//!     h(q[0])
//!     x.ctrl(q[0], q[1])
//!     mz(q[0])
//!     mz(q[1])
//! "#,
//! )
//! .unwrap();
//!
//! assert_eq!(circuit.num_qubits(), 2);
//! assert_eq!(circuit.len(), 4);
//! ```

pub mod lexer;
pub mod parser;
pub mod translate;

use crate::frontend::error::FrontendError;
use crate::ir::Circuit;

/// Parses a CUDA-Q kernel into a validated [`Circuit`] named `main`.
///
/// # Errors
///
/// See [`parse_cudaq_named`].
pub fn parse_cudaq(source: &str) -> Result<Circuit, FrontendError> {
    parse_cudaq_named(source, "main")
}

/// Parses a CUDA-Q kernel into a validated [`Circuit`] with the given name.
///
/// # Errors
///
/// - [`FrontendError::Syntax`] — malformed kernel source, with a line/column.
/// - [`FrontendError::Unsupported`] — a construct outside the documented
///   subset (loops, conditionals, kernel calls, classical variables, …).
/// - [`FrontendError::Semantic`] — no kernel, an unallocated register, an
///   index out of range, a call with the wrong number of arguments.
/// - [`FrontendError::ParamArity`] — wrong number of gate parameters.
/// - [`FrontendError::Ir`] — the resulting circuit violates a QC-IR invariant.
pub fn parse_cudaq_named(source: &str, name: &str) -> Result<Circuit, FrontendError> {
    let kernel = parser::parse(source)?;
    translate::translate(&kernel, name)
}
