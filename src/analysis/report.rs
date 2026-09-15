//! Resource reports: gate counts, depth, register sizes.
//!
//! `final-deliverables-spec.md` §13.1–13.3 requires gate counts (total,
//! one-qubit, two-qubit), DAG-aware depth, and a structured resource report.
//! This module is the **single place** any of those numbers is computed: the
//! pass manager's before/after bookkeeping and the CLI's `analyze` command
//! both call [`analyze`], so a metric shown in one view can never disagree
//! with the same metric in another.

use std::collections::BTreeMap;

use crate::ir::{Circuit, GateKind, Instruction, IrError, qc_to_qco};

/// A structured resource summary of a circuit.
///
/// Counts describe the circuit **as currently represented**. Target-native
/// gate counts and routing overhead (also named in §13) require a target
/// profile, which does not exist yet — they are deliberately absent rather
/// than reported as zero.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct ResourceReport {
    /// Declared qubit-register width.
    pub num_qubits: u32,
    /// Declared classical-register width.
    pub num_clbits: u32,
    /// Total instruction count, including measurement and reset.
    pub op_count: usize,
    /// Unitary gates acting on exactly one qubit.
    pub one_qubit_count: usize,
    /// Unitary gates acting on two or more qubits — the expensive ones on
    /// real hardware, and the figure routing overhead will later be measured
    /// against.
    pub two_qubit_count: usize,
    /// Measurement instructions.
    pub measure_count: usize,
    /// Reset instructions.
    pub reset_count: usize,
    /// Per-mnemonic counts, in deterministic (sorted) order. `measure` and
    /// `reset` appear here alongside gate mnemonics.
    pub gate_counts: BTreeMap<String, usize>,
    /// ASAP scheduling depth — see [`crate::ir::QcoCircuit::layers`].
    pub depth: usize,
    /// The widest scheduling layer: the most operations that could run at once.
    pub max_parallel_width: usize,
}

/// Computes a [`ResourceReport`] for a circuit.
///
/// ```
/// use oqci::analysis::analyze;
/// use oqci::ir::CircuitBuilder;
///
/// let mut b = CircuitBuilder::new("bell");
/// let q0 = b.alloc_qubit();
/// let q1 = b.alloc_qubit();
/// b.h(q0).cx(q0, q1);
///
/// let report = analyze(&b.build().unwrap()).unwrap();
/// assert_eq!(report.op_count, 2);
/// assert_eq!(report.one_qubit_count, 1);
/// assert_eq!(report.two_qubit_count, 1);
/// assert_eq!(report.depth, 2);
/// ```
///
/// # Errors
///
/// Propagates [`IrError`] from the QCO-IR conversion used to compute depth.
/// This cannot fail for a circuit obtained from `CircuitBuilder::build`.
pub fn analyze(circuit: &Circuit) -> Result<ResourceReport, IrError> {
    let dag = qc_to_qco(circuit)?;
    let layers = dag.layers()?;

    let mut report = ResourceReport {
        num_qubits: circuit.num_qubits(),
        num_clbits: circuit.num_clbits(),
        op_count: circuit.len(),
        depth: layers.len(),
        max_parallel_width: layers.iter().map(Vec::len).max().unwrap_or(0),
        ..ResourceReport::default()
    };

    for inst in circuit.instructions() {
        let mnemonic = match inst {
            Instruction::Gate { kind, qubits } => {
                if qubits.len() == 1 {
                    report.one_qubit_count += 1;
                } else {
                    report.two_qubit_count += 1;
                }
                gate_mnemonic(kind)
            }
            Instruction::Measure { .. } => {
                report.measure_count += 1;
                "measure".to_string()
            }
            Instruction::Reset { .. } => {
                report.reset_count += 1;
                "reset".to_string()
            }
        };
        *report.gate_counts.entry(mnemonic).or_insert(0) += 1;
    }

    Ok(report)
}

fn gate_mnemonic(kind: &GateKind) -> String {
    kind.mnemonic().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CircuitBuilder, Param};

    #[test]
    fn empty_circuit_reports_zeroes() {
        let report = analyze(&CircuitBuilder::new("empty").build().unwrap()).unwrap();
        assert_eq!(report, ResourceReport::default());
    }

    #[test]
    fn counts_split_one_and_multi_qubit_gates() {
        let mut b = CircuitBuilder::new("mixed");
        let q = b.alloc_qubits(3);
        b.h(q[0]).cx(q[0], q[1]).ccx(q[0], q[1], q[2]).x(q[2]);

        let report = analyze(&b.build().unwrap()).unwrap();
        assert_eq!(report.op_count, 4);
        assert_eq!(report.one_qubit_count, 2);
        assert_eq!(report.two_qubit_count, 2, "ccx counts as multi-qubit");
    }

    #[test]
    fn measure_and_reset_are_counted_separately_from_gates() {
        let mut b = CircuitBuilder::new("nonunitary");
        let q0 = b.alloc_qubit();
        let c0 = b.alloc_clbit();
        b.h(q0).measure(q0, c0).reset(q0);

        let report = analyze(&b.build().unwrap()).unwrap();
        assert_eq!(report.measure_count, 1);
        assert_eq!(report.reset_count, 1);
        assert_eq!(report.one_qubit_count, 1, "only the H is a gate");
        assert_eq!(report.gate_counts["measure"], 1);
        assert_eq!(report.gate_counts["reset"], 1);
        assert_eq!(report.gate_counts["h"], 1);
    }

    #[test]
    fn gate_counts_are_keyed_by_mnemonic() {
        let mut b = CircuitBuilder::new("repeat");
        let q0 = b.alloc_qubit();
        b.x(q0).x(q0).h(q0);

        let report = analyze(&b.build().unwrap()).unwrap();
        assert_eq!(report.gate_counts["x"], 2);
        assert_eq!(report.gate_counts["h"], 1);
    }

    #[test]
    fn symbolic_parameters_do_not_obstruct_analysis() {
        let mut b = CircuitBuilder::new("ansatz");
        let q0 = b.alloc_qubit();
        b.rz(Param::symbol("theta"), q0);

        let report = analyze(&b.build().unwrap()).unwrap();
        assert_eq!(report.gate_counts["rz"], 1);
        assert_eq!(report.depth, 1);
    }

    #[test]
    fn depth_and_width_come_from_the_dag() {
        let mut b = CircuitBuilder::new("wide");
        let q = b.alloc_qubits(4);
        b.h(q[0]).h(q[1]).h(q[2]).h(q[3]);

        let report = analyze(&b.build().unwrap()).unwrap();
        assert_eq!(report.depth, 1);
        assert_eq!(report.max_parallel_width, 4);
    }
}
