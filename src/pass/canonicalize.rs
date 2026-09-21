//! Canonicalization: remove operations that are exactly the identity.
//!
//! `final-deliverables-spec.md` §8.1 asks canonicalization to "normalize
//! equivalent structural representations before deeper optimization" —
//! its job is to expose opportunities for the passes that follow, not to
//! optimize on its own account.
//!
//! # What it removes, and why only these
//!
//! - [`GateKind::I`] — the identity matrix, literally.
//! - `Rx`/`Ry`/`Rz`/`P` with a **concrete** parameter of exactly `0.0` — each
//!   is the identity matrix at zero, with no global-phase caveat.
//!
//! Nothing else. It is tempting to add rewrites like `P(π) → Z` or
//! `U(0,0,λ) → P(λ)`, but those equivalences hold only up to a global phase
//! that this IR does not track (there is no `gphase` operation), so applying
//! them would quietly change what a controlled version of the circuit means.
//! §33.5 forbids silently expanding the gate set; the same caution applies to
//! silently rewriting between gates. Such rules can be added later — but only
//! with the phase question answered and `tests/pass_equivalence.rs` extended
//! to prove them.
//!
//! The zero-angle rule requires **exact** `0.0`, not "close to zero". An
//! epsilon would mean deciding how much error is acceptable in someone else's
//! circuit, which is not this pass's call to make.

use crate::ir::{Circuit, GateKind, Instruction, Param};
use crate::pass::{Pass, PassContext, PassError, PassOutput, rebuild};

/// Removes provably-identity operations. See the [module docs](self).
pub struct Canonicalize;

impl Pass for Canonicalize {
    fn id(&self) -> &'static str {
        "canonicalize"
    }

    fn description(&self) -> &'static str {
        "remove identity gates and exact zero-angle rotations"
    }

    fn run(&self, circuit: &Circuit, _context: &PassContext<'_>) -> Result<PassOutput, PassError> {
        let kept: Vec<Instruction> = circuit
            .instructions()
            .iter()
            .filter(|inst| !is_identity(inst))
            .cloned()
            .collect();

        let removed = circuit.len() - kept.len();
        if removed == 0 {
            return Ok(PassOutput::unchanged(circuit.clone()));
        }

        Ok(PassOutput {
            circuit: rebuild(self.id(), circuit, kept)?,
            changed: true,
            notes: vec![format!("removed {removed} identity operation(s)")],
        })
    }
}

/// `true` if this instruction provably does nothing at all.
fn is_identity(inst: &Instruction) -> bool {
    let Instruction::Gate { kind, .. } = inst else {
        // Measurement and reset always do something.
        return false;
    };

    match kind {
        GateKind::I => true,
        GateKind::Rx(p) | GateKind::Ry(p) | GateKind::Rz(p) | GateKind::P(p) => is_exactly_zero(p),
        _ => false,
    }
}

/// Exactly zero — a symbolic parameter is never assumed to be anything.
fn is_exactly_zero(param: &Param) -> bool {
    param
        .as_concrete()
        .is_some_and(|angle| angle.radians() == 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CircuitBuilder, QubitId};

    fn run(build: impl FnOnce(&mut CircuitBuilder)) -> PassOutput {
        let mut b = CircuitBuilder::new("c");
        b.alloc_qubits(2);
        b.alloc_clbits(1);
        build(&mut b);
        Canonicalize
            .run(&b.build().unwrap(), &PassContext::none())
            .unwrap()
    }

    #[test]
    fn identity_gates_are_removed() {
        let out = run(|b| {
            b.h(QubitId(0))
                .gate(GateKind::I, [QubitId(0)])
                .x(QubitId(0));
        });
        assert!(out.changed);
        assert_eq!(out.circuit.len(), 2);
        assert_eq!(out.notes, vec!["removed 1 identity operation(s)"]);
    }

    #[test]
    fn exact_zero_rotations_are_removed() {
        let out = run(|b| {
            b.rx(0.0, QubitId(0))
                .ry(0.0, QubitId(0))
                .rz(0.0, QubitId(0))
                .gate(GateKind::P(Param::concrete(0.0)), [QubitId(0)]);
        });
        assert!(out.changed);
        assert!(out.circuit.is_empty());
    }

    #[test]
    fn nonzero_rotations_are_kept() {
        let out = run(|b| {
            b.rz(0.5, QubitId(0));
        });
        assert!(!out.changed);
        assert_eq!(out.circuit.len(), 1);
    }

    #[test]
    fn near_zero_is_not_treated_as_zero() {
        // Deliberate: this pass does not decide what counts as "close enough".
        let out = run(|b| {
            b.rz(f64::EPSILON, QubitId(0));
        });
        assert!(!out.changed);
        assert_eq!(out.circuit.len(), 1);
    }

    #[test]
    fn negative_zero_is_still_zero() {
        let out = run(|b| {
            b.rz(-0.0, QubitId(0));
        });
        assert!(out.changed, "-0.0 == 0.0 in IEEE-754");
        assert!(out.circuit.is_empty());
    }

    #[test]
    fn symbolic_parameters_are_never_assumed_zero() {
        let out = run(|b| {
            b.rz(Param::symbol("theta"), QubitId(0));
        });
        assert!(!out.changed);
        assert_eq!(out.circuit.parameters(), vec!["theta".to_string()]);
    }

    #[test]
    fn measurement_and_reset_are_never_removed() {
        let out = run(|b| {
            b.measure(QubitId(0), crate::ir::ClbitId(0))
                .reset(QubitId(1));
        });
        assert!(!out.changed);
        assert_eq!(out.circuit.len(), 2);
    }

    #[test]
    fn other_gates_are_left_alone_even_when_equivalent_up_to_phase() {
        // P(pi) and Z differ by a global phase; this pass does not rewrite it.
        let out = run(|b| {
            b.gate(
                GateKind::P(Param::concrete(std::f64::consts::PI)),
                [QubitId(0)],
            );
        });
        assert!(!out.changed);
    }

    #[test]
    fn register_widths_survive_removal() {
        let out = run(|b| {
            b.gate(GateKind::I, [QubitId(0)]);
        });
        assert_eq!(out.circuit.num_qubits(), 2);
        assert_eq!(out.circuit.num_clbits(), 1);
    }
}
