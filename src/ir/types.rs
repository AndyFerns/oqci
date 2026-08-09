//! Core value types shared across QC-IR and QCO-IR.
//!
//! These types are deliberately *newtype-wrapped* rather than bare primitives.
//! The wrapping is the cheap Rust equivalent of MLIR's typed SSA values and
//! typed attributes: it makes the eventual MLIR mapping explicit and prevents a
//! stringly-typed / integer-soup redesign in Phase 2. See
//! [`crate::ir::mlir_compat`] and `docs/mlir_dialect.md` for the exact
//! correspondence.

use std::fmt;

/// A reference to a qubit in a [`crate::ir::Circuit`].
///
/// Newtype-wrapped (rather than a bare `u32`) so that qubit references map
/// one-to-one onto typed MLIR SSA values of `!quantum.qubit` in Phase 2. Values
/// are dense indices into a circuit's qubit register: a circuit declaring `n`
/// qubits owns exactly `QubitId(0)..QubitId(n)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QubitId(pub u32);

impl QubitId {
    /// Returns the raw index this reference wraps.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

impl fmt::Display for QubitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%q{}", self.0)
    }
}

/// A reference to a classical bit in a [`crate::ir::Circuit`].
///
/// Classical bits are measurement targets. In Phase 0 they are write-only:
/// gates never read them (there is no classical feed-forward yet — see
/// `docs/architecture_decision_no_frontend.md` and the scope notes in
/// `docs/ir_spec.md`). Maps onto a typed MLIR SSA value of `!quantum.result`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClbitId(pub u32);

impl ClbitId {
    /// Returns the raw index this reference wraps.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

impl fmt::Display for ClbitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%c{}", self.0)
    }
}

/// A rotation angle / gate parameter, in **radians**.
///
/// A dedicated type (rather than a bare `f64`) so Phase 2 can swap the internal
/// representation for MLIR's `FloatAttr` without touching a single call site.
/// The value is stored as `f64`; angles are **not** normalised modulo `2π`, so
/// the IR preserves exactly what a frontend or builder supplied.
///
/// Construction is infallible; finiteness (no `NaN`/`inf`) is enforced by
/// circuit validation, which reports [`crate::ir::IrError::NonFiniteAngle`]
/// rather than panicking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Angle(f64);

impl Angle {
    /// Wraps a radian value as an [`Angle`]. Does not validate finiteness;
    /// that is checked during [`crate::ir::CircuitBuilder::build`].
    #[must_use]
    pub const fn new(radians: f64) -> Self {
        Angle(radians)
    }

    /// Returns the angle in radians.
    #[must_use]
    pub const fn radians(self) -> f64 {
        self.0
    }

    /// Returns `true` if the underlying value is finite (neither `NaN` nor
    /// infinite).
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.0.is_finite()
    }
}

impl From<f64> for Angle {
    fn from(radians: f64) -> Self {
        Angle(radians)
    }
}

impl fmt::Display for Angle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The identity of a quantum gate together with its parameters — but **not** its
/// qubit operands.
///
/// This is the closed "registered op" set plus an [`GateKind::Opaque`] escape
/// hatch, mirroring MLIR's registered-op + unknown-op model. Parameters
/// (angles) live here because they lower to MLIR **attributes**; qubit operands
/// live on the [`crate::ir::Instruction`] because they lower to MLIR **SSA
/// operands**. See `docs/mlir_dialect.md`.
///
/// Every variant has a fixed qubit [`arity`](GateKind::arity), enforced by
/// circuit validation.
#[derive(Debug, Clone, PartialEq)]
pub enum GateKind {
    /// Identity.
    I,
    /// Pauli-X (NOT).
    X,
    /// Pauli-Y.
    Y,
    /// Pauli-Z.
    Z,
    /// Hadamard.
    H,
    /// Phase gate `S = diag(1, i)`.
    S,
    /// Adjoint of `S`, `S† = diag(1, -i)`.
    Sdg,
    /// `T = diag(1, e^{iπ/4})`.
    T,
    /// Adjoint of `T`.
    Tdg,
    /// Rotation about X by the given angle.
    Rx(Angle),
    /// Rotation about Y by the given angle.
    Ry(Angle),
    /// Rotation about Z by the given angle.
    Rz(Angle),
    /// Phase / `R1` gate `P(λ) = diag(1, e^{iλ})`.
    P(Angle),
    /// General single-qubit unitary `U(θ, φ, λ)` (Euler / OpenQASM `U`).
    U {
        /// Polar angle θ.
        theta: Angle,
        /// First azimuthal angle φ.
        phi: Angle,
        /// Second azimuthal angle λ.
        lambda: Angle,
    },
    /// Controlled-X (CNOT): operands `[control, target]`.
    Cx,
    /// Controlled-Y: operands `[control, target]`.
    Cy,
    /// Controlled-Z: operands `[control, target]`.
    Cz,
    /// Swap: operands `[a, b]`.
    Swap,
    /// Toffoli (CCX): operands `[control0, control1, target]`.
    Ccx,
    /// Escape hatch for a gate not in the registered set.
    ///
    /// Mirrors MLIR's ability to carry an unregistered op. The `name` is the
    /// lowercase gate mnemonic; `params` are its angle parameters. Its qubit
    /// arity is not fixed by this enum and is therefore accepted as-is by
    /// validation (any non-zero number of distinct qubits).
    Opaque {
        /// Gate mnemonic (must be non-empty; validated).
        name: String,
        /// Angle parameters, lowered to MLIR attributes.
        params: Vec<Angle>,
    },
}

impl GateKind {
    /// The number of qubit operands this gate requires, or `None` for
    /// [`GateKind::Opaque`] (whose arity is not statically known).
    #[must_use]
    pub fn arity(&self) -> Option<usize> {
        Some(match self {
            GateKind::I
            | GateKind::X
            | GateKind::Y
            | GateKind::Z
            | GateKind::H
            | GateKind::S
            | GateKind::Sdg
            | GateKind::T
            | GateKind::Tdg
            | GateKind::Rx(_)
            | GateKind::Ry(_)
            | GateKind::Rz(_)
            | GateKind::P(_)
            | GateKind::U { .. } => 1,
            GateKind::Cx | GateKind::Cy | GateKind::Cz | GateKind::Swap => 2,
            GateKind::Ccx => 3,
            GateKind::Opaque { .. } => return None,
        })
    }

    /// Returns the lowercase mnemonic used in textual dumps and as the base of
    /// the QIR intrinsic name (e.g. `"h"`, `"cx"`, `"rz"`).
    #[must_use]
    pub fn mnemonic(&self) -> &str {
        match self {
            GateKind::I => "id",
            GateKind::X => "x",
            GateKind::Y => "y",
            GateKind::Z => "z",
            GateKind::H => "h",
            GateKind::S => "s",
            GateKind::Sdg => "sdg",
            GateKind::T => "t",
            GateKind::Tdg => "tdg",
            GateKind::Rx(_) => "rx",
            GateKind::Ry(_) => "ry",
            GateKind::Rz(_) => "rz",
            GateKind::P(_) => "p",
            GateKind::U { .. } => "u",
            GateKind::Cx => "cx",
            GateKind::Cy => "cy",
            GateKind::Cz => "cz",
            GateKind::Swap => "swap",
            GateKind::Ccx => "ccx",
            GateKind::Opaque { name, .. } => name,
        }
    }

    /// Returns this gate's angle parameters in canonical order.
    ///
    /// The order is significant and forms the lowering contract to MLIR
    /// attributes / QIR intrinsic arguments.
    #[must_use]
    pub fn params(&self) -> Vec<Angle> {
        match self {
            GateKind::Rx(a) | GateKind::Ry(a) | GateKind::Rz(a) | GateKind::P(a) => vec![*a],
            GateKind::U { theta, phi, lambda } => vec![*theta, *phi, *lambda],
            GateKind::Opaque { params, .. } => params.clone(),
            _ => Vec::new(),
        }
    }

    /// Whether this gate is a unitary (reversible) operation. Every
    /// [`GateKind`] is unitary; measurement and reset are modelled as separate
    /// [`crate::ir::Instruction`] variants, not gates. Provided so dependency
    /// classification reads declaratively.
    #[must_use]
    pub const fn is_unitary(&self) -> bool {
        true
    }
}

impl fmt::Display for GateKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let params = self.params();
        if params.is_empty() {
            write!(f, "{}", self.mnemonic())
        } else {
            let joined = params
                .iter()
                .map(|a| a.radians().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            write!(f, "{}({joined})", self.mnemonic())
        }
    }
}
