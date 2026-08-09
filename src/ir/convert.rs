//! QC-IR → QCO-IR conversion: deterministic linearization + dependency
//! extraction.
//!
//! The conversion is a single left-to-right pass over the QC-IR instruction
//! list that threads each wire (qubit or classical bit) from its boundary
//! [`crate::ir::qco::NodeKind::Input`] node through the operations that touch
//! it, in program order, to its boundary
//! [`crate::ir::qco::NodeKind::Output`] node.
//!
//! # Algorithm
//!
//! Maintain `last[w]` = the most recent node that wrote wire `w`, initialised to
//! each wire's input boundary node. For each instruction `op` in program order:
//!
//! 1. Add an operation node for `op`.
//! 2. For every wire `w` that `op` touches (its qubits, plus its target clbit
//!    for a measurement), add a dependency edge `last[w] → op` and set
//!    `last[w] = op`. The edge is [`DepKind::Control`] when `last[w]` is a
//!    state-collapsing op (measure/reset) on a **qubit** wire, otherwise
//!    [`DepKind::Data`].
//! 3. After all instructions, add an edge `last[w] → output(w)` for every wire.
//!
//! # Determinism
//!
//! Instructions are visited in program order; within an instruction, wires are
//! visited in a fixed order (qubit operands as written, then the clbit). Node
//! and edge insertion is therefore a pure function of the input circuit.
//!
//! # Semantics preservation (summary)
//!
//! The graph edges are exactly the ordering constraints that a valid execution
//! must respect: two operations are ordered iff they share a wire. Any
//! topological order of the graph is therefore a valid re-execution of the same
//! circuit, and the canonical order (by program index) reproduces the original
//! program exactly. The full argument is in `docs/ir_spec.md`.

use std::collections::HashMap;

use crate::ir::error::IrError;
use crate::ir::qc::Circuit;
use crate::ir::qco::{DepKind, NodeRef, QcoCircuit, Wire};
use crate::ir::types::ClbitId;

/// Converts a validated QC-IR [`Circuit`] into a QCO-IR [`QcoCircuit`].
///
/// The input must be a `Circuit` produced by
/// [`crate::ir::CircuitBuilder::build`] (hence already validated); this function
/// performs no further validation and cannot fail for well-formed input. The
/// [`Result`] exists only so that the internal DAG invariant can be surfaced
/// rather than panicked (it never returns [`Err`] for valid circuits).
///
/// # Errors
///
/// Never returns an error for a circuit obtained from `CircuitBuilder::build`.
/// The signature is fallible purely to avoid any `unwrap` on internal
/// invariants.
pub fn qc_to_qco(circuit: &Circuit) -> Result<QcoCircuit, IrError> {
    let mut dag =
        QcoCircuit::with_registers(circuit.name(), circuit.num_qubits(), circuit.num_clbits());

    // `last[w]` starts at each wire's input boundary node, and whether that
    // producer was a collapsing (measure/reset) op determines edge kind.
    let mut last: HashMap<Wire, NodeRef> = HashMap::new();
    let mut last_was_collapse: HashMap<Wire, bool> = HashMap::new();

    for q in 0..circuit.num_qubits() {
        let w = Wire::Qubit(crate::ir::QubitId(q));
        last.insert(w, dag.input_ref(w));
        last_was_collapse.insert(w, false);
    }
    for c in 0..circuit.num_clbits() {
        let w = Wire::Clbit(ClbitId(c));
        last.insert(w, dag.input_ref(w));
        last_was_collapse.insert(w, false);
    }

    for (index, inst) in circuit.instructions().iter().enumerate() {
        let node = dag.add_op(index, inst.clone());

        // Qubit wires this instruction touches, in operand order.
        for q in inst.qubits() {
            let w = Wire::Qubit(q);
            let prev = last[&w];
            // A control dependency is one whose *producer* was a collapsing op
            // on this qubit wire; otherwise it is a data dependency.
            let kind = if last_was_collapse[&w] {
                DepKind::Control
            } else {
                DepKind::Data
            };
            dag.add_dependency(prev, node, w, kind);
            last.insert(w, node);
            // Measurement and reset collapse the qubit's state.
            last_was_collapse.insert(w, !inst.is_unitary());
        }

        // Classical wire written by a measurement (value flow → Data).
        if let Some(c) = inst.clbit() {
            let w = Wire::Clbit(c);
            let prev = last[&w];
            dag.add_dependency(prev, node, w, DepKind::Data);
            last.insert(w, node);
            last_was_collapse.insert(w, false);
        }
    }

    // Close every wire onto its output boundary node.
    for q in 0..circuit.num_qubits() {
        let w = Wire::Qubit(crate::ir::QubitId(q));
        let kind = if last_was_collapse[&w] {
            DepKind::Control
        } else {
            DepKind::Data
        };
        dag.add_dependency(last[&w], dag.output_ref(w), w, kind);
    }
    for c in 0..circuit.num_clbits() {
        let w = Wire::Clbit(ClbitId(c));
        dag.add_dependency(last[&w], dag.output_ref(w), w, DepKind::Data);
    }

    // Confirm the DAG invariant holds (defensive; never fails for valid input).
    let _ = dag.topological_ops()?;
    Ok(dag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::GateKind;
    use crate::ir::qc::{CircuitBuilder, Instruction};
    use crate::ir::qco::NodeKind;

    fn count_edges(dag: &QcoCircuit, want: DepKind) -> usize {
        dag.dependencies()
            .filter(|(_, _, d)| d.kind == want)
            .count()
    }

    #[test]
    fn bell_has_expected_shape() {
        let mut b = CircuitBuilder::new("bell");
        let q0 = b.alloc_qubit();
        let q1 = b.alloc_qubit();
        b.h(q0).cx(q0, q1);
        let circuit = b.build().unwrap();
        let dag = qc_to_qco(&circuit).unwrap();

        assert_eq!(dag.op_count(), 2);
        // H then CX share q0, so a dependency chain exists; topo order valid.
        let ops = dag.topological_ops().unwrap();
        assert_eq!(ops.len(), 2);
        // H must come before CX (shared q0).
        let kinds: Vec<GateKind> = ops
            .iter()
            .filter_map(|(_, i)| match i {
                Instruction::Gate { kind, .. } => Some(kind.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(kinds, vec![GateKind::H, GateKind::Cx]);
    }

    #[test]
    fn measurement_after_gate_is_data_then_gate_after_measure_is_control() {
        // X q0 ; measure q0 -> c0 ; X q0  : the second X depends on a collapse.
        let mut b = CircuitBuilder::new("midmeasure");
        let q0 = b.alloc_qubit();
        let c0 = b.alloc_clbit();
        b.x(q0).measure(q0, c0).x(q0);
        let circuit = b.build().unwrap();
        let dag = qc_to_qco(&circuit).unwrap();

        // Exactly one control edge: from the measurement to the trailing X.
        assert_eq!(count_edges(&dag, DepKind::Control), 1);
        // Linearization reproduces program order.
        assert_eq!(dag.linearize(), circuit.instructions().to_vec());
    }

    #[test]
    fn reset_induces_control_dependency() {
        let mut b = CircuitBuilder::new("reset");
        let q0 = b.alloc_qubit();
        b.x(q0).reset(q0).x(q0);
        let circuit = b.build().unwrap();
        let dag = qc_to_qco(&circuit).unwrap();
        assert!(count_edges(&dag, DepKind::Control) >= 1);
    }

    #[test]
    fn boundary_nodes_exist_for_every_wire() {
        let mut b = CircuitBuilder::new("boundaries");
        let _q = b.alloc_qubits(3);
        let _c = b.alloc_clbits(2);
        let circuit = b.build().unwrap();
        let dag = qc_to_qco(&circuit).unwrap();
        let inputs = dag
            .nodes()
            .filter(|n| matches!(n.kind, NodeKind::Input(_)))
            .count();
        let outputs = dag
            .nodes()
            .filter(|n| matches!(n.kind, NodeKind::Output(_)))
            .count();
        assert_eq!(inputs, 5); // 3 qubits + 2 clbits
        assert_eq!(outputs, 5);
    }
}
