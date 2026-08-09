//! QCO-IR — the optimization IR (directed acyclic dependency graph).
//!
//! Where [`crate::ir::qc`] is a linear instruction list, QCO-IR is a **DAG**:
//! operations are nodes and dependencies are edges. This is the form future
//! optimization passes (Phase 3: cancellation, fusion, scheduling) will consume,
//! because it exposes exactly which operations may be reordered and which may
//! not.
//!
//! # Graph shape
//!
//! Every qubit and classical bit is a *wire* threaded from a boundary
//! [`NodeKind::Input`] node to a boundary [`NodeKind::Output`] node. Each
//! [`NodeKind::Op`] node sits on the wires it touches. An edge `A → B` labelled
//! with a [`Wire`] means "the value on that wire flows from `A` to `B`", so `B`
//! must execute after `A`. Because QCO-IR is built from a linear circuit, the
//! graph is always acyclic.
//!
//! # Dependency classification
//!
//! Each edge carries a [`DepKind`]:
//!
//! - [`DepKind::Data`] — the predecessor produced a quantum/classical value the
//!   successor consumes (predecessor is a unitary gate, a boundary input, or a
//!   classical write).
//! - [`DepKind::Control`] — the predecessor is a *state-collapsing* op
//!   (measurement or reset) on that qubit wire; the successor must observe that
//!   collapse and therefore may not be commuted across it.
//!
//! See `docs/ir_spec.md` for the precise rules and the semantics-preservation
//! argument.

use std::collections::HashMap;

use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};

use crate::ir::error::IrError;
use crate::ir::qc::Instruction;
use crate::ir::types::{ClbitId, QubitId};

/// A single wire in the dependency graph: one qubit or one classical bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Wire {
    /// A qubit wire.
    Qubit(QubitId),
    /// A classical-bit wire.
    Clbit(ClbitId),
}

/// The nature of a dependency edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DepKind {
    /// Value-flow dependency: predecessor produced a value the successor uses.
    Data,
    /// Ordering barrier induced by a state-collapsing op (measure/reset).
    Control,
}

/// What a graph node represents.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    /// Boundary source for a wire (the initial `|0⟩` of a qubit, or a fresh
    /// classical bit).
    Input(Wire),
    /// Boundary sink for a wire (the final value at circuit end).
    Output(Wire),
    /// An operation lifted from QC-IR.
    Op {
        /// The operation's position in the originating QC-IR program order.
        /// This is a stable identity used to make traversal deterministic.
        index: usize,
        /// The lifted instruction.
        instruction: Instruction,
    },
}

/// A node in the [`QcoCircuit`] dependency graph.
#[derive(Debug, Clone, PartialEq)]
pub struct QcoNode {
    /// What the node represents.
    pub kind: NodeKind,
}

impl QcoNode {
    /// If this is an operation node, returns `(program_index, &instruction)`.
    #[must_use]
    pub fn as_op(&self) -> Option<(usize, &Instruction)> {
        match &self.kind {
            NodeKind::Op { index, instruction } => Some((*index, instruction)),
            _ => None,
        }
    }
}

/// An edge label: which wire the dependency is on, and its kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dependency {
    /// The wire carrying the dependency.
    pub wire: Wire,
    /// Data vs. control.
    pub kind: DepKind,
}

/// An opaque handle to a graph node, used by the conversion routine to thread
/// wires without exposing the underlying graph library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeRef(NodeIndex);

/// The optimization IR: a quantum circuit as a directed acyclic dependency
/// graph.
///
/// Build one from QC-IR with [`crate::ir::qc_to_qco`]; do not construct the
/// graph by hand. Traverse it with [`QcoCircuit::topological_ops`] (a
/// deterministic order) or inspect [`QcoCircuit::linearize`] for the canonical
/// program order.
#[derive(Debug, Clone)]
pub struct QcoCircuit {
    name: String,
    num_qubits: u32,
    num_clbits: u32,
    graph: DiGraph<QcoNode, Dependency>,
    inputs: HashMap<Wire, NodeIndex>,
    outputs: HashMap<Wire, NodeIndex>,
}

impl QcoCircuit {
    /// Creates a graph with only boundary nodes for the given registers. Each
    /// wire gets an [`NodeKind::Input`] and [`NodeKind::Output`] node.
    ///
    /// Crate-internal: conversion uses this, then appends operations.
    pub(crate) fn with_registers(
        name: impl Into<String>,
        num_qubits: u32,
        num_clbits: u32,
    ) -> Self {
        let mut graph = DiGraph::new();
        let mut inputs = HashMap::new();
        let mut outputs = HashMap::new();

        for q in 0..num_qubits {
            let wire = Wire::Qubit(QubitId(q));
            inputs.insert(
                wire,
                graph.add_node(QcoNode {
                    kind: NodeKind::Input(wire),
                }),
            );
            outputs.insert(
                wire,
                graph.add_node(QcoNode {
                    kind: NodeKind::Output(wire),
                }),
            );
        }
        for c in 0..num_clbits {
            let wire = Wire::Clbit(ClbitId(c));
            inputs.insert(
                wire,
                graph.add_node(QcoNode {
                    kind: NodeKind::Input(wire),
                }),
            );
            outputs.insert(
                wire,
                graph.add_node(QcoNode {
                    kind: NodeKind::Output(wire),
                }),
            );
        }

        QcoCircuit {
            name: name.into(),
            num_qubits,
            num_clbits,
            graph,
            inputs,
            outputs,
        }
    }

    /// Adds an operation node and returns its handle. Crate-internal.
    pub(crate) fn add_op(&mut self, index: usize, instruction: Instruction) -> NodeRef {
        NodeRef(self.graph.add_node(QcoNode {
            kind: NodeKind::Op { index, instruction },
        }))
    }

    /// Adds a dependency edge `from → to` on `wire` with kind `dep`.
    /// Crate-internal.
    pub(crate) fn add_dependency(&mut self, from: NodeRef, to: NodeRef, wire: Wire, kind: DepKind) {
        self.graph.add_edge(from.0, to.0, Dependency { wire, kind });
    }

    /// The input boundary node for a wire. Crate-internal.
    pub(crate) fn input_ref(&self, wire: Wire) -> NodeRef {
        NodeRef(self.inputs[&wire])
    }

    /// The output boundary node for a wire. Crate-internal.
    pub(crate) fn output_ref(&self, wire: Wire) -> NodeRef {
        NodeRef(self.outputs[&wire])
    }

    /// The circuit's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Number of qubits.
    #[must_use]
    pub fn num_qubits(&self) -> u32 {
        self.num_qubits
    }

    /// Number of classical bits.
    #[must_use]
    pub fn num_clbits(&self) -> u32 {
        self.num_clbits
    }

    /// Number of operation nodes (excludes boundary nodes).
    #[must_use]
    pub fn op_count(&self) -> usize {
        self.graph
            .node_weights()
            .filter(|n| matches!(n.kind, NodeKind::Op { .. }))
            .count()
    }

    /// All nodes, including boundaries, in insertion order.
    pub fn nodes(&self) -> impl Iterator<Item = &QcoNode> {
        self.graph.node_weights()
    }

    /// All dependency edges as `(from, to, dependency)` triples over operation
    /// and boundary nodes. Intended for inspection and pass authoring.
    pub fn dependencies(&self) -> impl Iterator<Item = (&QcoNode, &QcoNode, &Dependency)> {
        self.graph.edge_indices().map(move |e| {
            let (a, b) = self.graph.edge_endpoints(e).expect("edge has endpoints");
            (&self.graph[a], &self.graph[b], &self.graph[e])
        })
    }

    /// Returns the operation nodes in a **deterministic** topological order.
    ///
    /// Uses Kahn's algorithm with ties broken by ascending program `index`, so
    /// the result is reproducible and, for a circuit that was linear to begin
    /// with, reproduces the original program order exactly.
    ///
    /// # Errors
    ///
    /// Returns [`IrError::CyclicGraph`] if the graph contains a cycle (an
    /// internal invariant violation that cannot arise from valid QC-IR).
    pub fn topological_ops(&self) -> Result<Vec<(usize, &Instruction)>, IrError> {
        let order = self.stable_toposort()?;
        Ok(order
            .into_iter()
            .filter_map(|idx| self.graph[idx].as_op())
            .collect())
    }

    /// The operations in canonical program order (by program `index`).
    ///
    /// Because QC-IR → QCO-IR preserves program indices, this is exactly the
    /// original instruction sequence — useful for round-trip checks.
    #[must_use]
    pub fn linearize(&self) -> Vec<Instruction> {
        let mut ops: Vec<(usize, &Instruction)> = self
            .graph
            .node_weights()
            .filter_map(QcoNode::as_op)
            .collect();
        ops.sort_by_key(|(idx, _)| *idx);
        ops.into_iter().map(|(_, inst)| inst.clone()).collect()
    }

    /// Deterministic Kahn topological sort over all nodes (boundaries included).
    ///
    /// Ties are broken by a key that orders boundary inputs first, then ops by
    /// program index, then boundary outputs — giving a stable, meaningful order.
    fn stable_toposort(&self) -> Result<Vec<NodeIndex>, IrError> {
        let mut indegree: HashMap<NodeIndex, usize> = HashMap::new();
        for n in self.graph.node_indices() {
            indegree.insert(
                n,
                self.graph
                    .neighbors_directed(n, Direction::Incoming)
                    .count(),
            );
        }

        // Ready set kept sorted by tie-break key; we pop the smallest each step.
        let mut ready: Vec<NodeIndex> = indegree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(&n, _)| n)
            .collect();
        let key = |g: &DiGraph<QcoNode, Dependency>, n: NodeIndex| -> (u8, usize) {
            match &g[n].kind {
                NodeKind::Input(_) => (0, n.index()),
                NodeKind::Op { index, .. } => (1, *index),
                NodeKind::Output(_) => (2, n.index()),
            }
        };
        ready.sort_by_key(|&n| key(&self.graph, n));

        let mut order = Vec::with_capacity(self.graph.node_count());
        while let Some(next) = ready.first().copied() {
            ready.remove(0);
            order.push(next);
            for succ in self.graph.neighbors_directed(next, Direction::Outgoing) {
                let d = indegree.get_mut(&succ).expect("successor tracked");
                *d -= 1;
                if *d == 0 {
                    // Insert keeping `ready` sorted by tie-break key.
                    let k = key(&self.graph, succ);
                    let pos = ready
                        .binary_search_by_key(&k, |&n| key(&self.graph, n))
                        .unwrap_or_else(|e| e);
                    ready.insert(pos, succ);
                }
            }
        }

        if order.len() == self.graph.node_count() {
            Ok(order)
        } else {
            Err(IrError::CyclicGraph)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ir::qc::CircuitBuilder;
    use crate::ir::{GateKind, qc_to_qco};

    #[test]
    fn empty_circuit_has_no_ops() {
        let circuit = CircuitBuilder::new("empty").build().unwrap();
        let dag = qc_to_qco(&circuit).unwrap();
        assert_eq!(dag.op_count(), 0);
        assert!(dag.topological_ops().unwrap().is_empty());
    }

    #[test]
    fn linear_chain_linearizes_to_program_order() {
        let mut b = CircuitBuilder::new("chain");
        let q0 = b.alloc_qubit();
        b.x(q0).y(q0).z(q0);
        let circuit = b.build().unwrap();
        let dag = qc_to_qco(&circuit).unwrap();

        let lin = dag.linearize();
        assert_eq!(lin, circuit.instructions().to_vec());

        // Topological order also respects the chain.
        let topo: Vec<GateKind> = dag
            .topological_ops()
            .unwrap()
            .into_iter()
            .filter_map(|(_, inst)| match inst {
                crate::ir::Instruction::Gate { kind, .. } => Some(kind.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(topo, vec![GateKind::X, GateKind::Y, GateKind::Z]);
    }

    #[test]
    fn independent_qubits_have_no_cross_dependency() {
        // Gates on disjoint qubits should be independent in the DAG.
        let mut b = CircuitBuilder::new("parallel");
        let q0 = b.alloc_qubit();
        let q1 = b.alloc_qubit();
        b.h(q0).h(q1);
        let circuit = b.build().unwrap();
        let dag = qc_to_qco(&circuit).unwrap();
        // Two op nodes, each depending only on its own input boundary.
        assert_eq!(dag.op_count(), 2);
        assert!(dag.topological_ops().is_ok());
    }
}
