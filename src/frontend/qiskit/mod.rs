//! The Qiskit adapter's language-neutral core.
//!
//! `final-deliverables-spec.md` §5.3 requires translation from a Qiskit
//! `QuantumCircuit` into QC-IR, with the adapter living "at the integration
//! boundary rather than contaminating QC-IR with Qiskit types". This module is
//! the half of that adapter which contains all the decisions — gate mapping,
//! operand resolution, measurement destinations, parameter handling — and
//! **none** of the Python.
//!
//! The split is deliberate:
//!
//! - [`QiskitCircuitIr`] is a plain Rust struct describing what a
//!   `QuantumCircuit` *says*: register widths and an ordered instruction list
//!   with flat bit indices.
//! - [`translate`] turns that into a validated [`Circuit`]. It is exercised by
//!   ordinary `cargo test` with no Python interpreter present.
//! - The PyO3 boundary in the `oqci-python` crate does nothing but read a live
//!   `QuantumCircuit` into a [`QiskitCircuitIr`] and call [`translate`].
//!
//! So the logic that can be wrong is testable without Qiskit installed, and
//! the layer that needs Qiskit is thin enough to inspect by eye.
//!
//! # Known limitations
//!
//! - **Compound parameter expressions are refused.** A bare, unbound
//!   `Parameter` maps to [`Param::Symbol`]; an expression built from one
//!   (`2*theta`, `theta + phi`) has no QC-IR representation, so it is reported
//!   rather than approximated. See `docs/qiskit_adapter.md`.
//! - **`barrier` is dropped.** QC-IR has no barrier concept. Barriers carry no
//!   semantics for the operations OQCI currently models, so they are discarded
//!   deliberately — but note that a later scheduling pass must not treat a
//!   barrier-free circuit as evidence that none was written.

use crate::frontend::error::FrontendError;
use crate::frontend::gate_map::map_gate;
use crate::ir::{Circuit, CircuitBuilder, ClbitId, Param, QubitId};

/// A Qiskit `QuantumCircuit` reduced to the facts QC-IR needs.
///
/// Bit indices are flat and circuit-wide, exactly as `QuantumCircuit.find_bit`
/// reports them, so Qiskit's multiple named registers collapse to the same
/// dense space QC-IR uses.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QiskitCircuitIr {
    /// Circuit name.
    pub name: String,
    /// Total qubit count (`QuantumCircuit.num_qubits`).
    pub num_qubits: u32,
    /// Total classical bit count (`QuantumCircuit.num_clbits`).
    pub num_clbits: u32,
    /// Instructions in `QuantumCircuit.data` order.
    pub instructions: Vec<QiskitInstruction>,
}

/// One entry of `QuantumCircuit.data`.
#[derive(Debug, Clone, PartialEq)]
pub struct QiskitInstruction {
    /// `instruction.operation.name`.
    pub name: String,
    /// `instruction.operation.params`, in order.
    pub params: Vec<QiskitParam>,
    /// Flat indices of `instruction.qubits`, in operand order.
    pub qubits: Vec<u32>,
    /// Flat indices of `instruction.clbits`, in operand order.
    pub clbits: Vec<u32>,
}

/// A Qiskit instruction parameter.
#[derive(Debug, Clone, PartialEq)]
pub enum QiskitParam {
    /// A bound numeric value.
    Concrete(f64),
    /// An unbound `Parameter`, carried through as a symbol.
    Symbol(String),
    /// A `ParameterExpression` that is not a bare parameter. Represented so
    /// the PyO3 layer can hand it over verbatim and let [`translate`] produce
    /// one consistent diagnostic.
    Expression(String),
}

impl QiskitInstruction {
    /// Convenience constructor for a gate on the given qubits.
    #[must_use]
    pub fn gate(name: impl Into<String>, params: Vec<QiskitParam>, qubits: Vec<u32>) -> Self {
        QiskitInstruction {
            name: name.into(),
            params,
            qubits,
            clbits: Vec::new(),
        }
    }
}

/// Translates a [`QiskitCircuitIr`] into a validated [`Circuit`].
///
/// Instruction order is preserved exactly: `QuantumCircuit.data` is already in
/// program order, and QC-IR is likewise an ordered list, so no reordering is
/// needed or performed.
///
/// ```
/// use oqci::frontend::{QiskitCircuitIr, QiskitInstruction, qiskit::translate};
///
/// let bell = QiskitCircuitIr {
///     name: "bell".into(),
///     num_qubits: 2,
///     num_clbits: 0,
///     instructions: vec![
///         QiskitInstruction::gate("h", vec![], vec![0]),
///         QiskitInstruction::gate("cx", vec![], vec![0, 1]),
///     ],
/// };
/// let circuit = translate(&bell).unwrap();
/// assert_eq!(circuit.len(), 2);
/// ```
///
/// # Errors
///
/// - [`FrontendError::Unsupported`] for a compound `ParameterExpression`, or
///   an operation QC-IR cannot model (`if_else`, `while_loop`, and the other
///   dynamic-circuit operations Stage F defers).
/// - [`FrontendError::Semantic`] if a measurement's operand counts do not
///   pair up.
/// - [`FrontendError::ParamArity`] for a registered gate given the wrong
///   number of parameters.
/// - [`FrontendError::Ir`] for anything QC-IR's own validation rejects, such
///   as an out-of-range bit index.
pub fn translate(circuit: &QiskitCircuitIr) -> Result<Circuit, FrontendError> {
    let mut builder = CircuitBuilder::new(if circuit.name.is_empty() {
        "main"
    } else {
        &circuit.name
    });
    builder.alloc_qubits(circuit.num_qubits);
    builder.alloc_clbits(circuit.num_clbits);

    for inst in &circuit.instructions {
        let name = inst.name.to_ascii_lowercase();
        match name.as_str() {
            // Dynamic-circuit operations: explicitly out of scope (Stage F).
            "if_else" | "while_loop" | "for_loop" | "switch_case" | "break_loop"
            | "continue_loop" => {
                return Err(FrontendError::unsupported(format!(
                    "Qiskit control-flow operation `{}`; OQCI supports static circuits only",
                    inst.name
                )));
            }
            // Structural annotations with no QC-IR counterpart.
            "barrier" | "delay" => {}
            "measure" => {
                if inst.qubits.len() != inst.clbits.len() {
                    return Err(FrontendError::semantic(format!(
                        "measurement pairs {} qubit(s) with {} classical bit(s)",
                        inst.qubits.len(),
                        inst.clbits.len()
                    )));
                }
                for (q, c) in inst.qubits.iter().zip(&inst.clbits) {
                    builder.measure(QubitId(*q), ClbitId(*c));
                }
            }
            "reset" => {
                for q in &inst.qubits {
                    builder.reset(QubitId(*q));
                }
            }
            _ => {
                let params = inst
                    .params
                    .iter()
                    .map(|p| convert_param(p, &inst.name))
                    .collect::<Result<Vec<_>, _>>()?;
                let kind = map_gate(&name, params)?;
                let qubits: Vec<QubitId> = inst.qubits.iter().copied().map(QubitId).collect();
                builder.gate(kind, qubits);
            }
        }
    }

    Ok(builder.build()?)
}

fn convert_param(param: &QiskitParam, gate: &str) -> Result<Param, FrontendError> {
    match param {
        QiskitParam::Concrete(value) => Ok(Param::concrete(*value)),
        QiskitParam::Symbol(name) => Ok(Param::symbol(name)),
        QiskitParam::Expression(text) => Err(FrontendError::unsupported(format!(
            "compound parameter expression `{text}` on gate `{gate}`; \
             only a bare unbound Parameter is representable in QC-IR"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{GateKind, Instruction, IrError};

    fn circuit_ir(
        num_qubits: u32,
        num_clbits: u32,
        instructions: Vec<QiskitInstruction>,
    ) -> QiskitCircuitIr {
        QiskitCircuitIr {
            name: "test".into(),
            num_qubits,
            num_clbits,
            instructions,
        }
    }

    fn measure(qubits: Vec<u32>, clbits: Vec<u32>) -> QiskitInstruction {
        QiskitInstruction {
            name: "measure".into(),
            params: vec![],
            qubits,
            clbits,
        }
    }

    #[test]
    fn empty_circuit_translates() {
        let c = translate(&circuit_ir(0, 0, vec![])).unwrap();
        assert!(c.is_empty());
        assert_eq!(c.num_qubits(), 0);
    }

    #[test]
    fn bell_translates_in_order() {
        let c = translate(&circuit_ir(
            2,
            0,
            vec![
                QiskitInstruction::gate("h", vec![], vec![0]),
                QiskitInstruction::gate("cx", vec![], vec![0, 1]),
            ],
        ))
        .unwrap();
        assert_eq!(
            c.instructions(),
            [
                Instruction::Gate {
                    kind: GateKind::H,
                    qubits: vec![QubitId(0)]
                },
                Instruction::Gate {
                    kind: GateKind::Cx,
                    qubits: vec![QubitId(0), QubitId(1)]
                },
            ]
        );
    }

    #[test]
    fn ghz3_translates() {
        let c = translate(&circuit_ir(
            3,
            0,
            vec![
                QiskitInstruction::gate("h", vec![], vec![0]),
                QiskitInstruction::gate("cx", vec![], vec![0, 1]),
                QiskitInstruction::gate("cx", vec![], vec![1, 2]),
            ],
        ))
        .unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!(c.num_qubits(), 3);
    }

    #[test]
    fn mid_circuit_measurement_preserves_order() {
        let c = translate(&circuit_ir(
            2,
            1,
            vec![
                QiskitInstruction::gate("h", vec![], vec![0]),
                measure(vec![0], vec![0]),
                QiskitInstruction::gate("x", vec![], vec![1]),
            ],
        ))
        .unwrap();
        assert_eq!(c.len(), 3);
        assert!(matches!(c.instructions()[1], Instruction::Measure { .. }));
        assert!(matches!(c.instructions()[2], Instruction::Gate { .. }));
    }

    #[test]
    fn measurement_broadcasts_pairwise() {
        let c = translate(&circuit_ir(2, 2, vec![measure(vec![0, 1], vec![0, 1])])).unwrap();
        assert_eq!(
            c.instructions(),
            [
                Instruction::Measure {
                    qubit: QubitId(0),
                    target: ClbitId(0)
                },
                Instruction::Measure {
                    qubit: QubitId(1),
                    target: ClbitId(1)
                },
            ]
        );
    }

    #[test]
    fn reset_translates() {
        let c = translate(&circuit_ir(
            1,
            0,
            vec![QiskitInstruction {
                name: "reset".into(),
                params: vec![],
                qubits: vec![0],
                clbits: vec![],
            }],
        ))
        .unwrap();
        assert_eq!(c.instructions(), [Instruction::Reset { qubit: QubitId(0) }]);
    }

    #[test]
    fn barriers_are_dropped() {
        let c = translate(&circuit_ir(
            2,
            0,
            vec![
                QiskitInstruction::gate("h", vec![], vec![0]),
                QiskitInstruction::gate("barrier", vec![], vec![0, 1]),
            ],
        ))
        .unwrap();
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn unknown_gate_becomes_opaque() {
        let c = translate(&circuit_ir(
            2,
            0,
            vec![QiskitInstruction::gate("iswap", vec![], vec![0, 1])],
        ))
        .unwrap();
        assert!(matches!(
            &c.instructions()[0],
            Instruction::Gate { kind: GateKind::Opaque { name, .. }, .. } if name == "iswap"
        ));
    }

    #[test]
    fn bound_rotation_translates() {
        let c = translate(&circuit_ir(
            1,
            0,
            vec![QiskitInstruction::gate(
                "rz",
                vec![QiskitParam::Concrete(0.5)],
                vec![0],
            )],
        ))
        .unwrap();
        assert_eq!(
            c.instructions()[0],
            Instruction::Gate {
                kind: GateKind::Rz(Param::concrete(0.5)),
                qubits: vec![QubitId(0)]
            }
        );
    }

    #[test]
    fn unbound_parameter_becomes_a_symbol() {
        let c = translate(&circuit_ir(
            1,
            0,
            vec![QiskitInstruction::gate(
                "rz",
                vec![QiskitParam::Symbol("theta".into())],
                vec![0],
            )],
        ))
        .unwrap();
        assert_eq!(c.parameters(), vec!["theta".to_string()]);
    }

    #[test]
    fn compound_parameter_expression_is_rejected() {
        let error = translate(&circuit_ir(
            1,
            0,
            vec![QiskitInstruction::gate(
                "rz",
                vec![QiskitParam::Expression("2*theta".into())],
                vec![0],
            )],
        ))
        .unwrap_err();
        assert!(matches!(
            error,
            FrontendError::Unsupported(msg) if msg.contains("2*theta")
        ));
    }

    #[test]
    fn control_flow_operations_are_rejected() {
        for op in ["if_else", "while_loop", "for_loop", "switch_case"] {
            let error = translate(&circuit_ir(
                1,
                1,
                vec![QiskitInstruction::gate(op, vec![], vec![0])],
            ))
            .unwrap_err();
            assert!(
                matches!(&error, FrontendError::Unsupported(msg) if msg.contains("static circuits")),
                "{op} produced {error:?}"
            );
        }
    }

    #[test]
    fn out_of_range_qubit_propagates_from_the_ir() {
        let error = translate(&circuit_ir(
            1,
            0,
            vec![QiskitInstruction::gate("x", vec![], vec![7])],
        ))
        .unwrap_err();
        assert!(matches!(
            error,
            FrontendError::Ir(IrError::QubitOutOfRange { .. })
        ));
    }

    #[test]
    fn mismatched_measurement_operands_are_rejected() {
        let error = translate(&circuit_ir(2, 1, vec![measure(vec![0, 1], vec![0])])).unwrap_err();
        assert!(matches!(error, FrontendError::Semantic(_)));
    }

    #[test]
    fn wrong_parameter_count_is_rejected() {
        let error = translate(&circuit_ir(
            1,
            0,
            vec![QiskitInstruction::gate("rz", vec![], vec![0])],
        ))
        .unwrap_err();
        assert!(matches!(error, FrontendError::ParamArity { .. }));
    }
}
