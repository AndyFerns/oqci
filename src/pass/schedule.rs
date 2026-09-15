//! DAG scheduling: report the parallelism the dependency graph proves exists.
//!
//! `final-deliverables-spec.md` §8.5 asks that QCO-IR dependencies be used to
//! "expose parallelism while respecting wire dependencies, control barriers
//! and measurement/reset semantics," and warns: "Do not reorder operations
//! solely because they touch different qubits if an explicit
//! dependency/barrier forbids it."
//!
//! This pass takes the strictest possible reading of that: it **never
//! reorders anything at all**. It computes the ASAP layering
//! ([`crate::ir::QcoCircuit::layers`]) and reports it. Operations sharing a
//! layer are provably independent — no path in the graph connects them — so
//! the report is a statement about the circuit, not a rearrangement of it.
//!
//! Reordering only becomes meaningful once there is a target whose
//! constraints make one schedule better than another, which is Stage D/Phase
//! 3 work. Until then a reordering pass would be choosing between schedules
//! on no evidence, and the deterministic program order QCO-IR already
//! preserves is as good a choice as any.

use crate::ir::{Circuit, qc_to_qco};
use crate::pass::{Pass, PassError, PassOutput};

/// Reports scheduling depth and width without modifying the circuit. See the
/// [module docs](self).
pub struct Schedule;

impl Pass for Schedule {
    fn id(&self) -> &'static str {
        "schedule"
    }

    fn description(&self) -> &'static str {
        "report ASAP scheduling layers (analysis only; never reorders)"
    }

    fn run(&self, circuit: &Circuit) -> Result<PassOutput, PassError> {
        let dag = qc_to_qco(circuit).map_err(|e| PassError::from_ir(self.id(), e))?;
        let layers = dag.layers().map_err(|e| PassError::from_ir(self.id(), e))?;

        let depth = layers.len();
        let width = layers.iter().map(Vec::len).max().unwrap_or(0);
        let ops = circuit.len();

        Ok(PassOutput::unchanged(circuit.clone()).with_note(format!(
            "depth {depth}, {ops} operation(s), max parallel width {width}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::CircuitBuilder;

    fn run(build: impl FnOnce(&mut CircuitBuilder)) -> PassOutput {
        let mut b = CircuitBuilder::new("c");
        build(&mut b);
        Schedule.run(&b.build().unwrap()).unwrap()
    }

    #[test]
    fn the_circuit_is_never_modified() {
        let mut b = CircuitBuilder::new("c");
        let q0 = b.alloc_qubit();
        let q1 = b.alloc_qubit();
        b.h(q0).cx(q0, q1).x(q1);
        let circuit = b.build().unwrap();

        let out = Schedule.run(&circuit).unwrap();
        assert!(!out.changed);
        assert_eq!(out.circuit, circuit, "analysis passes rewrite nothing");
    }

    #[test]
    fn reports_depth_and_width_for_parallel_gates() {
        let out = run(|b| {
            let q = b.alloc_qubits(3);
            b.h(q[0]).h(q[1]).h(q[2]);
        });
        assert_eq!(
            out.notes,
            vec!["depth 1, 3 operation(s), max parallel width 3"]
        );
    }

    #[test]
    fn reports_depth_for_a_dependency_chain() {
        let out = run(|b| {
            let q0 = b.alloc_qubit();
            b.x(q0).y(q0).z(q0);
        });
        assert_eq!(
            out.notes,
            vec!["depth 3, 3 operation(s), max parallel width 1"]
        );
    }

    #[test]
    fn an_empty_circuit_reports_zero_depth() {
        let out = run(|_| {});
        assert_eq!(
            out.notes,
            vec!["depth 0, 0 operation(s), max parallel width 0"]
        );
    }
}
