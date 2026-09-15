//! Gate cancellation: remove adjacent gate/inverse-gate pairs.
//!
//! `final-deliverables-spec.md` §8.2 asks for "provably safe cancellation for
//! supported inverse gate pairs" and warns: "must not cancel across an
//! intervening operation that changes the relevant wire semantics."
//!
//! # Why this works on the DAG rather than the instruction list
//!
//! Textual adjacency is the wrong test in both directions. In
//! `H q0; H q1; H q0` the two `H q0` gates are *not* adjacent in program order
//! but absolutely do cancel — nothing touches `q0` between them. Conversely
//! two neighbouring instructions may be separated by a measurement on a shared
//! wire and must not cancel.
//!
//! QCO-IR answers both cases directly. Two operations are cancellable when the
//! second is the immediate successor of the first **on every wire the first
//! touches** — which is precisely the statement "nothing intervenes". The
//! measurement barrier needs no special handling either: a collapsing
//! operation is itself a node on the wire, so it breaks the adjacency, and the
//! [`crate::ir::DepKind::Control`] edge it produces can never appear between two
//! unitaries.
//!
//! # The inverse table is deliberately conservative
//!
//! | Gates | Rule |
//! |---|---|
//! | `X Y Z H Cx Cy Cz Swap Ccx` | self-inverse |
//! | `S`↔`Sdg`, `T`↔`Tdg` | mutual inverses |
//! | `Rx Ry Rz P` | inverse iff both parameters are concrete and exactly negate |
//! | `U`, `Opaque`, `I` | never cancelled |
//!
//! `U`'s inverse is `U(-θ, -λ, -φ)` — a real identity, but note the swapped
//! φ/λ. That is exactly the kind of rule that is easy to get subtly wrong and
//! impossible to notice afterwards, so it is omitted until
//! `tests/pass_equivalence.rs` can demonstrate it across random angles.
//! `Opaque` has no knowable inverse at all: OQCI does not know what the gate
//! does, so it cannot know that doing it twice does nothing. `I` is left to
//! [`crate::pass::Canonicalize`].
//!
//! Rotation cancellation requires angles that *exactly* negate. `θ` and
//! `2π − θ` describe the same rotation but are not recognised, and no epsilon
//! is applied — deciding how much floating-point error is tolerable in
//! someone else's circuit is not this pass's call.

use std::collections::HashSet;

use crate::ir::{Circuit, GateKind, Instruction, Param};
use crate::pass::adjacency::{matched_pair, unanimous_successors};
use crate::pass::{Pass, PassError, PassOutput, rebuild};

/// Cancels adjacent inverse gate pairs. See the [module docs](self).
pub struct GateCancellation;

impl Pass for GateCancellation {
    fn id(&self) -> &'static str {
        "gate-cancellation"
    }

    fn description(&self) -> &'static str {
        "remove adjacent gate/inverse pairs (H;H, S;Sdg, Rz(θ);Rz(-θ), …)"
    }

    fn run(&self, circuit: &Circuit) -> Result<PassOutput, PassError> {
        let mut current = circuit.clone();
        let mut cancelled = 0usize;

        // Removing one pair can expose another (`H; X; X; H` collapses
        // entirely), so iterate to a fixed point. This terminates: every
        // round removes at least two instructions.
        loop {
            let doomed =
                find_cancellable(&current).map_err(|e| PassError::from_ir(self.id(), e))?;
            if doomed.is_empty() {
                break;
            }
            cancelled += doomed.len() / 2;

            let kept: Vec<Instruction> = current
                .instructions()
                .iter()
                .enumerate()
                .filter(|(i, _)| !doomed.contains(i))
                .map(|(_, inst)| inst.clone())
                .collect();
            current = rebuild(self.id(), &current, kept)?;
        }

        if cancelled == 0 {
            return Ok(PassOutput::unchanged(circuit.clone()));
        }
        Ok(PassOutput {
            circuit: current,
            changed: true,
            notes: vec![format!("cancelled {cancelled} inverse pair(s)")],
        })
    }
}

/// Returns the program indices to delete: every member of every cancellable
/// pair, chosen greedily in ascending order so overlapping candidates (as in
/// `X;X;X;X`) are matched consistently rather than double-counted.
fn find_cancellable(circuit: &Circuit) -> Result<HashSet<usize>, crate::ir::IrError> {
    let successors = unanimous_successors(circuit)?;
    let mut doomed = HashSet::new();

    for index in 0..circuit.len() {
        if doomed.contains(&index) {
            continue;
        }
        let Some(&partner) = successors.get(&index) else {
            continue;
        };
        if doomed.contains(&partner) {
            continue;
        }
        // `matched_pair` enforces identical operand lists in order, so a
        // controlled gate's control/target roles must line up too.
        let Some((first, second, _)) = matched_pair(circuit, index, partner) else {
            continue;
        };
        if !are_inverse(first, second) {
            continue;
        }

        doomed.insert(index);
        doomed.insert(partner);
    }

    Ok(doomed)
}

/// Whether applying `a` then `b` is provably the identity.
fn are_inverse(a: &GateKind, b: &GateKind) -> bool {
    use GateKind::{Ccx, Cx, Cy, Cz, H, P, Rx, Ry, Rz, S, Sdg, Swap, T, Tdg, X, Y, Z};
    match (a, b) {
        // Involutions: applying twice is the identity.
        (X, X)
        | (Y, Y)
        | (Z, Z)
        | (H, H)
        | (Cx, Cx)
        | (Cy, Cy)
        | (Cz, Cz)
        | (Swap, Swap)
        | (Ccx, Ccx) => true,
        // Adjoint pairs.
        (S, Sdg) | (Sdg, S) | (T, Tdg) | (Tdg, T) => true,
        // Same-axis rotations whose angles exactly negate.
        (Rx(p), Rx(q)) | (Ry(p), Ry(q)) | (Rz(p), Rz(q)) | (P(p), P(q)) => negate_exactly(p, q),
        // `U`, `Opaque` and `I` are deliberately absent — see the module docs.
        _ => false,
    }
}

fn negate_exactly(a: &Param, b: &Param) -> bool {
    match (a.as_concrete(), b.as_concrete()) {
        (Some(x), Some(y)) => x.radians() == -y.radians(),
        _ => false,
    }
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
        GateCancellation.run(&b.build().unwrap()).unwrap()
    }

    #[test]
    fn self_inverse_pairs_cancel() {
        for gate in [GateKind::X, GateKind::Y, GateKind::Z, GateKind::H] {
            let out = run(|b| {
                b.gate(gate.clone(), [QubitId(0)])
                    .gate(gate.clone(), [QubitId(0)]);
            });
            assert!(out.circuit.is_empty(), "{gate:?} should self-cancel");
        }
    }

    #[test]
    fn adjoint_pairs_cancel_in_both_orders() {
        for (a, b) in [
            (GateKind::S, GateKind::Sdg),
            (GateKind::Sdg, GateKind::S),
            (GateKind::T, GateKind::Tdg),
            (GateKind::Tdg, GateKind::T),
        ] {
            let out = run(|c| {
                c.gate(a.clone(), [QubitId(0)])
                    .gate(b.clone(), [QubitId(0)]);
            });
            assert!(out.circuit.is_empty(), "{a:?};{b:?} should cancel");
        }
    }

    #[test]
    fn two_qubit_gates_cancel_when_operands_match() {
        let out = run(|b| {
            b.cx(QubitId(0), QubitId(1)).cx(QubitId(0), QubitId(1));
        });
        assert!(out.circuit.is_empty());
    }

    #[test]
    fn swapped_control_and_target_do_not_cancel() {
        // Cx q0,q1 followed by Cx q1,q0 is not the identity.
        let out = run(|b| {
            b.cx(QubitId(0), QubitId(1)).cx(QubitId(1), QubitId(0));
        });
        assert!(!out.changed);
        assert_eq!(out.circuit.len(), 2);
    }

    #[test]
    fn cancellation_sees_through_gates_on_other_wires() {
        // The intervening H is on a different qubit, so the pair is still
        // adjacent on q0 — this is the case textual adjacency would miss.
        let out = run(|b| {
            b.h(QubitId(0)).h(QubitId(1)).h(QubitId(0));
        });
        assert_eq!(out.circuit.len(), 1);
        assert_eq!(
            out.circuit.instructions()[0],
            Instruction::Gate {
                kind: GateKind::H,
                qubits: vec![QubitId(1)]
            }
        );
    }

    #[test]
    fn an_intervening_gate_on_a_shared_wire_blocks_cancellation() {
        let out = run(|b| {
            b.h(QubitId(0)).x(QubitId(0)).h(QubitId(0));
        });
        assert!(!out.changed);
        assert_eq!(out.circuit.len(), 3);
    }

    #[test]
    fn a_measurement_blocks_cancellation() {
        // The collapse between them is exactly what must not be optimized
        // across.
        let out = run(|b| {
            b.x(QubitId(0))
                .measure(QubitId(0), ClbitId(0))
                .x(QubitId(0));
        });
        assert!(!out.changed);
        assert_eq!(out.circuit.len(), 3);
    }

    #[test]
    fn a_reset_blocks_cancellation() {
        let out = run(|b| {
            b.x(QubitId(0)).reset(QubitId(0)).x(QubitId(0));
        });
        assert!(!out.changed);
        assert_eq!(out.circuit.len(), 3);
    }

    #[test]
    fn a_two_qubit_gate_sharing_one_wire_blocks_cancellation() {
        let out = run(|b| {
            b.h(QubitId(0)).cx(QubitId(0), QubitId(1)).h(QubitId(0));
        });
        assert!(!out.changed);
    }

    #[test]
    fn rotations_cancel_only_when_angles_exactly_negate() {
        let out = run(|b| {
            b.rz(0.5, QubitId(0)).rz(-0.5, QubitId(0));
        });
        assert!(out.circuit.is_empty());

        let out = run(|b| {
            b.rz(0.5, QubitId(0)).rz(0.5, QubitId(0));
        });
        assert!(
            !out.changed,
            "same-sign rotations are merged, not cancelled"
        );
    }

    #[test]
    fn symbolic_rotations_never_cancel() {
        let out = run(|b| {
            b.rz(Param::symbol("theta"), QubitId(0))
                .rz(Param::symbol("theta"), QubitId(0));
        });
        assert!(!out.changed, "OQCI cannot know theta == -theta");
    }

    #[test]
    fn opaque_gates_never_cancel() {
        let opaque = GateKind::Opaque {
            name: "mystery".into(),
            params: vec![],
        };
        let out = run(|b| {
            b.gate(opaque.clone(), [QubitId(0)])
                .gate(opaque.clone(), [QubitId(0)]);
        });
        assert!(!out.changed, "an unknown gate has no knowable inverse");
    }

    #[test]
    fn u_gates_never_cancel() {
        let u = GateKind::U {
            theta: Param::concrete(0.1),
            phi: Param::concrete(0.2),
            lambda: Param::concrete(0.3),
        };
        let inverse = GateKind::U {
            theta: Param::concrete(-0.1),
            phi: Param::concrete(-0.3),
            lambda: Param::concrete(-0.2),
        };
        let out = run(|b| {
            b.gate(u.clone(), [QubitId(0)])
                .gate(inverse.clone(), [QubitId(0)]);
        });
        assert!(!out.changed, "documented as unverified, so not attempted");
    }

    #[test]
    fn nested_pairs_collapse_to_a_fixed_point() {
        // H;X;X;H — cancelling the inner pair exposes the outer one.
        let out = run(|b| {
            b.h(QubitId(0)).x(QubitId(0)).x(QubitId(0)).h(QubitId(0));
        });
        assert!(out.circuit.is_empty());
        assert_eq!(out.notes, vec!["cancelled 2 inverse pair(s)"]);
    }

    #[test]
    fn odd_runs_leave_exactly_one_gate() {
        let out = run(|b| {
            b.x(QubitId(0)).x(QubitId(0)).x(QubitId(0));
        });
        assert_eq!(out.circuit.len(), 1);
    }

    #[test]
    fn even_runs_cancel_completely() {
        let out = run(|b| {
            b.x(QubitId(0)).x(QubitId(0)).x(QubitId(0)).x(QubitId(0));
        });
        assert!(out.circuit.is_empty());
    }

    #[test]
    fn register_widths_survive_cancellation() {
        let out = run(|b| {
            b.h(QubitId(0)).h(QubitId(0));
        });
        assert_eq!(out.circuit.num_qubits(), 3);
        assert_eq!(out.circuit.num_clbits(), 1);
    }
}
