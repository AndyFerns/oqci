//! The adjacency primitive shared by the peephole passes.
//!
//! Every local rewrite in this crate — cancellation, rotation merging — asks
//! the same question: *is this operation immediately followed by that one,
//! with nothing in between?* Answering it on the instruction list is wrong
//! (see [`crate::pass::cancellation`]); answering it on QCO-IR is exact. This
//! module asks it once so the two passes cannot drift apart in their notion of
//! "adjacent".

use std::collections::HashMap;

use crate::ir::{Circuit, Instruction, IrError, NodeKind, QcoNode, Wire, qc_to_qco};

/// Stable identity for a QCO-IR node, usable as a map key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum NodeId {
    Input(Wire),
    Op(usize),
    Output(Wire),
}

fn node_id(node: &QcoNode) -> NodeId {
    match &node.kind {
        NodeKind::Input(wire) => NodeId::Input(*wire),
        NodeKind::Op { index, .. } => NodeId::Op(*index),
        NodeKind::Output(wire) => NodeId::Output(*wire),
    }
}

/// Maps a gate's program index to the index of the operation that immediately
/// follows it **on every wire it touches**.
///
/// An entry exists only when one operation follows on *all* of the gate's
/// wires. That unanimity is the precise statement of "nothing intervenes":
///
/// - A gate on another qubit does not appear, so it cannot block a rewrite it
///   is genuinely independent of (`H q0; H q1; H q0` still pairs the two
///   `H q0`s).
/// - Anything touching a shared wire *does* appear as that wire's successor,
///   so it breaks the unanimity and blocks the rewrite.
/// - A measurement or reset is itself a node on the wire, so a collapse
///   between two gates blocks them automatically — no special case needed.
///
/// Only gates are considered; measurement and reset never start a pair.
///
/// # Errors
///
/// Propagates [`IrError`] from the QCO-IR conversion.
pub(crate) fn unanimous_successors(circuit: &Circuit) -> Result<HashMap<usize, usize>, IrError> {
    let dag = qc_to_qco(circuit)?;

    // Each (node, wire) has exactly one outgoing edge: conversion threads
    // every wire linearly through the operations that touch it.
    let mut next_on_wire: HashMap<(NodeId, Wire), NodeId> = HashMap::new();
    for (from, to, dep) in dag.dependencies() {
        next_on_wire.insert((node_id(from), dep.wire), node_id(to));
    }

    let mut successors = HashMap::new();
    for (index, inst) in circuit.instructions().iter().enumerate() {
        let Instruction::Gate { qubits, .. } = inst else {
            continue;
        };

        let mut candidate: Option<usize> = None;
        let unanimous =
            qubits.iter().all(
                |q| match next_on_wire.get(&(NodeId::Op(index), Wire::Qubit(*q))) {
                    Some(NodeId::Op(next)) if candidate.is_none_or(|c| c == *next) => {
                        candidate = Some(*next);
                        true
                    }
                    _ => false,
                },
            );

        if let (true, Some(next)) = (unanimous, candidate) {
            successors.insert(index, next);
        }
    }

    Ok(successors)
}

/// The two instructions of a candidate pair, when both are gates acting on the
/// **identical operand list, in order**.
///
/// Operand order matters: `Cx q0,q1` and `Cx q1,q0` act on the same wires but
/// exchange control and target, so they are not a pair.
pub(crate) fn matched_pair(
    circuit: &Circuit,
    first: usize,
    second: usize,
) -> Option<(
    &crate::ir::GateKind,
    &crate::ir::GateKind,
    &Vec<crate::ir::QubitId>,
)> {
    let (
        Instruction::Gate {
            kind: a,
            qubits: a_qubits,
        },
        Instruction::Gate {
            kind: b,
            qubits: b_qubits,
        },
    ) = (
        circuit.instructions().get(first)?,
        circuit.instructions().get(second)?,
    )
    else {
        return None;
    };

    (a_qubits == b_qubits).then_some((a, b, a_qubits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CircuitBuilder, ClbitId, QubitId};

    fn successors(build: impl FnOnce(&mut CircuitBuilder)) -> HashMap<usize, usize> {
        let mut b = CircuitBuilder::new("c");
        b.alloc_qubits(3);
        b.alloc_clbits(1);
        build(&mut b);
        unanimous_successors(&b.build().unwrap()).unwrap()
    }

    #[test]
    fn consecutive_gates_on_one_wire_are_adjacent() {
        let succ = successors(|b| {
            b.h(QubitId(0)).x(QubitId(0));
        });
        assert_eq!(succ.get(&0), Some(&1));
    }

    #[test]
    fn a_gate_on_another_wire_does_not_break_adjacency() {
        let succ = successors(|b| {
            b.h(QubitId(0)).h(QubitId(1)).x(QubitId(0));
        });
        assert_eq!(succ.get(&0), Some(&2), "index 1 is on an independent wire");
    }

    #[test]
    fn a_shared_wire_makes_the_intervening_gate_the_successor() {
        let succ = successors(|b| {
            b.h(QubitId(0)).cx(QubitId(0), QubitId(1)).x(QubitId(0));
        });
        assert_eq!(succ.get(&0), Some(&1), "the cx shares q0");
    }

    #[test]
    fn a_two_qubit_gate_needs_unanimity_across_both_wires() {
        // The cx is followed by `x q0` on one wire and `y q1` on the other,
        // so it has no unanimous successor.
        let succ = successors(|b| {
            b.cx(QubitId(0), QubitId(1)).x(QubitId(0)).y(QubitId(1));
        });
        assert_eq!(succ.get(&0), None);
    }

    #[test]
    fn a_two_qubit_gate_pairs_with_an_identical_one() {
        let succ = successors(|b| {
            b.cx(QubitId(0), QubitId(1)).cx(QubitId(0), QubitId(1));
        });
        assert_eq!(succ.get(&0), Some(&1));
    }

    #[test]
    fn a_measurement_is_the_successor_and_blocks_the_pair() {
        let succ = successors(|b| {
            b.x(QubitId(0))
                .measure(QubitId(0), ClbitId(0))
                .x(QubitId(0));
        });
        // The successor is the measurement at index 1, not the trailing gate.
        assert_eq!(succ.get(&0), Some(&1));
        // And the measurement itself never starts a pair.
        assert_eq!(succ.get(&1), None);
    }

    #[test]
    fn a_trailing_gate_has_no_successor() {
        let succ = successors(|b| {
            b.h(QubitId(0));
        });
        assert_eq!(succ.get(&0), None, "its successor is the output boundary");
    }

    #[test]
    fn matched_pair_requires_identical_operand_order() {
        let mut b = CircuitBuilder::new("c");
        b.alloc_qubits(2);
        b.cx(QubitId(0), QubitId(1)).cx(QubitId(1), QubitId(0));
        let circuit = b.build().unwrap();
        assert!(matched_pair(&circuit, 0, 1).is_none());

        let mut b = CircuitBuilder::new("c");
        b.alloc_qubits(2);
        b.cx(QubitId(0), QubitId(1)).cx(QubitId(0), QubitId(1));
        let circuit = b.build().unwrap();
        assert!(matched_pair(&circuit, 0, 1).is_some());
    }
}
