//! Integration tests for the OpenQASM 3 frontend, end to end.
//!
//! Mirrors `tests/pipeline.rs`'s corpus — identity/empty, Bell, GHZ-3,
//! mid-circuit measurement, reset, opaque gates, parameterized rotations —
//! but driven from source text rather than a hand-built `CircuitBuilder`, plus
//! one rejection test per construct outside the documented subset.
//!
//! The key assertion style here is *equivalence*: a parsed circuit must emit
//! byte-identical QIR to the equivalent hand-built circuit. That is what makes
//! the frontend a faithful mapping onto the existing IR rather than a second,
//! subtly different definition of it.

use std::collections::HashMap;

use oqci::frontend::{FrontendError, parse_openqasm3, parse_openqasm3_named};
use oqci::ir::{CircuitBuilder, GateKind, Instruction, IrError, Param, QubitId};
use oqci::ir::{bind_parameters, emit_qir, qc_to_qco};

/// Parses, converts and lowers, returning the QIR text.
fn qir_of_source(source: &str) -> String {
    let circuit = parse_openqasm3_named(source, "itest").expect("valid program");
    let dag = qc_to_qco(&circuit).expect("conversion");
    emit_qir(&dag).expect("lowering")
}

/// Builds, converts and lowers, returning the QIR text.
fn qir_of_builder(build: impl FnOnce(&mut CircuitBuilder)) -> String {
    let mut b = CircuitBuilder::new("itest");
    build(&mut b);
    let circuit = b.build().expect("valid circuit");
    emit_qir(&qc_to_qco(&circuit).expect("conversion")).expect("lowering")
}

fn expect_error(source: &str) -> FrontendError {
    parse_openqasm3(source).expect_err("expected the frontend to refuse this program")
}

// --- Required corpus --------------------------------------------------------

#[test]
fn empty_program_is_the_identity_circuit() {
    let circuit = parse_openqasm3("OPENQASM 3.0;").unwrap();
    assert!(circuit.is_empty());
    assert_eq!(circuit.num_qubits(), 0);
}

#[test]
fn declarations_without_operations_allocate_registers() {
    let circuit = parse_openqasm3("qubit[3] q; bit[2] c;").unwrap();
    assert_eq!((circuit.num_qubits(), circuit.num_clbits()), (3, 2));
    assert!(circuit.is_empty());
}

#[test]
fn bell_state_matches_the_hand_built_circuit() {
    let parsed = qir_of_source(
        r#"
        OPENQASM 3.0;
        include "stdgates.inc";
        qubit[2] q;
        bit[2] c;
        h q[0];
        cx q[0], q[1];
        c = measure q;
        "#,
    );

    let expected = qir_of_builder(|b| {
        let q0 = b.alloc_qubit();
        let q1 = b.alloc_qubit();
        let c0 = b.alloc_clbit();
        let c1 = b.alloc_clbit();
        b.h(q0).cx(q0, q1).measure(q0, c0).measure(q1, c1);
    });

    assert_eq!(parsed, expected);
}

#[test]
fn ghz3_matches_the_hand_built_circuit() {
    let parsed = qir_of_source(
        r#"
        qubit[3] q;
        h q[0];
        cx q[0], q[1];
        cx q[1], q[2];
        "#,
    );

    let expected = qir_of_builder(|b| {
        let q = b.alloc_qubits(3);
        b.h(q[0]).cx(q[0], q[1]).cx(q[1], q[2]);
    });

    assert_eq!(parsed, expected);
}

#[test]
fn mid_circuit_measurement_keeps_program_order() {
    let circuit = parse_openqasm3(
        r#"
        qubit[2] q;
        bit[1] c;
        h q[0];
        measure q[0] -> c[0];
        x q[1];
        "#,
    )
    .unwrap();

    assert_eq!(circuit.len(), 3);
    assert!(matches!(
        circuit.instructions()[1],
        Instruction::Measure { .. }
    ));
    assert!(matches!(
        circuit.instructions()[2],
        Instruction::Gate { .. }
    ));
}

#[test]
fn both_measure_spellings_agree() {
    let arrow = qir_of_source("qubit[1] q; bit[1] c; measure q[0] -> c[0];");
    let assign = qir_of_source("qubit[1] q; bit[1] c; c[0] = measure q[0];");
    assert_eq!(arrow, assign);
}

#[test]
fn reset_lowers_to_the_reset_intrinsic() {
    let qir = qir_of_source("qubit[1] q; reset q[0];");
    assert!(qir.contains("@__quantum__qis__reset__body"));
}

#[test]
fn unrecognized_gate_lowers_as_an_extended_intrinsic() {
    let qir = qir_of_source("qubit[2] q; iswap q[0], q[1];");
    assert!(qir.contains("@__quantum__qis__iswap__body"));
}

#[test]
fn angle_arithmetic_folds_to_the_same_bits_as_a_literal() {
    let parsed = qir_of_source("qubit[1] q; rz(pi/2) q[0];");
    let expected = qir_of_builder(|b| {
        let q0 = b.alloc_qubit();
        b.rz(std::f64::consts::FRAC_PI_2, q0);
    });
    assert_eq!(parsed, expected);
}

#[test]
fn legacy_u_spellings_map_onto_the_registered_set() {
    let circuit = parse_openqasm3(
        r#"
        qubit[1] q;
        u1(0.5) q[0];
        u2(0.1, 0.2) q[0];
        u3(0.1, 0.2, 0.3) q[0];
        "#,
    )
    .unwrap();

    assert_eq!(
        circuit.instructions()[0],
        Instruction::Gate {
            kind: GateKind::P(Param::concrete(0.5)),
            qubits: vec![QubitId(0)]
        }
    );
    assert!(matches!(
        circuit.instructions()[1],
        Instruction::Gate {
            kind: GateKind::U { .. },
            ..
        }
    ));
    assert_eq!(
        circuit.instructions()[2],
        Instruction::Gate {
            kind: GateKind::U {
                theta: Param::concrete(0.1),
                phi: Param::concrete(0.2),
                lambda: Param::concrete(0.3),
            },
            qubits: vec![QubitId(0)]
        }
    );
}

// --- Parameterized circuits (Stage F) ---------------------------------------

#[test]
fn input_parameters_survive_to_binding_and_lowering() {
    let circuit = parse_openqasm3_named(
        r#"
        OPENQASM 3.0;
        input float[64] theta;
        qubit[1] q;
        ry(theta) q[0];
        "#,
        "itest",
    )
    .unwrap();

    assert_eq!(circuit.parameters(), vec!["theta".to_string()]);

    // Unbound, the circuit refuses to lower.
    let dag = qc_to_qco(&circuit).unwrap();
    assert!(matches!(
        emit_qir(&dag),
        Err(IrError::UnboundParameter { .. })
    ));

    // Bound, it is indistinguishable from the literal circuit.
    let bound = bind_parameters(&circuit, &HashMap::from([("theta".to_string(), 0.75)])).unwrap();
    let bound_qir = emit_qir(&qc_to_qco(&bound).unwrap()).unwrap();
    let expected = qir_of_builder(|b| {
        let q0 = b.alloc_qubit();
        b.ry(0.75, q0);
    });
    assert_eq!(bound_qir, expected);
}

// --- Refusals: everything outside the documented subset ----------------------

#[test]
fn classical_control_flow_is_refused() {
    for source in [
        "qubit[1] q; bit[1] c; if (c == 1) { x q[0]; }",
        "qubit[1] q; for i in [0:2] { x q[0]; }",
        "qubit[1] q; while (true) { x q[0]; }",
    ] {
        assert!(
            matches!(expect_error(source), FrontendError::Unsupported(_)),
            "{source}"
        );
    }
}

#[test]
fn subroutines_and_custom_gate_definitions_are_refused() {
    for source in ["gate mygate a { x a; }", "def f() { }", "extern g(int);"] {
        assert!(
            matches!(expect_error(source), FrontendError::Unsupported(_)),
            "{source}"
        );
    }
}

#[test]
fn output_declarations_and_barriers_are_refused() {
    for source in ["output bit c;", "qubit[2] q; barrier q;"] {
        assert!(
            matches!(expect_error(source), FrontendError::Unsupported(_)),
            "{source}"
        );
    }
}

#[test]
fn compound_symbolic_expressions_are_refused() {
    let error = expect_error("input float[64] theta; qubit[1] q; rz(2*theta) q[0];");
    assert!(matches!(
        error,
        FrontendError::Unsupported(msg) if msg.contains("compound expression")
    ));
}

#[test]
fn openqasm2_register_syntax_is_refused() {
    assert!(matches!(
        expect_error("qreg q[2]; creg c[2];"),
        FrontendError::Unsupported(_)
    ));
}

#[test]
fn malformed_source_reports_a_position() {
    let error = expect_error("qubit[2] q;\nh q[0]\n");
    let FrontendError::Syntax { line, .. } = error else {
        panic!("expected a syntax error, got {error:?}");
    };
    assert_eq!(
        line, 3,
        "the missing `;` should be reported where it is due"
    );
}

#[test]
fn undeclared_register_is_refused() {
    assert!(matches!(
        expect_error("h nonexistent[0];"),
        FrontendError::Semantic(_)
    ));
}

#[test]
fn out_of_range_index_is_refused() {
    assert!(matches!(
        expect_error("qubit[2] q; x q[9];"),
        FrontendError::Semantic(_)
    ));
}

#[test]
fn ir_invariants_are_enforced_through_the_frontend() {
    // The frontend does not re-implement QC-IR's validation; it surfaces it.
    assert!(matches!(
        expect_error("qubit[2] q; cx q[0], q[0];"),
        FrontendError::Ir(IrError::DuplicateQubit { .. })
    ));
    assert!(matches!(
        expect_error("qubit[2] q; cx q[0];"),
        FrontendError::Ir(IrError::GateArityMismatch { .. })
    ));
}
