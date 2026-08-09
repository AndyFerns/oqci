//! Round-trip fidelity: QC-IR -> QCO-IR -> `linearize()` reproduces the
//! original instruction order for Bell, GHZ-3, and mid-circuit measurement.

use oqci::ir::{Circuit, CircuitBuilder, qc_to_qco};

fn bell() -> Circuit {
    let mut b = CircuitBuilder::new("bell");
    let q0 = b.alloc_qubit();
    let q1 = b.alloc_qubit();
    let c0 = b.alloc_clbit();
    let c1 = b.alloc_clbit();
    b.h(q0).cx(q0, q1).measure(q0, c0).measure(q1, c1);
    b.build().expect("valid bell")
}

fn ghz3() -> Circuit {
    let mut b = CircuitBuilder::new("ghz3");
    let qs = b.alloc_qubits(3);
    let cs = b.alloc_clbits(3);
    b.h(qs[0]).cx(qs[0], qs[1]).cx(qs[1], qs[2]);
    for (q, c) in qs.iter().zip(cs.iter()) {
        b.measure(*q, *c);
    }
    b.build().expect("valid ghz3")
}

fn mid_measure() -> Circuit {
    let mut b = CircuitBuilder::new("midmeasure");
    let q0 = b.alloc_qubit();
    let c0 = b.alloc_clbit();
    let c1 = b.alloc_clbit();
    b.x(q0).measure(q0, c0).x(q0).measure(q0, c1);
    b.build().expect("valid mid-measure")
}

/// Bell: linearize() equals the original instruction sequence.
#[test]
fn roundtrip_bell_linearize_preserves_order() {
    let circuit = bell();
    let dag = qc_to_qco(&circuit).expect("acyclic conversion");
    assert_eq!(dag.linearize(), circuit.instructions().to_vec());
}

/// GHZ-3: linearize() equals the original instruction sequence.
#[test]
fn roundtrip_ghz3_linearize_preserves_order() {
    let circuit = ghz3();
    let dag = qc_to_qco(&circuit).expect("acyclic conversion");
    assert_eq!(dag.linearize(), circuit.instructions().to_vec());
}

/// Mid-circuit measurement: linearize() equals the original instruction order.
#[test]
fn roundtrip_mid_circuit_measurement_linearize_preserves_order() {
    let circuit = mid_measure();
    let dag = qc_to_qco(&circuit).expect("acyclic conversion");
    assert_eq!(dag.linearize(), circuit.instructions().to_vec());
}

/// For these forced-order circuits, the topological index sequence equals the
/// canonical (linearize) index sequence 0..n.
#[test]
fn roundtrip_topological_matches_linearize_for_linear_circuits() {
    for circuit in [bell(), ghz3(), mid_measure()] {
        let dag = qc_to_qco(&circuit).expect("acyclic conversion");
        let topo: Vec<usize> = dag
            .topological_ops()
            .expect("acyclic")
            .into_iter()
            .map(|(idx, _)| idx)
            .collect();
        let expected: Vec<usize> = (0..circuit.len()).collect();
        assert_eq!(topo, expected);
    }
}
