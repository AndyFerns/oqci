//! QC-IR — the imperative quantum circuit IR.
//!
//! QC-IR is the direct lowering target for future frontends (Phase 1). It is a
//! *sequential* list of [`Instruction`]s over a fixed qubit register and a
//! fixed classical register, mirroring how a source program is written: gates
//! in program order, measurements writing into classical bits, and resets.
//!
//! # Structure
//!
//! - A [`Circuit`] owns `num_qubits` qubits (`QubitId(0..num_qubits)`) and
//!   `num_clbits` classical bits, plus an ordered `Vec<Instruction>`.
//! - Circuits are built through [`CircuitBuilder`] (never by direct field
//!   mutation), mirroring MLIR's `OpBuilder`. The builder accumulates
//!   instructions with infallible, chainable methods and performs **all**
//!   validation in [`CircuitBuilder::build`].
//!
//! # Semantics
//!
//! Operationally, a QC-IR circuit denotes: prepare `num_qubits` qubits in
//! `|0…0⟩`, apply each instruction in list order, and record measurement
//! outcomes into the classical register. Gates are unitary; [`Instruction::Measure`]
//! and [`Instruction::Reset`] are non-unitary. See `docs/ir_spec.md` for the
//! full operational semantics.

use crate::ir::error::IrError;
use crate::ir::types::{Angle, ClbitId, GateKind, QubitId};

/// A single QC-IR instruction.
///
/// The three variants map one-to-one onto MLIR ops: `quantum.gate`,
/// `quantum.measure`, and `quantum.reset` (see `docs/mlir_dialect.md`). For
/// gates, the [`GateKind`] carries parameters (→ MLIR attributes) while the
/// `qubits` field carries operands (→ MLIR SSA operands).
#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    /// Apply a unitary gate to an ordered list of qubit operands.
    ///
    /// Operand order is significant and role-specific: for controlled gates the
    /// leading operands are controls and the last is the target (see
    /// [`Instruction::control`] / [`Instruction::target`]).
    Gate {
        /// Gate identity and parameters.
        kind: GateKind,
        /// Ordered qubit operands.
        qubits: Vec<QubitId>,
    },
    /// Measure a qubit in the computational (Z) basis, writing the outcome into
    /// a classical bit.
    Measure {
        /// Qubit being measured.
        qubit: QubitId,
        /// Destination classical bit.
        target: ClbitId,
    },
    /// Reset a qubit to `|0⟩` (non-unitary).
    Reset {
        /// Qubit being reset.
        qubit: QubitId,
    },
}

impl Instruction {
    /// The qubits this instruction touches, in operand order.
    #[must_use]
    pub fn qubits(&self) -> Vec<QubitId> {
        match self {
            Instruction::Gate { qubits, .. } => qubits.clone(),
            Instruction::Measure { qubit, .. } | Instruction::Reset { qubit } => vec![*qubit],
        }
    }

    /// The classical bit this instruction writes, if any.
    #[must_use]
    pub fn clbit(&self) -> Option<ClbitId> {
        match self {
            Instruction::Measure { target, .. } => Some(*target),
            _ => None,
        }
    }

    /// `true` if this instruction is a unitary gate; `false` for measurement
    /// and reset. Used by QCO-IR to classify data vs. control dependencies.
    #[must_use]
    pub fn is_unitary(&self) -> bool {
        matches!(self, Instruction::Gate { .. })
    }

    /// The control qubit(s) of a controlled gate, in order. Empty for
    /// non-controlled gates, measurement, and reset.
    #[must_use]
    pub fn control(&self) -> Vec<QubitId> {
        match self {
            Instruction::Gate { kind, qubits } => match kind {
                GateKind::Cx | GateKind::Cy | GateKind::Cz => {
                    qubits.first().copied().into_iter().collect()
                }
                GateKind::Ccx => qubits.iter().take(2).copied().collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// The target qubit of a controlled gate, if this is one.
    #[must_use]
    pub fn target(&self) -> Option<QubitId> {
        match self {
            Instruction::Gate {
                kind: GateKind::Cx | GateKind::Cy | GateKind::Cz | GateKind::Ccx,
                qubits,
            } => qubits.last().copied(),
            _ => None,
        }
    }
}

/// An immutable, validated QC-IR circuit.
///
/// Construct via [`CircuitBuilder`]; a `Circuit` returned from
/// [`CircuitBuilder::build`] is guaranteed to satisfy every invariant in
/// `docs/ir_spec.md` (all qubit/clbit references in range, correct gate arity,
/// no duplicate operands, finite angles).
#[derive(Debug, Clone, PartialEq)]
pub struct Circuit {
    name: String,
    num_qubits: u32,
    num_clbits: u32,
    instructions: Vec<Instruction>,
}

impl Circuit {
    /// The circuit's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Number of declared qubits. Valid qubit ids are `QubitId(0..num_qubits)`.
    #[must_use]
    pub fn num_qubits(&self) -> u32 {
        self.num_qubits
    }

    /// Number of declared classical bits.
    #[must_use]
    pub fn num_clbits(&self) -> u32 {
        self.num_clbits
    }

    /// The instruction sequence in program order.
    #[must_use]
    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    /// Number of instructions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.instructions.len()
    }

    /// `true` if the circuit has no instructions (the identity circuit).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }
}

/// Builder for [`Circuit`], mirroring MLIR's `OpBuilder`.
///
/// The builder is the *only* way to construct a [`Circuit`]. Register-allocation
/// methods ([`alloc_qubit`](CircuitBuilder::alloc_qubit),
/// [`alloc_clbit`](CircuitBuilder::alloc_clbit)) hand back typed ids; instruction
/// methods are infallible and chainable. Validation is deferred entirely to
/// [`build`](CircuitBuilder::build), keeping all invariant checks in one
/// auditable place.
///
/// ```
/// use oqci::ir::CircuitBuilder;
/// let mut b = CircuitBuilder::new("bell");
/// let q0 = b.alloc_qubit();
/// let q1 = b.alloc_qubit();
/// let c0 = b.alloc_clbit();
/// let c1 = b.alloc_clbit();
/// b.h(q0).cx(q0, q1).measure(q0, c0).measure(q1, c1);
/// let circuit = b.build().unwrap();
/// assert_eq!(circuit.num_qubits(), 2);
/// assert_eq!(circuit.len(), 4);
/// ```
#[derive(Debug, Clone)]
pub struct CircuitBuilder {
    name: String,
    num_qubits: u32,
    num_clbits: u32,
    instructions: Vec<Instruction>,
}

impl CircuitBuilder {
    /// Creates an empty builder for a circuit with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        CircuitBuilder {
            name: name.into(),
            num_qubits: 0,
            num_clbits: 0,
            instructions: Vec::new(),
        }
    }

    /// Allocates a fresh qubit and returns its id.
    pub fn alloc_qubit(&mut self) -> QubitId {
        let id = QubitId(self.num_qubits);
        self.num_qubits += 1;
        id
    }

    /// Allocates `n` fresh qubits, returning their ids in order.
    pub fn alloc_qubits(&mut self, n: u32) -> Vec<QubitId> {
        (0..n).map(|_| self.alloc_qubit()).collect()
    }

    /// Allocates a fresh classical bit and returns its id.
    pub fn alloc_clbit(&mut self) -> ClbitId {
        let id = ClbitId(self.num_clbits);
        self.num_clbits += 1;
        id
    }

    /// Allocates `n` fresh classical bits, returning their ids in order.
    pub fn alloc_clbits(&mut self, n: u32) -> Vec<ClbitId> {
        (0..n).map(|_| self.alloc_clbit()).collect()
    }

    /// Appends an arbitrary gate. Prefer the named helpers below where they
    /// exist; this is the general entry point (and the one frontends use).
    pub fn gate(&mut self, kind: GateKind, qubits: impl Into<Vec<QubitId>>) -> &mut Self {
        self.instructions.push(Instruction::Gate {
            kind,
            qubits: qubits.into(),
        });
        self
    }

    /// Appends a measurement of `qubit` into `target`.
    pub fn measure(&mut self, qubit: QubitId, target: ClbitId) -> &mut Self {
        self.instructions
            .push(Instruction::Measure { qubit, target });
        self
    }

    /// Appends a reset of `qubit` to `|0⟩`.
    pub fn reset(&mut self, qubit: QubitId) -> &mut Self {
        self.instructions.push(Instruction::Reset { qubit });
        self
    }

    // --- Named single-qubit gate helpers -------------------------------------

    /// Appends a Hadamard on `q`.
    pub fn h(&mut self, q: QubitId) -> &mut Self {
        self.gate(GateKind::H, [q])
    }
    /// Appends a Pauli-X on `q`.
    pub fn x(&mut self, q: QubitId) -> &mut Self {
        self.gate(GateKind::X, [q])
    }
    /// Appends a Pauli-Y on `q`.
    pub fn y(&mut self, q: QubitId) -> &mut Self {
        self.gate(GateKind::Y, [q])
    }
    /// Appends a Pauli-Z on `q`.
    pub fn z(&mut self, q: QubitId) -> &mut Self {
        self.gate(GateKind::Z, [q])
    }
    /// Appends an `Rx(theta)` on `q`.
    pub fn rx(&mut self, theta: Angle, q: QubitId) -> &mut Self {
        self.gate(GateKind::Rx(theta), [q])
    }
    /// Appends an `Ry(theta)` on `q`.
    pub fn ry(&mut self, theta: Angle, q: QubitId) -> &mut Self {
        self.gate(GateKind::Ry(theta), [q])
    }
    /// Appends an `Rz(theta)` on `q`.
    pub fn rz(&mut self, theta: Angle, q: QubitId) -> &mut Self {
        self.gate(GateKind::Rz(theta), [q])
    }

    // --- Named multi-qubit gate helpers --------------------------------------

    /// Appends a CNOT with the given control and target.
    pub fn cx(&mut self, control: QubitId, target: QubitId) -> &mut Self {
        self.gate(GateKind::Cx, [control, target])
    }
    /// Appends a controlled-Z with the given control and target.
    pub fn cz(&mut self, control: QubitId, target: QubitId) -> &mut Self {
        self.gate(GateKind::Cz, [control, target])
    }
    /// Appends a swap of `a` and `b`.
    pub fn swap(&mut self, a: QubitId, b: QubitId) -> &mut Self {
        self.gate(GateKind::Swap, [a, b])
    }
    /// Appends a Toffoli (CCX) with two controls and a target.
    pub fn ccx(&mut self, c0: QubitId, c1: QubitId, target: QubitId) -> &mut Self {
        self.gate(GateKind::Ccx, [c0, c1, target])
    }

    /// Validates every invariant and produces an immutable [`Circuit`].
    ///
    /// # Errors
    ///
    /// Returns the first [`IrError`] encountered while checking (in instruction
    /// order): qubit/clbit references in range, correct gate arity, no repeated
    /// operand within a gate, non-empty opaque names/operands, and finite
    /// angles. See `docs/ir_spec.md` for the invariant list.
    pub fn build(&self) -> Result<Circuit, IrError> {
        for inst in &self.instructions {
            self.validate_instruction(inst)?;
        }
        Ok(Circuit {
            name: self.name.clone(),
            num_qubits: self.num_qubits,
            num_clbits: self.num_clbits,
            instructions: self.instructions.clone(),
        })
    }

    fn validate_instruction(&self, inst: &Instruction) -> Result<(), IrError> {
        match inst {
            Instruction::Gate { kind, qubits } => self.validate_gate(kind, qubits),
            Instruction::Measure { qubit, target } => {
                self.check_qubit(*qubit)?;
                self.check_clbit(*target)
            }
            Instruction::Reset { qubit } => self.check_qubit(*qubit),
        }
    }

    fn validate_gate(&self, kind: &GateKind, qubits: &[QubitId]) -> Result<(), IrError> {
        let mnemonic = kind.mnemonic().to_string();

        // Angle finiteness.
        if kind.params().iter().any(|a| !a.is_finite()) {
            return Err(IrError::NonFiniteAngle { gate: mnemonic });
        }

        // Opaque-specific rules.
        if let GateKind::Opaque { name, .. } = kind {
            if name.is_empty() {
                return Err(IrError::EmptyOpaqueName);
            }
            if qubits.is_empty() {
                return Err(IrError::EmptyOpaqueOperands { gate: mnemonic });
            }
        }

        // Arity (registered gates only; Opaque has no fixed arity).
        if let Some(expected) = kind.arity()
            && qubits.len() != expected
        {
            return Err(IrError::GateArityMismatch {
                gate: mnemonic,
                expected,
                found: qubits.len(),
            });
        }

        // No repeated operand within a single gate.
        for (i, q) in qubits.iter().enumerate() {
            if qubits[..i].contains(q) {
                return Err(IrError::DuplicateQubit {
                    gate: mnemonic,
                    qubit: *q,
                });
            }
        }

        // All operands in range.
        for q in qubits {
            self.check_qubit(*q)?;
        }
        Ok(())
    }

    fn check_qubit(&self, q: QubitId) -> Result<(), IrError> {
        if q.index() >= self.num_qubits {
            Err(IrError::QubitOutOfRange {
                qubit: q,
                declared: self.num_qubits,
            })
        } else {
            Ok(())
        }
    }

    fn check_clbit(&self, c: ClbitId) -> Result<(), IrError> {
        if c.index() >= self.num_clbits {
            Err(IrError::ClbitOutOfRange {
                clbit: c,
                declared: self.num_clbits,
            })
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_circuit_is_identity() {
        let circuit = CircuitBuilder::new("empty").build().unwrap();
        assert!(circuit.is_empty());
        assert_eq!(circuit.num_qubits(), 0);
        assert_eq!(circuit.len(), 0);
    }

    #[test]
    fn bell_circuit_builds() {
        let mut b = CircuitBuilder::new("bell");
        let q0 = b.alloc_qubit();
        let q1 = b.alloc_qubit();
        b.h(q0).cx(q0, q1);
        let circuit = b.build().unwrap();
        assert_eq!(circuit.num_qubits(), 2);
        assert_eq!(circuit.len(), 2);
    }

    #[test]
    fn control_and_target_accessors() {
        let inst = Instruction::Gate {
            kind: GateKind::Cx,
            qubits: vec![QubitId(0), QubitId(1)],
        };
        assert_eq!(inst.control(), vec![QubitId(0)]);
        assert_eq!(inst.target(), Some(QubitId(1)));
    }

    #[test]
    fn ccx_control_and_target() {
        let inst = Instruction::Gate {
            kind: GateKind::Ccx,
            qubits: vec![QubitId(0), QubitId(1), QubitId(2)],
        };
        assert_eq!(inst.control(), vec![QubitId(0), QubitId(1)]);
        assert_eq!(inst.target(), Some(QubitId(2)));
    }

    #[test]
    fn qubit_out_of_range_is_rejected() {
        let mut b = CircuitBuilder::new("bad");
        let _q0 = b.alloc_qubit();
        b.gate(GateKind::X, [QubitId(5)]);
        assert_eq!(
            b.build(),
            Err(IrError::QubitOutOfRange {
                qubit: QubitId(5),
                declared: 1
            })
        );
    }

    #[test]
    fn clbit_out_of_range_is_rejected() {
        let mut b = CircuitBuilder::new("bad");
        let q0 = b.alloc_qubit();
        b.measure(q0, ClbitId(0));
        assert_eq!(
            b.build(),
            Err(IrError::ClbitOutOfRange {
                clbit: ClbitId(0),
                declared: 0
            })
        );
    }

    #[test]
    fn gate_arity_mismatch_is_rejected() {
        let mut b = CircuitBuilder::new("bad");
        let q0 = b.alloc_qubit();
        b.gate(GateKind::Cx, [q0]); // Cx needs 2
        assert_eq!(
            b.build(),
            Err(IrError::GateArityMismatch {
                gate: "cx".into(),
                expected: 2,
                found: 1
            })
        );
    }

    #[test]
    fn duplicate_qubit_is_rejected() {
        let mut b = CircuitBuilder::new("bad");
        let q0 = b.alloc_qubit();
        let _q1 = b.alloc_qubit();
        b.gate(GateKind::Cx, [q0, q0]);
        assert_eq!(
            b.build(),
            Err(IrError::DuplicateQubit {
                gate: "cx".into(),
                qubit: QubitId(0)
            })
        );
    }

    #[test]
    fn empty_opaque_name_is_rejected() {
        let mut b = CircuitBuilder::new("bad");
        let q0 = b.alloc_qubit();
        b.gate(
            GateKind::Opaque {
                name: String::new(),
                params: vec![],
            },
            [q0],
        );
        assert_eq!(b.build(), Err(IrError::EmptyOpaqueName));
    }

    #[test]
    fn empty_opaque_operands_is_rejected() {
        let mut b = CircuitBuilder::new("bad");
        b.gate(
            GateKind::Opaque {
                name: "custom".into(),
                params: vec![],
            },
            [],
        );
        assert_eq!(
            b.build(),
            Err(IrError::EmptyOpaqueOperands {
                gate: "custom".into()
            })
        );
    }

    #[test]
    fn non_finite_angle_is_rejected() {
        let mut b = CircuitBuilder::new("bad");
        let q0 = b.alloc_qubit();
        b.rx(Angle::new(f64::NAN), q0);
        assert_eq!(
            b.build(),
            Err(IrError::NonFiniteAngle { gate: "rx".into() })
        );
    }

    #[test]
    fn opaque_gate_with_operands_is_accepted() {
        let mut b = CircuitBuilder::new("ok");
        let q0 = b.alloc_qubit();
        let q1 = b.alloc_qubit();
        b.gate(
            GateKind::Opaque {
                name: "iswap".into(),
                params: vec![],
            },
            [q0, q1],
        );
        assert!(b.build().is_ok());
    }
}
