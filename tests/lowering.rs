//! Behavioural tests for target lowering.
//!
//! `tests/lowering_equivalence.rs` proves the *property* — that lowering
//! preserves semantics and produces legal circuits. This file covers what a
//! property test is bad at: that each specific refusal fires for its own
//! reason, that the reported metadata means what it says, and that the
//! orientation policies are the identities they claim to be.

mod support;

use oqci::ir::{Circuit, CircuitBuilder, GateKind, Instruction, Param, QubitId};
use oqci::lowering::routing::{OrientationPolicy, orientation_policy};
use oqci::lowering::{
    Layout, LayoutChoice, LoweringConfig, LoweringError, RuleSetError, UnroutableReason, lower,
};
use oqci::target::{
    BasisProfileBuilder, CouplingMode, MeasurementSupport, PhysicalQubit, Topology, builtin, check,
};
use support::statevector::{same_state_up_to_global_phase, simulate};

fn bell_on(a: usize, b: usize, n: u32) -> Circuit {
    let mut builder = CircuitBuilder::new("bell");
    let q = builder.alloc_qubits(n);
    builder.h(q[a]).cx(q[a], q[b]);
    builder.build().unwrap()
}

// --- The happy path ---------------------------------------------------------

#[test]
fn a_local_circuit_lowers_into_the_basis_without_routing() {
    let profile = builtin::linear_nisq(3);
    let lowered = lower(&bell_on(0, 1, 3), &profile, &LoweringConfig::default()).unwrap();

    assert_eq!(lowered.swaps_inserted, 0, "q0 and q1 are already adjacent");
    assert!(lowered.legality.is_legal());
    assert!(lowered.rules_applied.contains(&"h-to-rz-sx".to_string()));
    for instruction in lowered.circuit.instructions() {
        let Instruction::Gate { kind, .. } = instruction else {
            unreachable!()
        };
        assert!(
            profile.supports_operation(kind.mnemonic()),
            "`{}` is outside the basis",
            kind.mnemonic()
        );
    }
}

#[test]
fn a_non_local_circuit_gets_swaps_and_a_moved_layout() {
    // q0 and q2 are two hops apart on a line, so this is the circuit routing
    // exists for.
    let profile = builtin::linear_nisq(3);
    let lowered = lower(&bell_on(0, 2, 3), &profile, &LoweringConfig::default()).unwrap();

    assert!(lowered.swaps_inserted > 0);
    assert!(lowered.legality.is_legal());
    assert_ne!(
        lowered.initial_layout.permutation(),
        lowered.final_layout.permutation(),
        "routing moved qubits, so the layouts must differ"
    );
}

#[test]
fn an_unconstrained_target_needs_no_work_at_all() {
    let lowered = lower(
        &bell_on(0, 1, 2),
        &builtin::ideal_simulator(),
        &LoweringConfig::default(),
    )
    .unwrap();
    assert_eq!(lowered.swaps_inserted, 0);
    assert_eq!(lowered.orientations_repaired, 0);
    assert!(lowered.rules_applied.is_empty());
    assert_eq!(lowered.circuit, bell_on(0, 1, 2));
}

#[test]
fn the_output_register_is_sized_to_what_is_used_not_to_the_device() {
    // `ideal-simulator` declares 32 qubits. A Bell circuit that came back as
    // a 32-qubit circuit would need 2^32 amplitudes to simulate and would be
    // useless to every consumer downstream.
    let lowered = lower(
        &bell_on(0, 1, 2),
        &builtin::ideal_simulator(),
        &LoweringConfig::default(),
    )
    .unwrap();
    assert_eq!(lowered.circuit.num_qubits(), 2);
}

#[test]
fn the_reported_steps_describe_the_schedule_that_ran() {
    let lowered = lower(
        &bell_on(0, 2, 3),
        &builtin::linear_nisq(3),
        &LoweringConfig::default(),
    )
    .unwrap();
    let ids: Vec<&str> = lowered.steps.iter().map(|s| s.id).collect();
    assert_eq!(
        ids,
        vec![
            "arity-reduction",
            "layout",
            "routing",
            "basis-decomposition",
            "orientation-repair",
            "single-qubit-cleanup",
            "verify",
        ]
    );
    assert_eq!(lowered.profile_id, "linear-nisq@1");
}

#[test]
fn a_three_qubit_gate_is_reduced_before_anything_consults_the_coupling_map() {
    // The silent-wrongness path this ordering exists to close: `check` never
    // connectivity-checks a three-qubit gate, so a surviving `Ccx` would be
    // reported legal on a device that cannot run it.
    let mut b = CircuitBuilder::new("toffoli");
    let q = b.alloc_qubits(3);
    b.ccx(q[0], q[1], q[2]);
    let circuit = b.build().unwrap();

    // Demonstrate the hazard first: unlowered, `check` is content.
    let profile = builtin::linear_nisq(3);
    let unreduced = check(&circuit, &profile);
    assert!(
        unreduced
            .violations
            .iter()
            .all(|v| !matches!(v, oqci::target::Violation::ConnectivityViolation { .. })),
        "check does not connectivity-check a 3-qubit gate, which is the whole problem"
    );

    let lowered = lower(&circuit, &profile, &LoweringConfig::default()).unwrap();
    assert!(
        lowered
            .circuit
            .instructions()
            .iter()
            .all(|i| i.qubits().len() <= 2)
    );
    assert!(lowered.legality.is_legal());
}

// --- Orientation ------------------------------------------------------------

#[test]
fn the_declared_orientation_policies_are_the_identities_they_claim() {
    // `orientation_policy` is a table of claims about quantum mechanics.
    // Checked here rather than trusted, because getting one wrong is silent:
    // a swapped control and target agree on every computational basis state
    // and disagree only on superpositions.
    fn state(build: impl FnOnce(&mut CircuitBuilder)) -> Vec<num_complex::Complex64> {
        let mut b = CircuitBuilder::new("t");
        let q = b.alloc_qubits(2);
        // Full support on both wires, or a wrong orientation is invisible.
        b.h(q[0]).gate(GateKind::T, [q[0]]).h(q[1]);
        build(&mut b);
        simulate(&b.build().unwrap())
    }

    // Symmetric: exchanging operands gives the same operator back.
    for kind in [GateKind::Cz, GateKind::Swap] {
        assert_eq!(orientation_policy(&kind), OrientationPolicy::Symmetric);
        let forward = state(|b| {
            b.gate(kind.clone(), [QubitId(0), QubitId(1)]);
        });
        let reversed = state(|b| {
            b.gate(kind.clone(), [QubitId(1), QubitId(0)]);
        });
        assert!(
            same_state_up_to_global_phase(&forward, &reversed),
            "`{}` was declared symmetric but is not",
            kind.mnemonic()
        );
    }

    // Cx is NOT symmetric — the reason it needs a repair at all.
    let forward = state(|b| {
        b.cx(QubitId(0), QubitId(1));
    });
    let naively_reversed = state(|b| {
        b.cx(QubitId(1), QubitId(0));
    });
    assert!(
        !same_state_up_to_global_phase(&forward, &naively_reversed),
        "if this passed, operands could be swapped freely and no repair would be needed"
    );

    // ...and the declared repair really is CX(a,b) = (H x H) CX(b,a) (H x H).
    assert_eq!(
        orientation_policy(&GateKind::Cx),
        OrientationPolicy::ConjugateBoth("h")
    );
    let conjugated = state(|b| {
        b.h(QubitId(0))
            .h(QubitId(1))
            .cx(QubitId(1), QubitId(0))
            .h(QubitId(0))
            .h(QubitId(1));
    });
    assert!(same_state_up_to_global_phase(&forward, &conjugated));
}

#[test]
fn an_operation_facing_the_wrong_way_down_a_one_way_edge_is_repaired() {
    let mut topology = Topology::disconnected(2);
    topology.add_directed(PhysicalQubit(0), PhysicalQubit(1));
    let profile = BasisProfileBuilder::new("oneway", "1", "test", topology)
        .operations(["rz", "sx", "x", "cx"])
        .decomposition_rules(["h-to-rz-sx"])
        .cost_model("uniform")
        .build()
        .unwrap();

    // `cx q1, q0` runs against the declared direction.
    let mut b = CircuitBuilder::new("reverse");
    let q = b.alloc_qubits(2);
    b.cx(q[1], q[0]);

    let lowered = lower(&b.build().unwrap(), &profile, &LoweringConfig::default()).unwrap();
    assert_eq!(lowered.orientations_repaired, 1);
    assert!(lowered.legality.is_legal());
    // The conjugation was lowered too — no stray `h` survives.
    assert!(lowered.rules_applied.contains(&"h-to-rz-sx".to_string()));
}

#[test]
fn an_operation_with_no_known_reversal_is_refused_not_guessed_at() {
    // `cy` is not symmetric and has no verified conjugation, so a target that
    // makes it native on a one-way edge cannot run it backwards. Refusing is
    // the honest answer; silently exchanging the operands would be wrong.
    let mut topology = Topology::disconnected(2);
    topology.add_directed(PhysicalQubit(0), PhysicalQubit(1));
    let profile = BasisProfileBuilder::new("cy-oneway", "1", "test", topology)
        .operations(["cy", "h"])
        .cost_model("uniform")
        .build()
        .unwrap();

    assert_eq!(orientation_policy(&GateKind::Cy), OrientationPolicy::None);

    let mut b = CircuitBuilder::new("reverse-cy");
    let q = b.alloc_qubits(2);
    b.gate(GateKind::Cy, [q[1], q[0]]);

    let err = lower(&b.build().unwrap(), &profile, &LoweringConfig::default()).unwrap_err();
    assert!(
        matches!(&err, LoweringError::UnrepairableOrientation { mnemonic, .. } if mnemonic == "cy"),
        "got {err}"
    );
}

// --- Refusals ---------------------------------------------------------------

#[test]
fn a_disconnected_device_refuses_rather_than_emitting_something_illegal() {
    let profile = BasisProfileBuilder::new("split", "1", "test", Topology::disconnected(4))
        .operations(["cx", "h"])
        .cost_model("uniform")
        .build()
        .unwrap();

    let err = lower(&bell_on(0, 2, 4), &profile, &LoweringConfig::default()).unwrap_err();
    assert!(
        matches!(
            err,
            LoweringError::Unroutable {
                reason: UnroutableReason::NoPath,
                ..
            }
        ),
        "got {err}"
    );
}

#[test]
fn routing_refuses_to_cross_a_measured_wire_it_cannot_reuse() {
    // The frozen-wire rule. On a 3-line with no mid-circuit measurement,
    // measuring q1 and then interacting q0 with q2 would need to route
    // through the measured wire. Without the rule, routing would happily do
    // it and `check` would then blame the *measurement* for being
    // mid-circuit — an instruction that was fine when it was written.
    let profile = builtin::linear_nisq(3);
    assert!(!profile.measurement().mid_circuit_measurement);

    let mut b = CircuitBuilder::new("frozen");
    let q = b.alloc_qubits(3);
    let c = b.alloc_clbits(1);
    b.measure(q[1], c[0]).cx(q[0], q[2]);

    let err = lower(&b.build().unwrap(), &profile, &LoweringConfig::default()).unwrap_err();
    assert!(
        matches!(
            err,
            LoweringError::Unroutable {
                reason: UnroutableReason::MeasurementFrozenWire,
                ..
            }
        ),
        "got {err}"
    );
}

#[test]
fn routing_goes_around_a_measured_wire_when_a_detour_exists() {
    // The frozen-wire rule must not become a blanket refusal. On a ring,
    // measuring one qubit leaves the other way round intact, and routing is
    // expected to take it. An implementation that took the single best path
    // and rejected it for touching a measured wire would refuse this circuit
    // even though it compiles perfectly well.
    let mut ring = Topology::disconnected(4);
    ring.add_undirected(PhysicalQubit(0), PhysicalQubit(1));
    ring.add_undirected(PhysicalQubit(1), PhysicalQubit(2));
    ring.add_undirected(PhysicalQubit(2), PhysicalQubit(3));
    ring.add_undirected(PhysicalQubit(3), PhysicalQubit(0));

    let profile = BasisProfileBuilder::new("ring", "1", "test", ring)
        .operations(["rz", "sx", "x", "cx", "measure"])
        .measurement(MeasurementSupport {
            measurement: true,
            mid_circuit_measurement: false,
            reset: false,
        })
        .decomposition_rules(["h-to-rz-sx", "swap-to-cx"])
        .cost_model("uniform")
        .build()
        .unwrap();

    let mut b = CircuitBuilder::new("detour");
    let q = b.alloc_qubits(4);
    let c = b.alloc_clbits(1);
    // Measure the qubit sitting between q0 and q2 the short way round, then
    // ask q0 and q2 to interact. The long way round is still free.
    b.measure(q[1], c[0]).cx(q[0], q[2]);

    let lowered = lower(&b.build().unwrap(), &profile, &LoweringConfig::default()).unwrap();
    assert!(lowered.legality.is_legal());
    assert!(lowered.swaps_inserted > 0, "the operands were not adjacent");

    // Nothing may touch the measured wire after the measurement.
    let measured_at = lowered
        .circuit
        .instructions()
        .iter()
        .position(|i| matches!(i, Instruction::Measure { .. }))
        .expect("the measurement survives");
    let measured_qubit = lowered.circuit.instructions()[measured_at].qubits()[0];
    assert!(
        lowered.circuit.instructions()[measured_at + 1..]
            .iter()
            .all(|i| !i.qubits().contains(&measured_qubit)),
        "routing reused a measured wire on a device that cannot measure mid-circuit"
    );
}

#[test]
fn a_program_that_already_measures_mid_circuit_is_reported_against_the_program() {
    // Distinct from the frozen-wire refusal above, and deliberately so: no
    // layout or routing choice could have avoided this one, so the diagnostic
    // names the program rather than the router.
    let mut b = CircuitBuilder::new("mid");
    let q = b.alloc_qubits(2);
    let c = b.alloc_clbits(1);
    b.measure(q[0], c[0]).x(q[0]);

    let err = lower(
        &b.build().unwrap(),
        &builtin::linear_nisq(3),
        &LoweringConfig::default(),
    )
    .unwrap_err();
    assert!(
        matches!(&err, LoweringError::InputMeasuresMidCircuit { qubit, .. } if *qubit == QubitId(0)),
        "got {err}"
    );
}

#[test]
fn a_device_that_cannot_reset_refuses_a_circuit_that_does() {
    // Reset is non-unitary: no decomposition rule can ever produce one, so
    // this is refused up front rather than attempted.
    let mut b = CircuitBuilder::new("r");
    let q0 = b.alloc_qubit();
    b.reset(q0);

    let err = lower(
        &b.build().unwrap(),
        &builtin::linear_nisq(3),
        &LoweringConfig::default(),
    )
    .unwrap_err();
    assert!(
        matches!(&err, LoweringError::UnsupportedClassicalOperation { operation, .. } if *operation == "reset"),
        "got {err}"
    );
}

#[test]
fn a_device_that_cannot_measure_refuses_a_measurement() {
    let profile = BasisProfileBuilder::new("no-measure", "1", "test", Topology::linear(2))
        .operations(["cx", "h"])
        .measurement(MeasurementSupport {
            measurement: false,
            mid_circuit_measurement: false,
            reset: false,
        })
        .cost_model("uniform")
        .build()
        .unwrap();

    let mut b = CircuitBuilder::new("m");
    let q0 = b.alloc_qubit();
    let c0 = b.alloc_clbit();
    b.measure(q0, c0);

    let err = lower(&b.build().unwrap(), &profile, &LoweringConfig::default()).unwrap_err();
    assert!(
        matches!(&err, LoweringError::UnsupportedClassicalOperation { operation, .. } if *operation == "measure"),
        "got {err}"
    );
}

#[test]
fn a_circuit_wider_than_the_device_is_refused() {
    let err = lower(
        &bell_on(0, 1, 6),
        &builtin::linear_nisq(3),
        &LoweringConfig::default(),
    )
    .unwrap_err();
    assert!(matches!(err, LoweringError::Layout { .. }), "got {err}");
}

#[test]
fn a_target_whose_rules_cannot_reach_its_basis_is_refused_before_any_work() {
    let profile = BasisProfileBuilder::new("stuck", "1", "test", Topology::linear(3))
        .operations(["rz", "sx", "cx"])
        .decomposition_rules(["ccx-to-cx"])
        .cost_model("uniform")
        .build()
        .unwrap();

    let err = lower(&bell_on(0, 1, 3), &profile, &LoweringConfig::default()).unwrap_err();
    assert!(
        matches!(
            &err,
            LoweringError::RuleSet {
                source: RuleSetError::NotClosed { .. },
                ..
            }
        ),
        "got {err}"
    );
}

#[test]
fn an_explicit_layout_too_small_for_the_circuit_is_caught_at_the_entrance() {
    // The one path that bypasses a strategy's own fit check. Without the
    // guard this surfaces much later as `UnmappedQubit` against whichever
    // instruction happened to touch the missing qubit first.
    let layout = Layout::trivial(1, 3).unwrap();
    let config = LoweringConfig {
        layout: LayoutChoice::Explicit(layout),
        ..LoweringConfig::default()
    };
    let err = lower(&bell_on(0, 1, 3), &builtin::linear_nisq(3), &config).unwrap_err();
    assert!(matches!(err, LoweringError::Layout { .. }), "got {err}");
}

// --- Parameters -------------------------------------------------------------

#[test]
fn a_symbolic_parameter_survives_where_nothing_transforms_it() {
    // Stage F: a parameterized circuit compiles, keeps its symbol verbatim,
    // and is reported as awaiting binding rather than refused.
    let mut b = CircuitBuilder::new("ansatz");
    let q = b.alloc_qubits(2);
    b.rz(Param::symbol("theta"), q[0]).cx(q[0], q[1]);

    let lowered = lower(
        &b.build().unwrap(),
        &builtin::linear_nisq(3),
        &LoweringConfig::default(),
    )
    .unwrap();

    assert_eq!(lowered.circuit.parameters(), vec!["theta".to_string()]);
    assert!(
        !lowered.legality.is_legal(),
        "an unbound parameter is still a legality violation — it just is not a lowering failure"
    );
}

#[test]
fn a_symbolic_parameter_needing_arithmetic_is_refused_with_advice() {
    // `rx` reaches the basis only through a rule carrying a `+pi` offset, and
    // `Param` has no arithmetic. Emitting the angle unchanged would be
    // silently wrong with no oracle able to catch it.
    let mut b = CircuitBuilder::new("ansatz");
    let q0 = b.alloc_qubit();
    b.rx(Param::symbol("theta"), q0);

    let err = lower(
        &b.build().unwrap(),
        &builtin::linear_nisq(3),
        &LoweringConfig::default(),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("bind parameters"),
        "the error should say what to do: {err}"
    );
}

#[test]
fn binding_a_parameter_makes_the_same_circuit_lower_cleanly() {
    let mut b = CircuitBuilder::new("ansatz");
    let q0 = b.alloc_qubit();
    b.rx(Param::symbol("theta"), q0);
    let symbolic = b.build().unwrap();

    let bound = oqci::ir::bind_parameters(
        &symbolic,
        &std::collections::HashMap::from([("theta".to_string(), 0.5)]),
    )
    .unwrap();

    let lowered = lower(&bound, &builtin::linear_nisq(3), &LoweringConfig::default()).unwrap();
    assert!(lowered.legality.is_legal());
}

// --- Configuration ----------------------------------------------------------

#[test]
fn skipping_routing_reports_the_illegality_instead_of_hiding_it() {
    // `route: false` is an inspection aid, not a compilation mode. The result
    // is generally illegal, and lowering says so rather than pretending.
    let config = LoweringConfig {
        route: false,
        ..LoweringConfig::default()
    };
    let lowered = lower(&bell_on(0, 2, 3), &builtin::linear_nisq(3), &config).unwrap();
    assert_eq!(lowered.swaps_inserted, 0);
    assert!(
        !lowered.legality.is_legal(),
        "q0 and q2 are not adjacent, and skipping routing does not change that"
    );
}

#[test]
fn the_dense_layout_reduces_routing_on_a_line() {
    // Not a correctness claim — layout can only affect cost. Asserted so the
    // heuristic is known to do something, rather than silently degenerating
    // into the trivial layout.
    let mut b = CircuitBuilder::new("far");
    let q = b.alloc_qubits(4);
    b.cx(q[0], q[3]).cx(q[0], q[3]).cx(q[0], q[3]);
    let circuit = b.build().unwrap();
    let profile = builtin::linear_nisq(4);

    let trivial = lower(&circuit, &profile, &LoweringConfig::default()).unwrap();
    let dense = lower(
        &circuit,
        &profile,
        &LoweringConfig {
            layout: LayoutChoice::Dense,
            ..LoweringConfig::default()
        },
    )
    .unwrap();

    assert!(
        dense.swaps_inserted < trivial.swaps_inserted,
        "dense inserted {} swaps, trivial {}",
        dense.swaps_inserted,
        trivial.swaps_inserted
    );
    assert!(dense.legality.is_legal() && trivial.legality.is_legal());
}

#[test]
fn routing_never_moves_a_clbit() {
    // A classic bug: layout is a statement about qubits only.
    let profile = builtin::linear_nisq(3);
    let mut b = CircuitBuilder::new("measured");
    let q = b.alloc_qubits(3);
    let c = b.alloc_clbits(2);
    b.cx(q[0], q[2]).measure(q[0], c[1]).measure(q[2], c[0]);

    let lowered = lower(&b.build().unwrap(), &profile, &LoweringConfig::default()).unwrap();
    assert_eq!(lowered.circuit.num_clbits(), 2);

    let targets: Vec<u32> = lowered
        .circuit
        .instructions()
        .iter()
        .filter_map(|i| i.clbit().map(|c| c.index()))
        .collect();
    assert_eq!(
        targets,
        vec![1, 0],
        "measurement destinations must be untouched"
    );
}

#[test]
fn every_two_qubit_operation_ends_up_on_a_declared_coupling() {
    // Invariant I2/I4, checked directly against the topology rather than
    // through `check`.
    let profile = builtin::linear_nisq(4);
    let mut b = CircuitBuilder::new("mixed");
    let q = b.alloc_qubits(4);
    b.cx(q[0], q[3]).gate(GateKind::Cz, [q[1], q[3]]).h(q[2]);

    let lowered = lower(&b.build().unwrap(), &profile, &LoweringConfig::default()).unwrap();
    for instruction in lowered.circuit.instructions() {
        let qubits = instruction.qubits();
        if qubits.len() == 2 {
            assert!(
                profile.topology().couples(
                    PhysicalQubit(qubits[0].index()),
                    PhysicalQubit(qubits[1].index()),
                    CouplingMode::Directed
                ),
                "{instruction:?} sits on an undeclared coupling"
            );
        }
    }
}
