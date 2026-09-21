//! Integration tests for the target model.
//!
//! Exercises `oqci::target` the way a caller would: build or parse a circuit,
//! check it against a profile, cost it. The per-type unit tests live next to
//! the code; what these cover is the seams between profile, legality, cost and
//! the rest of the compiler.

use oqci::frontend::parse_openqasm3_named;
use oqci::ir::{CircuitBuilder, GateKind, Param};
use oqci::pass::{PassContext, PassManager, PassSelection};
use oqci::target::{
    BasisProfileBuilder, MeasurementSupport, PhysicalQubit, Topology, Violation, builtin, check,
    cost::{CostModel, resolve},
};

fn bell() -> oqci::ir::Circuit {
    let mut b = CircuitBuilder::new("bell");
    let q0 = b.alloc_qubit();
    let q1 = b.alloc_qubit();
    let c0 = b.alloc_clbit();
    let c1 = b.alloc_clbit();
    b.h(q0).cx(q0, q1).measure(q0, c0).measure(q1, c1);
    b.build().unwrap()
}

// --- Legality ---------------------------------------------------------------

#[test]
fn an_unconstrained_target_accepts_an_ordinary_circuit() {
    let report = check(&bell(), &builtin::ideal_simulator());
    assert!(report.is_legal(), "got {:?}", report.violations);
}

#[test]
fn a_restricted_basis_rejects_the_gates_it_lacks() {
    // `h` is outside a {rz, sx, x, cx} basis — this is precisely the circuit
    // that basis decomposition will later exist to fix.
    let report = check(&bell(), &builtin::linear_nisq(5));
    assert_eq!(
        report.violations,
        vec![Violation::UnsupportedOperation {
            index: 0,
            mnemonic: "h".into()
        }]
    );
}

#[test]
fn connectivity_is_checked_against_the_identity_layout() {
    // With no layout step yet, logical qubit n is physical qubit n. On a line,
    // q0-q2 are not adjacent, so this is illegal *as written* — which routing,
    // when it exists, is what will fix.
    let mut b = CircuitBuilder::new("far");
    let q = b.alloc_qubits(5);
    b.cx(q[0], q[2]);

    let report = check(&b.build().unwrap(), &builtin::linear_nisq(5));
    assert!(
        report
            .violations
            .iter()
            .any(|v| matches!(v, Violation::ConnectivityViolation { .. }))
    );
}

#[test]
fn a_directed_coupling_rejects_the_reverse_orientation() {
    // Stage D §7 end to end: an undirected edge must not be assumed.
    let mut topology = Topology::disconnected(2);
    topology.add_directed(PhysicalQubit(0), PhysicalQubit(1));
    let profile = BasisProfileBuilder::new("directed", "1", "test", topology)
        .operations(["cx"])
        .cost_model("uniform")
        .build()
        .unwrap();

    let mut forward = CircuitBuilder::new("f");
    let f = forward.alloc_qubits(2);
    forward.cx(f[0], f[1]);
    assert!(check(&forward.build().unwrap(), &profile).is_legal());

    let mut reverse = CircuitBuilder::new("r");
    let r = reverse.alloc_qubits(2);
    reverse.cx(r[1], r[0]);
    assert!(!check(&reverse.build().unwrap(), &profile).is_legal());
}

#[test]
fn a_device_without_reset_rejects_one() {
    let mut b = CircuitBuilder::new("r");
    let q0 = b.alloc_qubit();
    b.reset(q0);

    let report = check(&b.build().unwrap(), &builtin::linear_nisq(5));
    assert_eq!(
        report.violations,
        vec![Violation::ResetUnsupported { index: 0 }]
    );
}

#[test]
fn a_terminal_measurement_is_accepted_where_a_mid_circuit_one_is_not() {
    let profile = builtin::linear_nisq(5);

    // Terminal: fine.
    let mut terminal = CircuitBuilder::new("t");
    let q = terminal.alloc_qubits(2);
    let c = terminal.alloc_clbits(1);
    terminal.x(q[0]).measure(q[0], c[0]);
    assert!(check(&terminal.build().unwrap(), &profile).is_legal());

    // Followed by more work on the same qubit: not.
    let mut mid = CircuitBuilder::new("m");
    let q = mid.alloc_qubits(2);
    let c = mid.alloc_clbits(1);
    mid.measure(q[0], c[0]).x(q[0]);
    assert!(
        check(&mid.build().unwrap(), &profile)
            .violations
            .iter()
            .any(|v| matches!(v, Violation::MidCircuitMeasurementUnsupported { .. }))
    );
}

#[test]
fn every_violation_is_reported_together() {
    // Someone fixing a circuit wants the whole list, not the first problem.
    let mut b = CircuitBuilder::new("bad");
    let q = b.alloc_qubits(5);
    b.h(q[0]).gate(GateKind::T, [q[1]]).cx(q[0], q[3]);

    let report = check(&b.build().unwrap(), &builtin::linear_nisq(5));
    assert_eq!(report.violation_count(), 3);
}

#[test]
fn a_symbolic_parameter_cannot_be_domain_checked() {
    let profile = BasisProfileBuilder::new("bounded", "1", "test", Topology::linear(2))
        .operations(["rz"])
        .parameter_constraint("rz", oqci::target::ParameterConstraint::new(-1.0, 1.0))
        .cost_model("uniform")
        .build()
        .unwrap();

    let mut b = CircuitBuilder::new("ansatz");
    let q0 = b.alloc_qubit();
    b.rz(Param::symbol("theta"), q0);

    assert!(matches!(
        check(&b.build().unwrap(), &profile).violations.first(),
        Some(Violation::UnboundParameter { symbol, .. }) if symbol == "theta"
    ));
}

// --- Cost -------------------------------------------------------------------

#[test]
fn cost_components_survive_alongside_the_scalar() {
    // Stage E §7: reporting only a scalar would hide why a decision was made.
    let model = resolve("nisq-weighted").unwrap();
    let cost = model.evaluate(&bell(), &builtin::linear_nisq(5)).unwrap();

    assert_eq!(cost.total_gate_count, 4);
    assert_eq!(cost.two_qubit_count, 1);
    assert_eq!(cost.depth, 3);
    assert!(cost.scalar_score.is_some());
}

#[test]
fn the_same_circuit_costs_differently_on_different_targets() {
    // The whole point of Stage E: the target decides what is expensive.
    let circuit = bell();

    let simulator = resolve("uniform").unwrap();
    let nisq = resolve("nisq-weighted").unwrap();

    let on_simulator = simulator
        .evaluate(&circuit, &builtin::ideal_simulator())
        .unwrap();
    let on_nisq = nisq.evaluate(&circuit, &builtin::linear_nisq(5)).unwrap();

    assert_eq!(on_simulator.non_native_gate_count, 0);
    assert_eq!(on_nisq.non_native_gate_count, 1, "h needs decomposing");
    assert!(on_nisq.scalar_score > on_simulator.scalar_score);
}

#[test]
fn a_cost_model_reports_the_weights_behind_its_score() {
    // Stage E §6: a scalar with undisclosed weights is not evidence.
    let model = resolve("nisq-weighted").unwrap();
    let configuration = model.configuration();
    assert!(configuration.contains_key("two_qubit_weight"));
    assert!(configuration.contains_key("depth_weight"));
}

#[test]
fn unsupplied_estimates_stay_absent_rather_than_zero() {
    let model = resolve("uniform").unwrap();
    let cost = model
        .evaluate(&bell(), &builtin::ideal_simulator())
        .unwrap();
    assert_eq!(cost.estimated_duration, None);
    assert_eq!(cost.estimated_error, None);
}

// --- Seams with the rest of the compiler ------------------------------------

#[test]
fn a_parsed_program_can_be_checked_against_a_target() {
    let circuit = parse_openqasm3_named(
        "qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;",
        "parsed",
    )
    .unwrap();
    assert!(check(&circuit, &builtin::ideal_simulator()).is_legal());
}

#[test]
fn optimization_lowers_the_cost_it_is_measured_by() {
    // Ties the pass pipeline to the cost model: a circuit with a cancellable
    // pair must cost less after optimization.
    let circuit = parse_openqasm3_named(
        "qubit[2] q; h q[0]; x q[1]; x q[1]; cx q[0], q[1];",
        "redundant",
    )
    .unwrap();

    let profile = builtin::ideal_simulator();
    let model = resolve(profile.cost_model_id()).unwrap();

    let before = model.evaluate(&circuit, &profile).unwrap();
    let optimized = PassManager::default_pipeline()
        .run(&circuit, &PassSelection::All, &PassContext::none())
        .unwrap()
        .circuit;
    let after = model.evaluate(&optimized, &profile).unwrap();

    assert!(after.total_gate_count < before.total_gate_count);
    assert!(after.scalar_score < before.scalar_score);
}

#[test]
fn every_builtin_profile_names_a_resolvable_cost_model() {
    // A profile referring to a model nothing can resolve would make the
    // provenance it records false.
    for profile in builtin::all() {
        assert!(
            resolve(profile.cost_model_id()).is_some(),
            "{} references `{}`",
            profile.id(),
            profile.cost_model_id()
        );
    }
}

#[test]
fn a_profile_snapshots_reproducibly() {
    // Stage D §8: an experiment records which profile produced a result, so
    // the same profile must serialize identically every time.
    let once = serde_json::to_string(&builtin::linear_nisq(5)).unwrap();
    let twice = serde_json::to_string(&builtin::linear_nisq(5)).unwrap();
    assert_eq!(once, twice);
    assert!(once.contains("linear-nisq"));
}

#[test]
fn measurement_support_is_the_single_answer_for_measure_and_reset() {
    // The profile must not be able to contradict itself: listing `reset` in
    // the basis set does not override a device that cannot reset.
    let profile = BasisProfileBuilder::new("conflicted", "1", "test", Topology::linear(2))
        .operations(["x", "reset"])
        .measurement(MeasurementSupport {
            measurement: true,
            mid_circuit_measurement: true,
            reset: false,
        })
        .cost_model("uniform")
        .build()
        .unwrap();

    assert!(!profile.supports_operation("reset"));

    let mut b = CircuitBuilder::new("r");
    let q0 = b.alloc_qubit();
    b.reset(q0);
    let circuit = b.build().unwrap();

    // Legality and cost must agree about it.
    assert!(!check(&circuit, &profile).is_legal());
    let cost = resolve("uniform")
        .unwrap()
        .evaluate(&circuit, &profile)
        .unwrap();
    assert_eq!(cost.non_native_gate_count, 1);
}
