//! QCO-IR DAG structure: boundary-node counts, exact Data/Control edge
//! classification, and the Kahn program-index tiebreak on a diamond.

use std::collections::HashMap;

use oqci::ir::{CircuitBuilder, DepKind, Instruction, NodeKind, Wire, qc_to_qco};

/// Boundary invariant: each wire gets one Input and one Output node, so node
/// count == ops + 2*(qubits + clbits).
#[test]
fn boundary_node_count_matches_two_times_wires() {
    let mut b = CircuitBuilder::new("boundaries");
    let qs = b.alloc_qubits(3);
    let cs = b.alloc_clbits(2);
    b.h(qs[0]).cx(qs[0], qs[1]).measure(qs[0], cs[0]);
    let circuit = b.build().expect("valid circuit");
    let dag = qc_to_qco(&circuit).expect("acyclic conversion");

    let inputs = dag
        .nodes()
        .filter(|n| matches!(n.kind, NodeKind::Input(_)))
        .count();
    let outputs = dag
        .nodes()
        .filter(|n| matches!(n.kind, NodeKind::Output(_)))
        .count();
    let wires = 3 + 2;
    assert_eq!(inputs, wires);
    assert_eq!(outputs, wires);
    assert_eq!(dag.nodes().count(), dag.op_count() + 2 * wires);
}

/// Edge classification: an edge is Control iff its source is a collapsing op
/// (Measure/Reset) AND the wire is a qubit; everything else is Data.
#[test]
fn depkind_biconditional_control_iff_collapse_on_qubit_wire() {
    let mut b = CircuitBuilder::new("mixed");
    let q0 = b.alloc_qubit();
    let q1 = b.alloc_qubit();
    let c0 = b.alloc_clbit();
    b.x(q0).measure(q0, c0).x(q0).reset(q1).h(q1);
    let circuit = b.build().expect("valid circuit");
    let dag = qc_to_qco(&circuit).expect("acyclic conversion");

    for (from, _to, dep) in dag.dependencies() {
        let source_collapses = matches!(
            from.as_op(),
            Some((_, Instruction::Measure { .. })) | Some((_, Instruction::Reset { .. }))
        );
        let on_qubit_wire = matches!(dep.wire, Wire::Qubit(_));
        let expect_control = source_collapses && on_qubit_wire;
        assert_eq!(dep.kind == DepKind::Control, expect_control);
    }
}

/// Sanity partner to the biconditional: no Control edge ever originates from a
/// plain gate or from a classical wire.
#[test]
fn data_edges_only_from_gate_or_clbit_producers() {
    let mut b = CircuitBuilder::new("mixed2");
    let q0 = b.alloc_qubit();
    let c0 = b.alloc_clbit();
    b.h(q0).measure(q0, c0).reset(q0);
    let circuit = b.build().expect("valid circuit");
    let dag = qc_to_qco(&circuit).expect("acyclic conversion");

    let mut control_edges = 0usize;
    for (from, _to, dep) in dag.dependencies() {
        if dep.kind == DepKind::Control {
            control_edges += 1;
            assert!(matches!(dep.wire, Wire::Qubit(_)));
            assert!(matches!(
                from.as_op(),
                Some((_, Instruction::Measure { .. })) | Some((_, Instruction::Reset { .. }))
            ));
        }
    }
    // measure->reset and reset->output are both collapse-sourced qubit edges.
    assert!(control_edges >= 1);
}

/// Kahn tiebreak: on a diamond (A -> {B,C} -> D) the program-index tiebreak
/// emits B before C, reproducing program order [0,1,2,3].
#[test]
fn diamond_kahn_tiebreak_yields_program_order() {
    let mut b = CircuitBuilder::new("diamond");
    let q0 = b.alloc_qubit();
    let q1 = b.alloc_qubit();
    b.cx(q0, q1) // A (idx 0): writes q0 and q1
        .h(q0) // B (idx 1): depends on A via q0
        .h(q1) // C (idx 2): depends on A via q1
        .cx(q0, q1); // D (idx 3): depends on B via q0 and C via q1
    let circuit = b.build().expect("valid circuit");
    let dag = qc_to_qco(&circuit).expect("acyclic conversion");

    let order: Vec<usize> = dag
        .topological_ops()
        .expect("acyclic")
        .into_iter()
        .map(|(idx, _)| idx)
        .collect();
    assert_eq!(order, vec![0, 1, 2, 3]);
}

/// The topological order is a valid linear extension: for every op->op edge the
/// source appears before the destination.
#[test]
fn topological_order_is_a_valid_linear_extension() {
    let mut b = CircuitBuilder::new("extension");
    let q0 = b.alloc_qubit();
    let q1 = b.alloc_qubit();
    b.cx(q0, q1).h(q0).h(q1).cx(q0, q1);
    let circuit = b.build().expect("valid circuit");
    let dag = qc_to_qco(&circuit).expect("acyclic conversion");

    let pos: HashMap<usize, usize> = dag
        .topological_ops()
        .expect("acyclic")
        .into_iter()
        .enumerate()
        .map(|(p, (idx, _))| (idx, p))
        .collect();

    for (from, to, _dep) in dag.dependencies() {
        if let (Some((src, _)), Some((dst, _))) = (from.as_op(), to.as_op()) {
            let ps = pos.get(&src).expect("src op ordered");
            let pd = pos.get(&dst).expect("dst op ordered");
            assert!(ps < pd, "edge {src}->{dst} violates topological order");
        }
    }
}
