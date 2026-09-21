//! Checking a circuit against a target profile.
//!
//! Stage D exit criterion 4: "Unsupported operations are detected **before**
//! execution." This module is that check — the point at which a circuit stops
//! being target-independent and has to answer for itself against a specific
//! device.
//!
//! # It reports, it does not repair
//!
//! [`check`] returns every violation it finds rather than the first, and
//! rewrites nothing. Making a circuit legal is target *lowering* — mapping,
//! routing, decomposition — which is separate work by design (Stage D §4:
//! "these must not be conflated"). Conflating them here would mean a
//! validation function that silently changed the program it was asked to
//! inspect.
//!
//! Reporting all violations rather than the first is a deliberate ergonomic
//! choice: someone fixing a circuit by hand, or a future routing pass sizing
//! up how much work a target needs, wants the whole list.
//!
//! # What a clean report does and does not certify
//!
//! [`check`] is used as the postcondition oracle for target lowering, so what
//! it *misses* matters as much as what it catches. A clean report asserts
//! that every operation is in the basis, every declared parameter domain is
//! respected, no parameter is unbound, and measurement/reset constraints
//! hold. It does **not** assert:
//!
//! - **connectivity for operations of arity other than two.** A three-qubit
//!   `Ccx` is never checked against the coupling map, because a coupling map
//!   only describes pairs. Reducing arity is lowering's job, and lowering
//!   asserts it separately rather than relying on this function.
//! - **that operand order carries any particular meaning.** For every
//!   two-qubit operation, `qubits[0]` is read as the control and `qubits[1]`
//!   as the target, `Swap` included — so a symmetric operation emitted in a
//!   non-native order is reported as a connectivity violation. That is
//!   deliberate: [`crate::target::topology`] does not assume an undirected
//!   edge (Stage D §7), and this function will not either.
//!
//! # Logical qubits are interpreted as physical ones
//!
//! Until layout exists, a circuit's [`crate::ir::QubitId`] `n` is checked
//! against physical qubit `n` — the identity layout. That is the honest
//! reading of an unmapped circuit, and it is why a connectivity violation here
//! means "this circuit cannot run *as written*", not "this circuit can never
//! run on this device". Routing is what turns the latter into the former.

use serde::{Serialize, Serializer};

use crate::ir::{Circuit, GateKind, Instruction, Param, QubitId};
use crate::target::profile::BasisProfile;
use crate::target::topology::PhysicalQubit;

/// Serializes a logical qubit as its bare index.
///
/// The IR types are deliberately serde-free — their shape is a compiler
/// contract governed by `docs/ir_spec.md`, and it should not acquire a wire
/// format by accident (the same rule `src/cli/snapshot.rs` follows). So the
/// wire format is produced here, where it belongs, rather than by deriving
/// `Serialize` on [`QubitId`].
fn serialize_qubit<S: Serializer>(qubit: &QubitId, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_u32(qubit.index())
}

/// One reason a circuit cannot run on a target as written.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Violation {
    /// The profile's basis set does not contain this operation.
    UnsupportedOperation {
        /// Program index of the offending instruction.
        index: usize,
        /// The operation's mnemonic.
        mnemonic: String,
    },
    /// The circuit uses a qubit the device does not have.
    QubitOutOfRange {
        /// Program index of the offending instruction.
        index: usize,
        /// The offending qubit.
        #[serde(serialize_with = "serialize_qubit")]
        qubit: QubitId,
        /// How many physical qubits the device has.
        qubit_count: u32,
    },
    /// A two-qubit operation spans a coupling the device does not provide in
    /// that direction.
    ConnectivityViolation {
        /// Program index of the offending instruction.
        index: usize,
        /// Control operand.
        control: PhysicalQubit,
        /// Target operand.
        target: PhysicalQubit,
    },
    /// A parameter falls outside the operation's declared domain.
    ParameterOutOfRange {
        /// Program index of the offending instruction.
        index: usize,
        /// The operation's mnemonic.
        mnemonic: String,
        /// The offending value, in radians.
        value: f64,
        /// Declared lower bound.
        min: f64,
        /// Declared upper bound.
        max: f64,
    },
    /// A parameter is still symbolic.
    ///
    /// Reported whether or not the profile declares a domain for the
    /// operation: no execution API accepts a symbol, so an unbound parameter
    /// makes a circuit unexecutable on every target, not merely
    /// undomain-checkable on this one.
    UnboundParameter {
        /// Program index of the offending instruction.
        index: usize,
        /// The operation's mnemonic.
        mnemonic: String,
        /// The unbound symbol.
        symbol: String,
    },
    /// The device cannot measure.
    MeasurementUnsupported {
        /// Program index of the offending instruction.
        index: usize,
    },
    /// The device can only measure at the end of a circuit, and this
    /// measurement is followed by further work on the measured qubit.
    MidCircuitMeasurementUnsupported {
        /// Program index of the offending measurement.
        index: usize,
        /// The qubit that is operated on again afterwards.
        #[serde(serialize_with = "serialize_qubit")]
        qubit: QubitId,
    },
    /// The device cannot reset.
    ResetUnsupported {
        /// Program index of the offending instruction.
        index: usize,
    },
}

/// The result of checking a circuit against a profile.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct LegalityReport {
    /// Every violation found, in program order.
    pub violations: Vec<Violation>,
}

impl LegalityReport {
    /// `true` if the circuit can run on the target as written.
    #[must_use]
    pub fn is_legal(&self) -> bool {
        self.violations.is_empty()
    }

    /// How many violations were found.
    #[must_use]
    pub fn violation_count(&self) -> usize {
        self.violations.len()
    }
}

/// Checks `circuit` against `profile`, reporting every violation.
///
/// ```
/// use oqci::ir::CircuitBuilder;
/// use oqci::target::{builtin, check};
///
/// let mut b = CircuitBuilder::new("bell");
/// let q0 = b.alloc_qubit();
/// let q1 = b.alloc_qubit();
/// b.h(q0).cx(q0, q1);
///
/// let report = check(&b.build().unwrap(), &builtin::ideal_simulator());
/// assert!(report.is_legal());
/// ```
#[must_use]
pub fn check(circuit: &Circuit, profile: &BasisProfile) -> LegalityReport {
    let mut violations = Vec::new();

    for (index, instruction) in circuit.instructions().iter().enumerate() {
        for qubit in instruction.qubits() {
            if qubit.index() >= profile.qubit_count() {
                violations.push(Violation::QubitOutOfRange {
                    index,
                    qubit,
                    qubit_count: profile.qubit_count(),
                });
            }
        }

        match instruction {
            Instruction::Gate { kind, qubits } => {
                check_gate(index, kind, qubits, profile, &mut violations);
            }
            Instruction::Measure { qubit, .. } => {
                let support = profile.measurement();
                if !support.measurement {
                    violations.push(Violation::MeasurementUnsupported { index });
                } else if !support.mid_circuit_measurement && touched_after(circuit, index, *qubit)
                {
                    violations.push(Violation::MidCircuitMeasurementUnsupported {
                        index,
                        qubit: *qubit,
                    });
                }
            }
            Instruction::Reset { .. } => {
                if !profile.measurement().reset {
                    violations.push(Violation::ResetUnsupported { index });
                }
            }
        }
    }

    LegalityReport { violations }
}

fn check_gate(
    index: usize,
    kind: &GateKind,
    qubits: &[QubitId],
    profile: &BasisProfile,
    violations: &mut Vec<Violation>,
) {
    let mnemonic = kind.mnemonic();

    if !profile.supports_operation(mnemonic) {
        violations.push(Violation::UnsupportedOperation {
            index,
            mnemonic: mnemonic.to_string(),
        });
    }

    // Connectivity, for two-qubit operations. Operand order is the coupling
    // direction — see `topology`'s module docs on why that is not symmetric.
    if qubits.len() == 2 {
        let (control, target) = (
            PhysicalQubit(qubits[0].index()),
            PhysicalQubit(qubits[1].index()),
        );
        if profile.topology().contains(control)
            && profile.topology().contains(target)
            && !profile.topology().supports(control, target)
        {
            violations.push(Violation::ConnectivityViolation {
                index,
                control,
                target,
            });
        }
    }

    // An unbound parameter is a violation on its own, independently of any
    // declared domain: no execution API accepts a symbol, so a circuit
    // carrying one cannot run on *any* target. Checking this only for
    // operations that happen to declare a `ParameterConstraint` would leave
    // `rz(theta)` reported legal against a profile with no `rz` constraint —
    // which is exactly the kind of "passes validation, cannot execute"
    // certificate this layer exists to prevent. Stage F §8 makes parameter
    // binding an explicit compiler/backend step for this reason.
    let constraint = profile.parameter_constraint(mnemonic);
    for param in kind.params() {
        match param {
            Param::Symbol(symbol) => {
                violations.push(Violation::UnboundParameter {
                    index,
                    mnemonic: mnemonic.to_string(),
                    symbol,
                });
            }
            Param::Concrete(angle) => {
                // A domain, when the profile declares one, is checked against
                // the concrete value.
                if let Some(constraint) = constraint
                    && !constraint.admits(angle.radians())
                {
                    violations.push(Violation::ParameterOutOfRange {
                        index,
                        mnemonic: mnemonic.to_string(),
                        value: angle.radians(),
                        min: constraint.min,
                        max: constraint.max,
                    });
                }
            }
        }
    }
}

/// Whether `qubit` is operated on after instruction `index`.
fn touched_after(circuit: &Circuit, index: usize, qubit: QubitId) -> bool {
    circuit
        .instructions()
        .iter()
        .skip(index + 1)
        .any(|later| later.qubits().contains(&qubit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CircuitBuilder, ClbitId};
    use crate::target::profile::{BasisProfileBuilder, MeasurementSupport, ParameterConstraint};
    use crate::target::topology::Topology;

    fn profile() -> BasisProfile {
        // A directed line: 0 -> 1 -> 2 only, so reverse couplings are illegal.
        let mut topology = Topology::disconnected(3);
        topology
            .add_directed(PhysicalQubit(0), PhysicalQubit(1))
            .add_directed(PhysicalQubit(1), PhysicalQubit(2));
        BasisProfileBuilder::new("test", "1", "test-backend", topology)
            .operations(["h", "cx", "rz", "measure", "reset"])
            .cost_model("test-cost")
            .build()
            .unwrap()
    }

    fn circuit(build: impl FnOnce(&mut CircuitBuilder)) -> Circuit {
        let mut b = CircuitBuilder::new("c");
        b.alloc_qubits(4);
        b.alloc_clbits(2);
        build(&mut b);
        b.build().unwrap()
    }

    #[test]
    fn a_conforming_circuit_is_legal() {
        let c = circuit(|b| {
            b.h(QubitId(0)).cx(QubitId(0), QubitId(1));
        });
        let report = check(&c, &profile());
        assert!(report.is_legal(), "got {:?}", report.violations);
    }

    #[test]
    fn an_unsupported_operation_is_reported() {
        let c = circuit(|b| {
            b.gate(GateKind::T, [QubitId(0)]);
        });
        assert_eq!(
            check(&c, &profile()).violations,
            vec![Violation::UnsupportedOperation {
                index: 0,
                mnemonic: "t".into()
            }]
        );
    }

    #[test]
    fn a_qubit_beyond_the_device_is_reported() {
        let c = circuit(|b| {
            b.h(QubitId(3));
        });
        let violations = check(&c, &profile()).violations;
        assert!(matches!(
            violations.first(),
            Some(Violation::QubitOutOfRange { qubit_count: 3, .. })
        ));
    }

    #[test]
    fn coupling_direction_is_enforced() {
        // The Stage D §7 rule, end to end: 0 -> 1 is native, 1 -> 0 is not.
        let forward = circuit(|b| {
            b.cx(QubitId(0), QubitId(1));
        });
        assert!(check(&forward, &profile()).is_legal());

        let reverse = circuit(|b| {
            b.cx(QubitId(1), QubitId(0));
        });
        assert_eq!(
            check(&reverse, &profile()).violations,
            vec![Violation::ConnectivityViolation {
                index: 0,
                control: PhysicalQubit(1),
                target: PhysicalQubit(0),
            }]
        );
    }

    #[test]
    fn a_non_adjacent_two_qubit_gate_is_reported() {
        let c = circuit(|b| {
            b.cx(QubitId(0), QubitId(2));
        });
        assert!(
            check(&c, &profile())
                .violations
                .iter()
                .any(|v| matches!(v, Violation::ConnectivityViolation { .. }))
        );
    }

    #[test]
    fn connectivity_is_not_reported_for_an_out_of_range_qubit() {
        // One problem, one message: an out-of-range qubit is already reported,
        // and adding "…and they aren't coupled" would be noise.
        let c = circuit(|b| {
            b.cx(QubitId(0), QubitId(3));
        });
        let violations = check(&c, &profile()).violations;
        assert_eq!(violations.len(), 1);
        assert!(matches!(violations[0], Violation::QubitOutOfRange { .. }));
    }

    #[test]
    fn a_parameter_outside_its_domain_is_reported() {
        let constrained = BasisProfileBuilder::new("t", "1", "b", Topology::linear(2))
            .operations(["rz"])
            .parameter_constraint("rz", ParameterConstraint::new(-1.0, 1.0))
            .cost_model("c")
            .build()
            .unwrap();

        let c = circuit(|b| {
            b.rz(3.0, QubitId(0));
        });
        assert!(matches!(
            check(&c, &constrained).violations.first(),
            Some(Violation::ParameterOutOfRange { value, .. }) if *value == 3.0
        ));
    }

    #[test]
    fn a_parameter_inside_its_domain_is_fine() {
        let constrained = BasisProfileBuilder::new("t", "1", "b", Topology::linear(2))
            .operations(["rz"])
            .parameter_constraint("rz", ParameterConstraint::new(-1.0, 1.0))
            .cost_model("c")
            .build()
            .unwrap();
        let c = circuit(|b| {
            b.rz(0.5, QubitId(0));
        });
        assert!(check(&c, &constrained).is_legal());
    }

    #[test]
    fn an_unbound_parameter_cannot_be_checked_and_is_reported() {
        let constrained = BasisProfileBuilder::new("t", "1", "b", Topology::linear(2))
            .operations(["rz"])
            .parameter_constraint("rz", ParameterConstraint::new(-1.0, 1.0))
            .cost_model("c")
            .build()
            .unwrap();
        let c = circuit(|b| {
            b.rz(Param::symbol("theta"), QubitId(0));
        });
        assert!(matches!(
            check(&c, &constrained).violations.first(),
            Some(Violation::UnboundParameter { symbol, .. }) if symbol == "theta"
        ));
    }

    #[test]
    fn an_unbound_parameter_is_reported_without_a_declared_domain() {
        // Regression: the symbolic check used to sit behind the
        // `parameter_constraint` lookup, so a profile that declared no domain
        // for `rz` — which `builtin::linear_nisq` does not — reported a
        // symbolic `rz` as fully legal. Nothing downstream could catch it:
        // the state-vector harness panics on symbolic parameters, so a
        // wrongly-transformed symbol had no oracle at all.
        let unconstrained = BasisProfileBuilder::new("t", "1", "b", Topology::linear(2))
            .operations(["rz"])
            .cost_model("c")
            .build()
            .unwrap();
        assert!(unconstrained.parameter_constraint("rz").is_none());

        let c = circuit(|b| {
            b.rz(Param::symbol("theta"), QubitId(0));
        });
        assert_eq!(
            check(&c, &unconstrained).violations,
            vec![Violation::UnboundParameter {
                index: 0,
                mnemonic: "rz".into(),
                symbol: "theta".into(),
            }]
        );
    }

    #[test]
    fn every_unbound_parameter_of_a_multi_parameter_gate_is_reported() {
        let unconstrained = BasisProfileBuilder::new("t", "1", "b", Topology::linear(2))
            .operations(["u"])
            .cost_model("c")
            .build()
            .unwrap();
        let c = circuit(|b| {
            b.gate(
                GateKind::U {
                    theta: Param::symbol("a"),
                    phi: Param::concrete(0.5),
                    lambda: Param::symbol("b"),
                },
                [QubitId(0)],
            );
        });
        let symbols: Vec<String> = check(&c, &unconstrained)
            .violations
            .into_iter()
            .filter_map(|v| match v {
                Violation::UnboundParameter { symbol, .. } => Some(symbol),
                _ => None,
            })
            .collect();
        assert_eq!(symbols, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn measurement_support_is_enforced() {
        let no_measure = BasisProfileBuilder::new("t", "1", "b", Topology::linear(2))
            .operations(["h"])
            .measurement(MeasurementSupport {
                measurement: false,
                mid_circuit_measurement: false,
                reset: false,
            })
            .cost_model("c")
            .build()
            .unwrap();

        let c = circuit(|b| {
            b.measure(QubitId(0), ClbitId(0));
        });
        assert_eq!(
            check(&c, &no_measure).violations,
            vec![Violation::MeasurementUnsupported { index: 0 }]
        );
    }

    #[test]
    fn a_terminal_measurement_is_legal_without_mid_circuit_support() {
        let terminal_only = BasisProfileBuilder::new("t", "1", "b", Topology::linear(2))
            .operations(["h"])
            .measurement(MeasurementSupport {
                measurement: true,
                mid_circuit_measurement: false,
                reset: false,
            })
            .cost_model("c")
            .build()
            .unwrap();

        let c = circuit(|b| {
            b.h(QubitId(0)).measure(QubitId(0), ClbitId(0));
        });
        assert!(check(&c, &terminal_only).is_legal());
    }

    #[test]
    fn a_mid_circuit_measurement_is_reported_when_unsupported() {
        let terminal_only = BasisProfileBuilder::new("t", "1", "b", Topology::linear(2))
            .operations(["h"])
            .measurement(MeasurementSupport {
                measurement: true,
                mid_circuit_measurement: false,
                reset: false,
            })
            .cost_model("c")
            .build()
            .unwrap();

        let c = circuit(|b| {
            b.measure(QubitId(0), ClbitId(0)).h(QubitId(0));
        });
        assert!(matches!(
            check(&c, &terminal_only).violations.first(),
            Some(Violation::MidCircuitMeasurementUnsupported { .. })
        ));
    }

    #[test]
    fn reset_support_is_enforced() {
        let no_reset = BasisProfileBuilder::new("t", "1", "b", Topology::linear(2))
            .operations(["h"])
            .measurement(MeasurementSupport {
                measurement: true,
                mid_circuit_measurement: true,
                reset: false,
            })
            .cost_model("c")
            .build()
            .unwrap();

        let c = circuit(|b| {
            b.reset(QubitId(0));
        });
        assert_eq!(
            check(&c, &no_reset).violations,
            vec![Violation::ResetUnsupported { index: 0 }]
        );
    }

    #[test]
    fn every_violation_is_reported_not_just_the_first() {
        let c = circuit(|b| {
            b.gate(GateKind::T, [QubitId(0)])
                .gate(GateKind::T, [QubitId(1)])
                .cx(QubitId(1), QubitId(0));
        });
        let report = check(&c, &profile());
        assert_eq!(report.violation_count(), 3);
        assert!(!report.is_legal());
    }

    #[test]
    fn violations_carry_the_instruction_index() {
        let c = circuit(|b| {
            b.h(QubitId(0)).gate(GateKind::T, [QubitId(0)]);
        });
        assert!(matches!(
            check(&c, &profile()).violations.first(),
            Some(Violation::UnsupportedOperation { index: 1, .. })
        ));
    }

    #[test]
    fn an_empty_circuit_is_legal_anywhere() {
        assert!(check(&circuit(|_| {}), &profile()).is_legal());
    }
}
