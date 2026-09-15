//! Property-based proof that the optimization passes preserve semantics.
//!
//! This is the test that makes the passes trustworthy. Every other test
//! asserts that a pass did what its author *intended* — cancelled a pair,
//! merged two rotations. These assert the thing that actually matters: that
//! the circuit still computes the same state afterwards.
//!
//! The check is `|⟨ψ_before|ψ_after⟩| ≈ 1` — equality up to global phase,
//! which is unobservable and which several legitimate rewrites change.
//! Semantics come from an independent matrix implementation
//! (`support::statevector`), not from the tables the passes consult, so a
//! mis-signed angle in a rewrite rule cannot hide behind a matching mistake.
//!
//! Circuits here are unitary-only and fully bound: a measurement has no
//! deterministic state-vector action, and a symbolic parameter has no value
//! to simulate.

mod support;

use proptest::prelude::*;

use oqci::ir::{Circuit, CircuitBuilder, GateKind, Param, QubitId};
use oqci::pass::{Canonicalize, GateCancellation, Pass, PassManager, PassSelection, RotationMerge};
use support::statevector::{same_state_up_to_global_phase, simulate};

/// Angles drawn from a small set including exact negatives and zero, so that
/// the exact-negation cancellation rule and the zero-angle canonicalization
/// actually fire instead of being tested only on paths that never trigger.
const ANGLES: &[f64] = &[
    0.0,
    0.25,
    -0.25,
    std::f64::consts::FRAC_PI_2,
    -std::f64::consts::FRAC_PI_2,
    std::f64::consts::PI,
    -std::f64::consts::PI,
];

/// One generated instruction, before qubit assignment.
#[derive(Debug, Clone)]
enum GateSpec {
    Unary(GateKind),
    Binary(GateKind),
    Ternary(GateKind),
}

fn gate_strategy() -> impl Strategy<Value = GateSpec> {
    let angle = prop::sample::select(ANGLES);
    prop_oneof![
        // Parameterless single-qubit gates, the self-inverse family among them.
        prop::sample::select(vec![
            GateKind::I,
            GateKind::X,
            GateKind::Y,
            GateKind::Z,
            GateKind::H,
            GateKind::S,
            GateKind::Sdg,
            GateKind::T,
            GateKind::Tdg,
        ])
        .prop_map(GateSpec::Unary),
        // Rotations — the merge and exact-negation-cancel paths.
        (0usize..4, angle.clone()).prop_map(|(which, a)| {
            let p = Param::concrete(a);
            GateSpec::Unary(match which {
                0 => GateKind::Rx(p),
                1 => GateKind::Ry(p),
                2 => GateKind::Rz(p),
                _ => GateKind::P(p),
            })
        }),
        // The general single-qubit unitary, which no pass may touch.
        (angle.clone(), angle.clone(), angle).prop_map(|(t, p, l)| {
            GateSpec::Unary(GateKind::U {
                theta: Param::concrete(t),
                phi: Param::concrete(p),
                lambda: Param::concrete(l),
            })
        }),
        prop::sample::select(vec![
            GateKind::Cx,
            GateKind::Cy,
            GateKind::Cz,
            GateKind::Swap,
        ])
        .prop_map(GateSpec::Binary),
        Just(GateSpec::Ternary(GateKind::Ccx)),
    ]
}

/// Generates a random unitary circuit over `1..=3` qubits.
///
/// The qubit count is small and the alphabet narrow on purpose: adjacent
/// identical gates and same-axis rotation pairs then occur often enough that
/// the passes genuinely fire, rather than the property passing vacuously on
/// circuits with nothing to optimize.
fn circuit_strategy() -> impl Strategy<Value = Circuit> {
    (
        1usize..=3,
        prop::collection::vec((gate_strategy(), 0usize..3, 0usize..3, 0usize..3), 0..12),
    )
        .prop_map(|(num_qubits, specs)| {
            let mut b = CircuitBuilder::new("generated");
            b.alloc_qubits(num_qubits as u32);

            for (spec, a, b_idx, c_idx) in specs {
                // Distinct operands, wrapped into the declared register.
                let q0 = a % num_qubits;
                let q1 = (q0 + 1 + (b_idx % num_qubits.max(1))) % num_qubits;
                let q2 = (q1 + 1 + (c_idx % num_qubits.max(1))) % num_qubits;

                match spec {
                    GateSpec::Unary(kind) => {
                        b.gate(kind, [QubitId(q0 as u32)]);
                    }
                    GateSpec::Binary(kind) if num_qubits >= 2 && q0 != q1 => {
                        b.gate(kind, [QubitId(q0 as u32), QubitId(q1 as u32)]);
                    }
                    GateSpec::Ternary(kind)
                        if num_qubits >= 3 && q0 != q1 && q1 != q2 && q0 != q2 =>
                    {
                        b.gate(
                            kind,
                            [QubitId(q0 as u32), QubitId(q1 as u32), QubitId(q2 as u32)],
                        );
                    }
                    // Not enough distinct qubits for this gate — skip it
                    // rather than emitting an invalid circuit.
                    _ => {}
                }
            }
            b.build()
                .expect("generated circuits are valid by construction")
        })
}

/// Asserts a pass preserved the circuit's action, with a failure message that
/// shows the circuit that broke it.
fn assert_preserves_semantics(pass: &dyn Pass, circuit: &Circuit) {
    let output = pass
        .run(circuit)
        .unwrap_or_else(|e| panic!("{} failed on {:?}: {e}", pass.id(), circuit.instructions()));

    let before = simulate(circuit);
    let after = simulate(&output.circuit);

    assert!(
        same_state_up_to_global_phase(&before, &after),
        "{} changed circuit semantics\n  before: {:?}\n  after:  {:?}",
        pass.id(),
        circuit.instructions(),
        output.circuit.instructions()
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn canonicalize_preserves_semantics(circuit in circuit_strategy()) {
        assert_preserves_semantics(&Canonicalize, &circuit);
    }

    #[test]
    fn gate_cancellation_preserves_semantics(circuit in circuit_strategy()) {
        assert_preserves_semantics(&GateCancellation, &circuit);
    }

    #[test]
    fn rotation_merge_preserves_semantics(circuit in circuit_strategy()) {
        assert_preserves_semantics(&RotationMerge, &circuit);
    }

    #[test]
    fn the_default_pipeline_preserves_semantics(circuit in circuit_strategy()) {
        let result = PassManager::default_pipeline()
            .run(&circuit, &PassSelection::All)
            .expect("the default pipeline should not fail");

        let before = simulate(&circuit);
        let after = simulate(&result.circuit);
        prop_assert!(
            same_state_up_to_global_phase(&before, &after),
            "the pipeline changed semantics\n  before: {:?}\n  after:  {:?}",
            circuit.instructions(),
            result.circuit.instructions()
        );
    }

    #[test]
    fn optimization_never_increases_operation_count(circuit in circuit_strategy()) {
        let result = PassManager::default_pipeline()
            .run(&circuit, &PassSelection::All)
            .expect("the default pipeline should not fail");
        prop_assert!(result.circuit.len() <= circuit.len());
    }
}

// --- Targeted regressions: the specific rules, on circuits that exercise them.

fn circuit(build: impl FnOnce(&mut CircuitBuilder)) -> Circuit {
    let mut b = CircuitBuilder::new("t");
    b.alloc_qubits(3);
    build(&mut b);
    b.build().unwrap()
}

#[test]
fn self_inverse_cancellation_is_semantics_preserving() {
    for gate in [GateKind::X, GateKind::Y, GateKind::Z, GateKind::H] {
        // Sandwiched inside an H so the state is a superposition, where a
        // wrong rewrite would actually show up.
        let c = circuit(|b| {
            b.h(QubitId(0))
                .gate(gate.clone(), [QubitId(0)])
                .gate(gate.clone(), [QubitId(0)]);
        });
        assert_preserves_semantics(&GateCancellation, &c);
    }
}

#[test]
fn adjoint_cancellation_is_semantics_preserving() {
    for (a, b) in [
        (GateKind::S, GateKind::Sdg),
        (GateKind::T, GateKind::Tdg),
        (GateKind::Tdg, GateKind::T),
    ] {
        let c = circuit(|builder| {
            builder
                .h(QubitId(0))
                .gate(a.clone(), [QubitId(0)])
                .gate(b.clone(), [QubitId(0)]);
        });
        assert_preserves_semantics(&GateCancellation, &c);
    }
}

#[test]
fn rotation_merging_is_semantics_preserving_on_every_axis() {
    let c = circuit(|b| {
        b.h(QubitId(0))
            .rx(0.3, QubitId(0))
            .rx(0.4, QubitId(0))
            .ry(0.1, QubitId(0))
            .ry(0.2, QubitId(0))
            .rz(0.5, QubitId(0))
            .rz(0.6, QubitId(0));
    });
    assert_preserves_semantics(&RotationMerge, &c);
}

#[test]
fn phase_gate_merging_is_semantics_preserving() {
    let c = circuit(|b| {
        b.h(QubitId(0))
            .gate(GateKind::P(Param::concrete(0.3)), [QubitId(0)])
            .gate(GateKind::P(Param::concrete(0.4)), [QubitId(0)]);
    });
    assert_preserves_semantics(&RotationMerge, &c);
}

#[test]
fn two_qubit_cancellation_is_semantics_preserving() {
    let c = circuit(|b| {
        b.h(QubitId(0))
            .h(QubitId(1))
            .cx(QubitId(0), QubitId(1))
            .cx(QubitId(0), QubitId(1));
    });
    assert_preserves_semantics(&GateCancellation, &c);
}

#[test]
fn cancellation_through_an_independent_wire_is_semantics_preserving() {
    // The rewrite that textual adjacency would miss — and the one most likely
    // to be wrong if the adjacency rule were sloppy.
    let c = circuit(|b| {
        b.h(QubitId(0))
            .h(QubitId(1))
            .x(QubitId(0))
            .y(QubitId(1))
            .x(QubitId(0));
    });
    assert_preserves_semantics(&GateCancellation, &c);
}

#[test]
fn zero_angle_removal_is_semantics_preserving() {
    let c = circuit(|b| {
        b.h(QubitId(0))
            .rz(0.0, QubitId(0))
            .gate(GateKind::I, [QubitId(0)]);
    });
    assert_preserves_semantics(&Canonicalize, &c);
}

#[test]
fn a_fully_cancelling_circuit_collapses_to_the_identity() {
    // H;X;X;H on |0> is the identity — and the pipeline should discover that.
    let c = circuit(|b| {
        b.h(QubitId(0)).x(QubitId(0)).x(QubitId(0)).h(QubitId(0));
    });
    let result = PassManager::default_pipeline()
        .run(&c, &PassSelection::All)
        .unwrap();

    assert!(result.circuit.is_empty(), "everything should cancel");
    assert!(same_state_up_to_global_phase(
        &simulate(&c),
        &simulate(&result.circuit)
    ));
}
