//! Error types for IR construction, validation, conversion, and lowering.
//!
//! All fallible IR operations return [`IrError`]. No IR routine panics or
//! `unwrap`s on malformed input; every reachable failure has a dedicated,
//! testable variant.

use crate::ir::types::{ClbitId, QubitId};

/// The single error type surfaced by the OQCI IR layer.
///
/// Variants are grouped by the phase that raises them: circuit **validation**
/// (raised by [`crate::ir::CircuitBuilder::build`]), **conversion**
/// (QC-IR → QCO-IR), and **lowering** (QCO-IR → QIR). Each validation variant
/// corresponds to exactly one documented invariant in `docs/ir_spec.md` and is
/// covered by a dedicated malformed-circuit test.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum IrError {
    /// A gate or instruction referenced a qubit index outside the circuit's
    /// declared qubit register.
    #[error("qubit {} is out of range: circuit declares {declared} qubit(s)", .qubit.index())]
    QubitOutOfRange {
        /// The offending reference.
        qubit: QubitId,
        /// Number of qubits the circuit declared.
        declared: u32,
    },

    /// A measurement referenced a classical-bit index outside the circuit's
    /// declared classical register.
    #[error("classical bit {} is out of range: circuit declares {declared} clbit(s)", .clbit.index())]
    ClbitOutOfRange {
        /// The offending reference.
        clbit: ClbitId,
        /// Number of classical bits the circuit declared.
        declared: u32,
    },

    /// A registered gate was applied to the wrong number of qubits.
    #[error("gate `{gate}` expects {expected} qubit operand(s), got {found}")]
    GateArityMismatch {
        /// Gate mnemonic.
        gate: String,
        /// Arity required by the [`crate::ir::GateKind`].
        expected: usize,
        /// Number of operands supplied.
        found: usize,
    },

    /// The same qubit appeared more than once in a single multi-qubit gate
    /// (e.g. `cx q0, q0`), which is physically meaningless.
    #[error("gate `{gate}` uses qubit {} more than once", .qubit.index())]
    DuplicateQubit {
        /// Gate mnemonic.
        gate: String,
        /// The qubit that was repeated.
        qubit: QubitId,
    },

    /// An [`crate::ir::GateKind::Opaque`] gate was given an empty name.
    #[error("opaque gate has an empty name")]
    EmptyOpaqueName,

    /// An [`crate::ir::GateKind::Opaque`] gate was applied to zero qubits.
    #[error("opaque gate `{gate}` must act on at least one qubit")]
    EmptyOpaqueOperands {
        /// Gate mnemonic.
        gate: String,
    },

    /// A gate parameter was `NaN` or infinite.
    #[error("gate `{gate}` has a non-finite angle parameter")]
    NonFiniteAngle {
        /// Gate mnemonic.
        gate: String,
    },

    /// The dependency graph produced during conversion contained a cycle.
    ///
    /// This indicates an internal invariant violation (QC-IR is a linear
    /// sequence and can only ever produce a DAG); it is surfaced rather than
    /// `unwrap`ped so that no code path can panic.
    #[error("internal error: dependency graph is cyclic")]
    CyclicGraph,
}
