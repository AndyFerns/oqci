//! Integration tests for the Qiskit adapter's language-neutral core.
//!
//! These drive `QiskitCircuitIr` — the struct the PyO3 boundary fills in from
//! a live `QuantumCircuit` — through the full pipeline, with no Python
//! interpreter involved. That is the point of the split: the translation
//! decisions are testable here, and the thin Python layer is verified
//! separately by `python/tests/test_adapter.py`.
//!
//! As with the OpenQASM tests, correctness is asserted by *equivalence* to the
//! hand-built circuit rather than by inspecting the adapter's own output.

use std::collections::HashMap;

use oqci::frontend::qiskit::translate;
use oqci::frontend::{FrontendError, QiskitCircuitIr, QiskitInstruction, QiskitParam};
use oqci::ir::{CircuitBuilder, IrError, bind_parameters, emit_qir, qc_to_qco};

fn ir(num_qubits: u32, num_clbits: u32, instructions: Vec<QiskitInstruction>) -> QiskitCircuitIr {
    QiskitCircuitIr {
        name: "itest".into(),
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

fn qir_of(circuit: &QiskitCircuitIr) -> String {
    let translated = translate(circuit).expect("valid circuit");
    emit_qir(&qc_to_qco(&translated).expect("conversion")).expect("lowering")
}

fn qir_of_builder(build: impl FnOnce(&mut CircuitBuilder)) -> String {
    let mut b = CircuitBuilder::new("itest");
    build(&mut b);
    emit_qir(&qc_to_qco(&b.build().unwrap()).unwrap()).unwrap()
}

#[test]
fn empty_circuit_is_the_identity() {
    let circuit = translate(&ir(0, 0, vec![])).unwrap();
    assert!(circuit.is_empty());
}

#[test]
fn bell_matches_the_hand_built_circuit() {
    let adapted = qir_of(&ir(
        2,
        2,
        vec![
            QiskitInstruction::gate("h", vec![], vec![0]),
            QiskitInstruction::gate("cx", vec![], vec![0, 1]),
            measure(vec![0], vec![0]),
            measure(vec![1], vec![1]),
        ],
    ));

    let expected = qir_of_builder(|b| {
        let q0 = b.alloc_qubit();
        let q1 = b.alloc_qubit();
        let c0 = b.alloc_clbit();
        let c1 = b.alloc_clbit();
        b.h(q0).cx(q0, q1).measure(q0, c0).measure(q1, c1);
    });

    assert_eq!(adapted, expected);
}

#[test]
fn ghz3_matches_the_hand_built_circuit() {
    let adapted = qir_of(&ir(
        3,
        0,
        vec![
            QiskitInstruction::gate("h", vec![], vec![0]),
            QiskitInstruction::gate("cx", vec![], vec![0, 1]),
            QiskitInstruction::gate("cx", vec![], vec![1, 2]),
        ],
    ));

    let expected = qir_of_builder(|b| {
        let q = b.alloc_qubits(3);
        b.h(q[0]).cx(q[0], q[1]).cx(q[1], q[2]);
    });

    assert_eq!(adapted, expected);
}

#[test]
fn the_two_frontends_agree_on_the_same_circuit() {
    // The shared gate table means an equivalent program through either
    // frontend must produce identical IR — this is the test that would fail
    // if the two adapters ever drifted apart.
    let via_qiskit = qir_of(&ir(
        2,
        2,
        vec![
            QiskitInstruction::gate("h", vec![], vec![0]),
            QiskitInstruction::gate("rz", vec![QiskitParam::Concrete(0.5)], vec![0]),
            QiskitInstruction::gate("cx", vec![], vec![0, 1]),
            measure(vec![0], vec![0]),
            measure(vec![1], vec![1]),
        ],
    ));

    let via_openqasm = {
        let circuit = oqci::frontend::parse_openqasm3_named(
            r#"
            qubit[2] q;
            bit[2] c;
            h q[0];
            rz(0.5) q[0];
            cx q[0], q[1];
            c = measure q;
            "#,
            "itest",
        )
        .unwrap();
        emit_qir(&qc_to_qco(&circuit).unwrap()).unwrap()
    };

    assert_eq!(via_qiskit, via_openqasm);
}

#[test]
fn parameterized_circuit_binds_then_lowers() {
    let circuit = translate(&ir(
        1,
        0,
        vec![QiskitInstruction::gate(
            "ry",
            vec![QiskitParam::Symbol("theta".into())],
            vec![0],
        )],
    ))
    .unwrap();

    assert_eq!(circuit.parameters(), vec!["theta".to_string()]);
    assert!(matches!(
        emit_qir(&qc_to_qco(&circuit).unwrap()),
        Err(IrError::UnboundParameter { .. })
    ));

    let bound = bind_parameters(&circuit, &HashMap::from([("theta".to_string(), 0.75)])).unwrap();
    let bound_qir = emit_qir(&qc_to_qco(&bound).unwrap()).unwrap();
    assert_eq!(
        bound_qir,
        qir_of_builder(|b| {
            let q0 = b.alloc_qubit();
            b.ry(0.75, q0);
        })
    );
}

#[test]
fn barriers_do_not_reach_the_ir() {
    let adapted = qir_of(&ir(
        2,
        0,
        vec![
            QiskitInstruction::gate("h", vec![], vec![0]),
            QiskitInstruction::gate("barrier", vec![], vec![0, 1]),
            QiskitInstruction::gate("cx", vec![], vec![0, 1]),
        ],
    ));

    let expected = qir_of_builder(|b| {
        let q0 = b.alloc_qubit();
        let q1 = b.alloc_qubit();
        b.h(q0).cx(q0, q1);
    });

    assert_eq!(adapted, expected);
}

#[test]
fn control_flow_is_refused() {
    let error = translate(&ir(
        1,
        1,
        vec![QiskitInstruction::gate("if_else", vec![], vec![0])],
    ))
    .unwrap_err();
    assert!(matches!(error, FrontendError::Unsupported(_)));
}

#[test]
fn compound_parameter_expression_is_refused() {
    let error = translate(&ir(
        1,
        0,
        vec![QiskitInstruction::gate(
            "rz",
            vec![QiskitParam::Expression("theta + phi".into())],
            vec![0],
        )],
    ))
    .unwrap_err();
    assert!(matches!(error, FrontendError::Unsupported(_)));
}

#[test]
fn ir_invariants_are_enforced_through_the_adapter() {
    assert!(matches!(
        translate(&ir(
            1,
            0,
            vec![QiskitInstruction::gate("x", vec![], vec![4])]
        ))
        .unwrap_err(),
        FrontendError::Ir(IrError::QubitOutOfRange { .. })
    ));
    assert!(matches!(
        translate(&ir(
            2,
            0,
            vec![QiskitInstruction::gate("cx", vec![], vec![0, 0])]
        ))
        .unwrap_err(),
        FrontendError::Ir(IrError::DuplicateQubit { .. })
    ));
}
