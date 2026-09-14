//! The error type shared by every OQCI frontend.
//!
//! Per `docs/core_architecture/final-deliverables-spec.md` §5.6, a frontend
//! must *report* — never silently repair — invalid wire references,
//! unsupported gates, invalid arity, invalid parameters, unsupported dynamic
//! behaviour, malformed source, and semantics QC-IR cannot represent safely.
//! [`FrontendError`] is that report, and it is deliberately uniform across
//! frontends so downstream code never learns which language a circuit came
//! from.
//!
//! Note the [`FrontendError::Ir`] variant: frontends do **not** re-implement
//! QC-IR's validation invariants. They build through
//! [`crate::ir::CircuitBuilder`] and let [`crate::ir::IrError`] speak for the
//! IR, as required by Stage A §3.2 ("frontends consume the IR; they do not
//! define it").

use crate::ir::IrError;

/// Anything that can go wrong turning an external program into QC-IR.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum FrontendError {
    /// The source text is lexically or syntactically malformed.
    #[error("{message} (line {line}, column {column})")]
    Syntax {
        /// What went wrong.
        message: String,
        /// 1-based source line.
        line: u32,
        /// 1-based source column.
        column: u32,
    },

    /// The source parsed, but means something the frontend cannot honour —
    /// an undeclared register, an out-of-bounds register index, a broadcast
    /// over mismatched register widths.
    #[error("{0}")]
    Semantic(String),

    /// A construct outside the documented supported subset.
    ///
    /// This is the honest-refusal path required by Stage F: dynamic control
    /// flow, subroutines, and compound parameter expressions are rejected
    /// here rather than being approximated.
    #[error("unsupported construct: {0}")]
    Unsupported(String),

    /// A gate was given the wrong number of *classical* parameters (angles).
    ///
    /// Qubit-operand arity is QC-IR's business and surfaces as
    /// [`IrError::GateArityMismatch`] through [`FrontendError::Ir`].
    #[error("gate `{gate}` expects {expected} parameter(s), got {found}")]
    ParamArity {
        /// Gate mnemonic as written in the source.
        gate: String,
        /// Parameters the gate requires.
        expected: usize,
        /// Parameters supplied.
        found: usize,
    },

    /// QC-IR rejected the circuit the frontend built.
    #[error(transparent)]
    Ir(#[from] IrError),
}

impl FrontendError {
    /// Builds a [`FrontendError::Syntax`] at the given 1-based position.
    #[must_use]
    pub fn syntax(message: impl Into<String>, line: u32, column: u32) -> Self {
        FrontendError::Syntax {
            message: message.into(),
            line,
            column,
        }
    }

    /// Builds a [`FrontendError::Semantic`].
    #[must_use]
    pub fn semantic(message: impl Into<String>) -> Self {
        FrontendError::Semantic(message.into())
    }

    /// Builds a [`FrontendError::Unsupported`].
    #[must_use]
    pub fn unsupported(what: impl Into<String>) -> Self {
        FrontendError::Unsupported(what.into())
    }
}
