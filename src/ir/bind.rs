//! Explicit parameter binding: symbolic QC-IR → concrete QC-IR.
//!
//! Stage F requires parameter binding to be its own compiler step rather than
//! an implicit side effect of lowering
//! (`docs/core_architecture/stage-f-static-parameterized-circuit-scope.md` §8:
//! "the IBM target-lowering layer must not receive ambiguous symbolic values
//! where the execution API requires concrete values"). [`bind_parameters`] is
//! that step.
//!
//! Binding preserves structure exactly: only [`crate::ir::Param::Symbol`]
//! values are replaced, in place, leaving the instruction sequence, qubit and
//! classical register sizes, and operand order untouched. The result is
//! re-validated through [`CircuitBuilder::build`], so a binding that supplies a
//! non-finite value is rejected the same way a literal `NaN` angle would be.

use std::collections::HashMap;

use crate::ir::error::IrError;
use crate::ir::qc::{Circuit, CircuitBuilder, Instruction};
use crate::ir::types::Angle;

/// Replaces every symbolic parameter in `circuit` with its bound value,
/// producing a new, re-validated circuit.
///
/// Bindings are keyed by symbol name. Extra entries that the circuit does not
/// use are ignored; a circuit that is already fully concrete is returned
/// unchanged (modulo re-validation).
///
/// ```
/// use std::collections::HashMap;
/// use oqci::ir::{bind_parameters, CircuitBuilder, Param};
///
/// let mut b = CircuitBuilder::new("ansatz");
/// let q0 = b.alloc_qubit();
/// b.rz(Param::symbol("theta"), q0);
/// let symbolic = b.build().unwrap();
/// assert_eq!(symbolic.parameters(), vec!["theta".to_string()]);
///
/// let bound = bind_parameters(&symbolic, &HashMap::from([("theta".into(), 1.5)])).unwrap();
/// assert!(bound.is_concrete());
/// ```
///
/// # Errors
///
/// - [`IrError::UnboundParameter`] if the circuit uses a symbol that
///   `bindings` does not supply.
/// - Any validation error from [`CircuitBuilder::build`] — notably
///   [`IrError::NonFiniteAngle`] if a supplied value is `NaN` or infinite.
pub fn bind_parameters(
    circuit: &Circuit,
    bindings: &HashMap<String, f64>,
) -> Result<Circuit, IrError> {
    let mut builder = CircuitBuilder::new(circuit.name());
    builder.alloc_qubits(circuit.num_qubits());
    builder.alloc_clbits(circuit.num_clbits());

    for inst in circuit.instructions() {
        match inst {
            Instruction::Gate { kind, qubits } => {
                let mnemonic = kind.mnemonic().to_string();
                let bound = kind.substitute(&mut |symbol: &str| {
                    bindings
                        .get(symbol)
                        .copied()
                        .map(Angle::new)
                        .ok_or_else(|| IrError::UnboundParameter {
                            gate: mnemonic.clone(),
                            symbol: symbol.to_string(),
                        })
                })?;
                builder.gate(bound, qubits.clone());
            }
            Instruction::Measure { qubit, target } => {
                builder.measure(*qubit, *target);
            }
            Instruction::Reset { qubit } => {
                builder.reset(*qubit);
            }
        }
    }

    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::param::Param;
    use crate::ir::types::GateKind;

    fn bindings(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    #[test]
    fn binding_matches_a_directly_built_concrete_circuit() {
        let mut symbolic = CircuitBuilder::new("ansatz");
        let q0 = symbolic.alloc_qubit();
        let q1 = symbolic.alloc_qubit();
        symbolic
            .h(q0)
            .rz(Param::symbol("theta"), q0)
            .cx(q0, q1)
            .ry(Param::symbol("phi"), q1);
        let symbolic = symbolic.build().unwrap();

        let bound =
            bind_parameters(&symbolic, &bindings(&[("theta", 0.25), ("phi", 0.5)])).unwrap();

        let mut expected = CircuitBuilder::new("ansatz");
        let e0 = expected.alloc_qubit();
        let e1 = expected.alloc_qubit();
        expected.h(e0).rz(0.25, e0).cx(e0, e1).ry(0.5, e1);
        assert_eq!(bound, expected.build().unwrap());
    }

    #[test]
    fn missing_binding_is_rejected() {
        let mut b = CircuitBuilder::new("ansatz");
        let q0 = b.alloc_qubit();
        b.rz(Param::symbol("theta"), q0);
        let circuit = b.build().unwrap();

        assert_eq!(
            bind_parameters(&circuit, &bindings(&[])),
            Err(IrError::UnboundParameter {
                gate: "rz".into(),
                symbol: "theta".into()
            })
        );
    }

    #[test]
    fn non_finite_binding_is_rejected() {
        let mut b = CircuitBuilder::new("ansatz");
        let q0 = b.alloc_qubit();
        b.rx(Param::symbol("theta"), q0);
        let circuit = b.build().unwrap();

        assert_eq!(
            bind_parameters(&circuit, &bindings(&[("theta", f64::INFINITY)])),
            Err(IrError::NonFiniteAngle { gate: "rx".into() })
        );
    }

    #[test]
    fn concrete_circuit_is_unchanged() {
        let mut b = CircuitBuilder::new("bell");
        let q0 = b.alloc_qubit();
        let q1 = b.alloc_qubit();
        let c0 = b.alloc_clbit();
        b.h(q0).cx(q0, q1).measure(q0, c0).reset(q1);
        let circuit = b.build().unwrap();

        assert_eq!(bind_parameters(&circuit, &bindings(&[])).unwrap(), circuit);
    }

    #[test]
    fn unused_bindings_are_ignored() {
        let mut b = CircuitBuilder::new("c");
        let q0 = b.alloc_qubit();
        b.rz(Param::symbol("theta"), q0);
        let circuit = b.build().unwrap();

        let bound =
            bind_parameters(&circuit, &bindings(&[("theta", 1.0), ("unused", 2.0)])).unwrap();
        assert!(bound.is_concrete());
    }

    #[test]
    fn binds_u_and_opaque_parameters() {
        let mut b = CircuitBuilder::new("c");
        let q0 = b.alloc_qubit();
        b.gate(
            GateKind::U {
                theta: Param::symbol("a"),
                phi: Param::concrete(0.5),
                lambda: Param::symbol("b"),
            },
            [q0],
        );
        b.gate(
            GateKind::Opaque {
                name: "custom".into(),
                params: vec![Param::symbol("a")],
            },
            [q0],
        );
        let circuit = b.build().unwrap();
        assert_eq!(circuit.parameters(), vec!["a".to_string(), "b".to_string()]);

        let bound = bind_parameters(&circuit, &bindings(&[("a", 1.0), ("b", 2.0)])).unwrap();
        assert!(bound.is_concrete());
    }
}
