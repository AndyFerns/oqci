//! Integration tests for the full QC-IR → QCO-IR → QIR pipeline.
//!
//! Covers the required corpus — identity/empty, Bell, GHZ-3, mid-circuit
//! measurement — end to end, plus one malformed circuit per validation rule to
//! exercise every error path through the public API.

use oqci::ir::{Angle, CircuitBuilder, GateKind, IrError, QubitId, emit_qir, qc_to_qco};

/// Runs a builder through the whole pipeline and returns the emitted QIR.
fn pipeline(build: impl FnOnce(&mut CircuitBuilder)) -> String {
    let mut b = CircuitBuilder::new("itest");
    build(&mut b);
    let circuit = b.build().expect("valid circuit");
    let dag = qc_to_qco(&circuit).expect("conversion");
    emit_qir(&dag).expect("lowering")
}

// --- Required corpus: full pipeline -----------------------------------------

#[test]
fn identity_empty_circuit() {
    let b = CircuitBuilder::new("identity");
    let circuit = b.build().unwrap();
    assert!(circuit.is_empty());

    let dag = qc_to_qco(&circuit).unwrap();
    assert_eq!(dag.op_count(), 0);
    assert!(dag.topological_ops().unwrap().is_empty());

    let qir = emit_qir(&dag).unwrap();
    // A valid, empty entry point with just `ret void`.
    assert!(qir.contains("define void @identity() #0"));
    assert!(qir.contains("ret void"));
    assert!(qir.contains("required_num_qubits\"=\"0\""));
}

#[test]
fn bell_state_full_pipeline() {
    let qir = pipeline(|b| {
        let q0 = b.alloc_qubit();
        let q1 = b.alloc_qubit();
        let c0 = b.alloc_clbit();
        let c1 = b.alloc_clbit();
        b.h(q0).cx(q0, q1).measure(q0, c0).measure(q1, c1);
    });
    assert!(qir.contains("@__quantum__qis__h__body"));
    assert!(qir.contains("@__quantum__qis__cnot__body"));
    assert_eq!(
        qir.matches("call void @__quantum__qis__mz__body").count(),
        2
    );
    assert_eq!(
        qir.matches("call void @__quantum__rt__result_record_output")
            .count(),
        2
    );
    assert!(qir.contains("required_num_qubits\"=\"2\""));
    assert!(qir.contains("required_num_results\"=\"2\""));
}

#[test]
fn ghz3_full_pipeline() {
    // GHZ-3: H q0 ; CX q0,q1 ; CX q1,q2 ; measure all.
    let mut b = CircuitBuilder::new("ghz3");
    let qs = b.alloc_qubits(3);
    let cs = b.alloc_clbits(3);
    b.h(qs[0]).cx(qs[0], qs[1]).cx(qs[1], qs[2]);
    for (q, c) in qs.iter().zip(cs.iter()) {
        b.measure(*q, *c);
    }
    let circuit = b.build().unwrap();
    assert_eq!(circuit.num_qubits(), 3);

    let dag = qc_to_qco(&circuit).unwrap();
    assert_eq!(dag.op_count(), 6);

    // The two CX gates share q1, so they must be ordered: CX(q0,q1) before
    // CX(q1,q2) in every topological order.
    let ops = dag.topological_ops().unwrap();
    let cx_positions: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter_map(|(pos, (_, inst))| match inst {
            oqci::ir::Instruction::Gate {
                kind: GateKind::Cx, ..
            } => Some(pos),
            _ => None,
        })
        .collect();
    assert_eq!(cx_positions.len(), 2);
    assert!(cx_positions[0] < cx_positions[1]);

    let qir = emit_qir(&dag).unwrap();
    assert_eq!(
        qir.matches("call void @__quantum__qis__cnot__body").count(),
        2
    );
    assert!(qir.contains("required_num_qubits\"=\"3\""));
}

#[test]
fn mid_circuit_measurement_full_pipeline() {
    // X q0 ; measure q0 -> c0 ; X q0 ; measure q0 -> c1.
    let mut b = CircuitBuilder::new("midmeasure");
    let q0 = b.alloc_qubit();
    let c0 = b.alloc_clbit();
    let c1 = b.alloc_clbit();
    b.x(q0).measure(q0, c0).x(q0).measure(q0, c1);
    let circuit = b.build().unwrap();

    let dag = qc_to_qco(&circuit).unwrap();
    // Linearization must reproduce program order exactly.
    assert_eq!(dag.linearize(), circuit.instructions().to_vec());

    // Both operations after each measurement stay ordered on q0's wire: the
    // only valid topological order is the program order.
    let topo_indices: Vec<usize> = dag
        .topological_ops()
        .unwrap()
        .into_iter()
        .map(|(idx, _)| idx)
        .collect();
    assert_eq!(topo_indices, vec![0, 1, 2, 3]);

    let qir = emit_qir(&dag).unwrap();
    assert_eq!(
        qir.matches("call void @__quantum__qis__mz__body").count(),
        2
    );
}

#[test]
fn opaque_gate_flows_through_pipeline() {
    // The escape hatch must survive conversion and lowering.
    let qir = pipeline(|b| {
        let q0 = b.alloc_qubit();
        let q1 = b.alloc_qubit();
        b.gate(
            GateKind::Opaque {
                name: "iswap".into(),
                params: vec![Angle::new(0.5)],
            },
            [q0, q1],
        );
    });
    assert!(qir.contains("@__quantum__qis__iswap__body"));
    assert!(qir.contains(&format!("double 0x{:016X}", 0.5f64.to_bits())));
}

// --- One malformed circuit per validation rule ------------------------------

#[test]
fn err_qubit_out_of_range() {
    let mut b = CircuitBuilder::new("bad");
    let _q0 = b.alloc_qubit();
    b.gate(GateKind::X, [QubitId(9)]);
    assert!(matches!(b.build(), Err(IrError::QubitOutOfRange { .. })));
}

#[test]
fn err_clbit_out_of_range() {
    let mut b = CircuitBuilder::new("bad");
    let q0 = b.alloc_qubit();
    b.measure(q0, oqci::ir::ClbitId(3));
    assert!(matches!(b.build(), Err(IrError::ClbitOutOfRange { .. })));
}

#[test]
fn err_gate_arity_mismatch() {
    let mut b = CircuitBuilder::new("bad");
    let q0 = b.alloc_qubit();
    let _q1 = b.alloc_qubit();
    b.gate(GateKind::Ccx, [q0]); // needs 3
    assert!(matches!(
        b.build(),
        Err(IrError::GateArityMismatch { expected: 3, .. })
    ));
}

#[test]
fn err_duplicate_qubit() {
    let mut b = CircuitBuilder::new("bad");
    let q0 = b.alloc_qubit();
    let _q1 = b.alloc_qubit();
    b.gate(GateKind::Cx, [q0, q0]);
    assert!(matches!(b.build(), Err(IrError::DuplicateQubit { .. })));
}

#[test]
fn err_empty_opaque_name() {
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
fn err_empty_opaque_operands() {
    let mut b = CircuitBuilder::new("bad");
    b.gate(
        GateKind::Opaque {
            name: "custom".into(),
            params: vec![],
        },
        [],
    );
    assert!(matches!(
        b.build(),
        Err(IrError::EmptyOpaqueOperands { .. })
    ));
}

#[test]
fn err_non_finite_angle() {
    let mut b = CircuitBuilder::new("bad");
    let q0 = b.alloc_qubit();
    b.rz(Angle::new(f64::INFINITY), q0);
    assert!(matches!(b.build(), Err(IrError::NonFiniteAngle { .. })));
}
