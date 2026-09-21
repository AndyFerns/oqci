//! Rotation merging: combine adjacent same-axis rotations.
//!
//! `Rz(a); Rz(b) → Rz(a+b)`, and likewise for `Rx`, `Ry` and the phase gate
//! `P`. All four are one-parameter families whose composition along a fixed
//! axis is simply the sum of the angles — no decomposition or synthesis is
//! involved, which is why this rewrite is safe to state outright.
//!
//! Adjacency is the QCO-IR notion used by [`crate::pass::cancellation`], so
//! `Rz(a) q0; H q1; Rz(b) q0` merges (the `H` is independent) while
//! `Rz(a) q0; H q0; Rz(b) q0` does not.
//!
//! # Scope: this is both "gate fusion" and "rotation merging"
//!
//! `final-deliverables-spec.md` lists §8.3 Gate Fusion and §8.4 Rotation
//! Merging separately. This one pass implements the additive-parameter
//! fusion both describe, and OQCI implements **nothing beyond that**: there
//! is no fusion of arbitrary consecutive gates into a synthesized unitary, no
//! two-qubit block collapsing, no Euler-angle recombination of `U` gates.
//!
//! That is a deliberate reading of §8.3's own instruction — "Do not claim
//! arbitrary unitary synthesis unless it is actually implemented" — rather
//! than a gap. Building a second pass that fused `U` gates would require
//! matrix synthesis this project has not written and cannot yet verify.
//! `docs/pass_manager.md` states the same boundary.
//!
//! # Symbolic parameters are never merged
//!
//! Merging `Rz(θ); Rz(φ)` would require a parameter meaning "θ + φ", and
//! [`Param`] represents a symbol, not an expression (see `docs/ir_spec.md`
//! §1.1). Folding the two into one symbol would silently lose a term, so a
//! pair is merged only when **both** angles are concrete. Stage F §6 makes
//! the same demand from the other direction: passes must preserve parameter
//! semantics, and must not evaluate symbolic parameters prematurely.

use std::collections::HashMap;

use crate::ir::{Circuit, GateKind, Instruction, Param};
use crate::pass::adjacency::{matched_pair, unanimous_successors};
use crate::pass::{Pass, PassContext, PassError, PassOutput, rebuild};

/// Merges adjacent same-axis rotations. See the [module docs](self).
pub struct RotationMerge;

impl Pass for RotationMerge {
    fn id(&self) -> &'static str {
        "rotation-merge"
    }

    fn description(&self) -> &'static str {
        "combine adjacent same-axis rotations (Rz(a); Rz(b) -> Rz(a+b))"
    }

    fn run(&self, circuit: &Circuit, _context: &PassContext<'_>) -> Result<PassOutput, PassError> {
        let mut current = circuit.clone();
        let mut merged = 0usize;

        // Merging can expose a further merge (`Rz(a); Rz(b); Rz(c)` takes two
        // rounds), so iterate to a fixed point. Each round removes at least
        // one instruction, so this terminates.
        loop {
            let rewrites = plan_merges(&current).map_err(|e| PassError::from_ir(self.id(), e))?;
            if rewrites.is_empty() {
                break;
            }
            merged += rewrites.len();

            let instructions = rewrite(&current, &rewrites);
            current = rebuild(self.id(), &current, instructions)?;
        }

        if merged == 0 {
            return Ok(PassOutput::unchanged(circuit.clone()));
        }
        Ok(PassOutput {
            circuit: current,
            changed: true,
            notes: vec![format!("merged {merged} rotation pair(s)")],
        })
    }
}

/// A planned merge: replace the gate at `first` with `combined`, and drop the
/// gate at `second`.
struct Merge {
    first: usize,
    second: usize,
    combined: GateKind,
}

/// Finds non-overlapping mergeable pairs, greedily in ascending index order.
fn plan_merges(circuit: &Circuit) -> Result<Vec<Merge>, crate::ir::IrError> {
    let successors = unanimous_successors(circuit)?;
    let mut merges: Vec<Merge> = Vec::new();
    let mut claimed: HashMap<usize, ()> = HashMap::new();

    for index in 0..circuit.len() {
        if claimed.contains_key(&index) {
            continue;
        }
        let Some(&partner) = successors.get(&index) else {
            continue;
        };
        if claimed.contains_key(&partner) {
            continue;
        }
        let Some((first, second, _)) = matched_pair(circuit, index, partner) else {
            continue;
        };
        let Some(combined) = combine(first, second) else {
            continue;
        };

        claimed.insert(index, ());
        claimed.insert(partner, ());
        merges.push(Merge {
            first: index,
            second: partner,
            combined,
        });
    }

    Ok(merges)
}

/// Applies planned merges to produce a new instruction sequence.
fn rewrite(circuit: &Circuit, merges: &[Merge]) -> Vec<Instruction> {
    let replacements: HashMap<usize, &GateKind> =
        merges.iter().map(|m| (m.first, &m.combined)).collect();
    let dropped: HashMap<usize, ()> = merges.iter().map(|m| (m.second, ())).collect();

    circuit
        .instructions()
        .iter()
        .enumerate()
        .filter(|(i, _)| !dropped.contains_key(i))
        .map(|(i, inst)| match (replacements.get(&i), inst) {
            (Some(kind), Instruction::Gate { qubits, .. }) => Instruction::Gate {
                kind: (*kind).clone(),
                qubits: qubits.clone(),
            },
            _ => inst.clone(),
        })
        .collect()
}

/// The single gate equivalent to applying `a` then `b`, when one exists in the
/// same one-parameter family.
fn combine(a: &GateKind, b: &GateKind) -> Option<GateKind> {
    match (a, b) {
        (GateKind::Rx(p), GateKind::Rx(q)) => add(p, q).map(GateKind::Rx),
        (GateKind::Ry(p), GateKind::Ry(q)) => add(p, q).map(GateKind::Ry),
        (GateKind::Rz(p), GateKind::Rz(q)) => add(p, q).map(GateKind::Rz),
        (GateKind::P(p), GateKind::P(q)) => add(p, q).map(GateKind::P),
        _ => None,
    }
}

/// Sums two parameters, but only when both are concrete — see the module docs
/// on why symbolic parameters are never merged.
fn add(a: &Param, b: &Param) -> Option<Param> {
    let (a, b) = (a.as_concrete()?, b.as_concrete()?);
    Some(Param::concrete(a.radians() + b.radians()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CircuitBuilder, ClbitId, QubitId};

    fn run(build: impl FnOnce(&mut CircuitBuilder)) -> PassOutput {
        let mut b = CircuitBuilder::new("c");
        b.alloc_qubits(3);
        b.alloc_clbits(1);
        build(&mut b);
        RotationMerge
            .run(&b.build().unwrap(), &PassContext::none())
            .unwrap()
    }

    fn only_gate(out: &PassOutput) -> &GateKind {
        match &out.circuit.instructions()[0] {
            Instruction::Gate { kind, .. } => kind,
            other => panic!("expected a gate, got {other:?}"),
        }
    }

    #[test]
    fn same_axis_rotations_sum() {
        let out = run(|b| {
            b.rz(0.25, QubitId(0)).rz(0.5, QubitId(0));
        });
        assert!(out.changed);
        assert_eq!(out.circuit.len(), 1);
        assert_eq!(*only_gate(&out), GateKind::Rz(Param::concrete(0.75)));
        assert_eq!(out.notes, vec!["merged 1 rotation pair(s)"]);
    }

    #[test]
    fn every_additive_family_merges() {
        let rx = run(|b| {
            b.rx(0.25, QubitId(0)).rx(0.5, QubitId(0));
        });
        assert_eq!(*only_gate(&rx), GateKind::Rx(Param::concrete(0.75)));

        let ry = run(|b| {
            b.ry(0.25, QubitId(0)).ry(0.5, QubitId(0));
        });
        assert_eq!(*only_gate(&ry), GateKind::Ry(Param::concrete(0.75)));

        let p = run(|b| {
            b.gate(GateKind::P(Param::concrete(0.25)), [QubitId(0)])
                .gate(GateKind::P(Param::concrete(0.5)), [QubitId(0)]);
        });
        assert_eq!(*only_gate(&p), GateKind::P(Param::concrete(0.75)));
    }

    #[test]
    fn different_axes_do_not_merge() {
        let out = run(|b| {
            b.rz(0.25, QubitId(0)).rx(0.5, QubitId(0));
        });
        assert!(!out.changed);
        assert_eq!(out.circuit.len(), 2);
    }

    #[test]
    fn rotations_on_different_qubits_do_not_merge() {
        let out = run(|b| {
            b.rz(0.25, QubitId(0)).rz(0.5, QubitId(1));
        });
        assert!(!out.changed);
    }

    #[test]
    fn merging_sees_through_gates_on_other_wires() {
        let out = run(|b| {
            b.rz(0.25, QubitId(0)).h(QubitId(1)).rz(0.5, QubitId(0));
        });
        assert!(out.changed);
        assert_eq!(out.circuit.len(), 2, "the H remains");
    }

    #[test]
    fn an_intervening_gate_on_the_same_wire_blocks_merging() {
        let out = run(|b| {
            b.rz(0.25, QubitId(0)).h(QubitId(0)).rz(0.5, QubitId(0));
        });
        assert!(!out.changed);
    }

    #[test]
    fn a_measurement_blocks_merging() {
        let out = run(|b| {
            b.rz(0.25, QubitId(0))
                .measure(QubitId(0), ClbitId(0))
                .rz(0.5, QubitId(0));
        });
        assert!(!out.changed);
    }

    #[test]
    fn symbolic_parameters_are_never_merged() {
        let out = run(|b| {
            b.rz(Param::symbol("theta"), QubitId(0))
                .rz(Param::symbol("phi"), QubitId(0));
        });
        assert!(!out.changed, "Param cannot represent theta + phi");
        assert_eq!(out.circuit.len(), 2);
    }

    #[test]
    fn a_symbolic_and_a_concrete_parameter_do_not_merge() {
        let out = run(|b| {
            b.rz(Param::symbol("theta"), QubitId(0)).rz(0.5, QubitId(0));
        });
        assert!(!out.changed);
    }

    #[test]
    fn runs_of_rotations_merge_to_a_fixed_point() {
        let out = run(|b| {
            b.rz(0.1, QubitId(0))
                .rz(0.2, QubitId(0))
                .rz(0.3, QubitId(0))
                .rz(0.4, QubitId(0));
        });
        assert_eq!(out.circuit.len(), 1);
        let GateKind::Rz(p) = only_gate(&out) else {
            panic!("expected Rz");
        };
        assert!((p.as_concrete().unwrap().radians() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn opposite_rotations_merge_to_zero_for_canonicalize_to_remove() {
        // This pass produces Rz(0); removing it is Canonicalize's job, which
        // is why the default pipeline runs canonicalize again afterwards.
        let out = run(|b| {
            b.rz(0.5, QubitId(0)).rz(-0.5, QubitId(0));
        });
        assert_eq!(*only_gate(&out), GateKind::Rz(Param::concrete(0.0)));
    }

    #[test]
    fn u_gates_are_not_fused() {
        let u = GateKind::U {
            theta: Param::concrete(0.1),
            phi: Param::concrete(0.2),
            lambda: Param::concrete(0.3),
        };
        let out = run(|b| {
            b.gate(u.clone(), [QubitId(0)])
                .gate(u.clone(), [QubitId(0)]);
        });
        assert!(
            !out.changed,
            "arbitrary unitary synthesis is not implemented"
        );
    }

    #[test]
    fn opaque_gates_are_not_fused() {
        let opaque = GateKind::Opaque {
            name: "mystery".into(),
            params: vec![Param::concrete(0.5)],
        };
        let out = run(|b| {
            b.gate(opaque.clone(), [QubitId(0)])
                .gate(opaque.clone(), [QubitId(0)]);
        });
        assert!(!out.changed);
    }

    #[test]
    fn register_widths_survive_merging() {
        let out = run(|b| {
            b.rz(0.25, QubitId(0)).rz(0.5, QubitId(0));
        });
        assert_eq!(out.circuit.num_qubits(), 3);
        assert_eq!(out.circuit.num_clbits(), 1);
    }
}
