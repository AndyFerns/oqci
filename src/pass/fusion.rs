//! Single-qubit gate fusion (`final-deliverables-spec.md` §8.3).
//!
//! # Policy
//!
//! §8.3 requires the fusion policy to be documented. It is:
//!
//! - **Family.** Runs of single-qubit gates on one wire. Two-qubit blocks are
//!   not fused; nothing here synthesizes a multi-qubit unitary.
//! - **What a run is.** A maximal sequence of single-qubit gates on the same
//!   qubit with nothing else touching that qubit in between. Gates on other
//!   wires may interleave in program order: they act on disjoint qubits and
//!   commute with the run.
//! - **What ends a run.** Any multi-qubit gate on the qubit, a measurement, a
//!   reset (both are barriers: §33.14 forbids optimizing across them without
//!   proof), a gate with a symbolic parameter (its matrix is unknown until
//!   binding), and an opaque gate (its semantics are not OQCI's to define).
//! - **Parameter composition.** The run's matrices are multiplied in order,
//!   and the product is re-expressed exactly — up to one global phase, which
//!   is unobservable (see the invariant on
//!   [`Exactness::UpToGlobalPhase`](crate::lowering::Exactness::UpToGlobalPhase))
//!   — as one of:
//!   - nothing, when the product is the identity;
//!   - `rz(λ)`, when the product is diagonal;
//!   - `U(θ, φ, λ)` otherwise, in the OpenQASM 3 convention documented on
//!     [`GateKind::U`], by the closed-form ZYZ Euler decomposition.
//!
//!   "The identity" and "diagonal" are decided within a fixed tolerance of
//!   1e-12 on matrix entries, far below anything a state-vector comparison
//!   (1e-9) or a device could resolve. That is the only approximation, and
//!   the in-compiler checker (`crate::verify`) and `tests/pass_equivalence.rs`
//!   both re-check every fusion against an independent model.
//! - **When a run is replaced.** Only runs of two or more gates, and only
//!   when the replacement is cheaper:
//!   - without a target, a run of two or more gates always becomes at most
//!     one, so operation count strictly falls;
//!   - with a target, the replacement must expand to *strictly fewer* native
//!     operations under the target's own decomposition rules than the run it
//!     replaces. Fusing `h; t` into a `U` would be worse on an `{rz, sx}`
//!     device (a `U` costs five native operations, `h; t` four), so it is
//!     left alone; `h; s; h` (seven) becomes one `U` (five).
//!
//! This is **not** arbitrary unitary synthesis, and it does not claim to find
//! an optimal single-qubit sequence for any basis: it fuses, and lets target
//! lowering's existing rules expand the result.

use std::collections::BTreeMap;
use std::f64::consts::PI;

use num_complex::Complex64 as C;

use crate::ir::{Circuit, GateKind, Instruction, Param};
use crate::lowering::RuleSet;
use crate::pass::{Pass, PassContext, PassError, PassOutput, rebuild};

/// Entry tolerance for "this product is the identity / diagonal".
const TOLERANCE: f64 = 1e-12;

type Matrix = [[C; 2]; 2];

/// Fuses runs of single-qubit gates. See the [module docs](self).
pub struct SingleQubitFusion;

impl Pass for SingleQubitFusion {
    fn id(&self) -> &'static str {
        "single-qubit-fusion"
    }

    fn description(&self) -> &'static str {
        "fuse runs of single-qubit gates on one wire into one U or rz, when cheaper"
    }

    fn run(&self, circuit: &Circuit, context: &PassContext<'_>) -> Result<PassOutput, PassError> {
        let costs = context
            .profile()
            .and_then(|profile| RuleSet::new(profile).ok().map(|rules| (profile, rules)));

        // Per qubit, the program indices of the run currently open on it.
        let mut open: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        // Program index -> what to emit there instead (None: drop it).
        let mut rewrites: BTreeMap<usize, Option<GateKind>> = BTreeMap::new();
        let mut fused_runs = 0usize;

        let close = |run: Vec<usize>,
                     rewrites: &mut BTreeMap<usize, Option<GateKind>>,
                     fused_runs: &mut usize| {
            if let Some(replacement) = fuse(circuit, &run, costs.as_ref()) {
                let last = *run.last().expect("a closed run is non-empty");
                for index in &run {
                    rewrites.insert(*index, None);
                }
                if let Some(gate) = replacement {
                    rewrites.insert(last, Some(gate));
                }
                *fused_runs += 1;
            }
        };

        for (index, instruction) in circuit.instructions().iter().enumerate() {
            if let Some(qubit) = fusible(instruction) {
                open.entry(qubit).or_default().push(index);
                continue;
            }
            for qubit in instruction.qubits() {
                if let Some(run) = open.remove(&qubit.0) {
                    close(run, &mut rewrites, &mut fused_runs);
                }
            }
        }
        for (_, run) in std::mem::take(&mut open) {
            close(run, &mut rewrites, &mut fused_runs);
        }

        if fused_runs == 0 {
            return Ok(PassOutput::unchanged(circuit.clone()));
        }
        let mut kept = Vec::with_capacity(circuit.len());
        for (index, instruction) in circuit.instructions().iter().enumerate() {
            match rewrites.get(&index) {
                None => kept.push(instruction.clone()),
                Some(None) => {}
                Some(Some(gate)) => kept.push(Instruction::Gate {
                    kind: gate.clone(),
                    qubits: instruction.qubits(),
                }),
            }
        }
        let removed = circuit.len() - kept.len();
        Ok(PassOutput {
            circuit: rebuild(self.id(), circuit, kept)?,
            changed: true,
            notes: vec![format!(
                "fused {fused_runs} single-qubit run(s), {removed} operation(s) fewer"
            )],
        })
    }
}

/// The qubit of a gate that may join a run, or `None` if it ends runs.
fn fusible(instruction: &Instruction) -> Option<u32> {
    let Instruction::Gate { kind, qubits } = instruction else {
        return None;
    };
    if qubits.len() != 1 || matches!(kind, GateKind::Opaque { .. }) || !kind.symbols().is_empty() {
        return None;
    }
    Some(qubits[0].0)
}

/// What to replace a run with — `Some(None)` to delete it, `Some(Some(g))`
/// for one gate — or `None` to leave it alone.
fn fuse(
    circuit: &Circuit,
    run: &[usize],
    costs: Option<&(&crate::target::BasisProfile, RuleSet)>,
) -> Option<Option<GateKind>> {
    if run.len() < 2 {
        return None;
    }
    let gates: Vec<&GateKind> = run
        .iter()
        .map(|index| match &circuit.instructions()[*index] {
            Instruction::Gate { kind, .. } => kind,
            _ => unreachable!("runs hold only gates"),
        })
        .collect();

    // Later gates multiply on the left.
    let mut product = identity();
    for gate in &gates {
        product = multiply(&matrix(gate)?, &product);
    }
    let replacement = synthesize(&product);

    if let Some((profile, rules)) = costs {
        let before: usize = gates
            .iter()
            .map(|g| native_cost(g.mnemonic(), profile, rules))
            .sum();
        let after = replacement
            .as_ref()
            .map_or(0, |g| native_cost(g.mnemonic(), profile, rules));
        if after >= before {
            return None;
        }
    }
    Some(replacement)
}

/// How many native operations `mnemonic` becomes under the target's rules.
fn native_cost(mnemonic: &str, profile: &crate::target::BasisProfile, rules: &RuleSet) -> usize {
    if profile.supports_operation(mnemonic) {
        return 1;
    }
    match rules.rule_for(mnemonic) {
        // The rule set is acyclic (checked by `RuleSet::new`), so this
        // recursion terminates.
        Some(rule) => rule
            .steps
            .iter()
            .map(|step| native_cost(step.op, profile, rules))
            .sum(),
        // Unlowerable either way; count it as itself.
        None => 1,
    }
}

/// The product, as nothing, an `rz`, or a `U`.
fn synthesize(m: &Matrix) -> Option<GateKind> {
    let off_diagonal = m[0][1].norm().max(m[1][0].norm());
    if off_diagonal < TOLERANCE {
        // Diagonal: diag(a, b) = a * diag(1, b/a), and diag(1, e^{iλ}) is
        // rz(λ) up to the global phase e^{iλ/2}.
        let lambda = wrap(m[1][1].arg() - m[0][0].arg());
        if lambda.abs() < TOLERANCE {
            return None;
        }
        return Some(GateKind::Rz(Param::concrete(lambda)));
    }
    // U(θ, φ, λ) = [[c, -e^{iλ} s], [e^{iφ} s, e^{i(φ+λ)} c]] with c, s ≥ 0.
    let theta = 2.0 * m[1][0].norm().atan2(m[0][0].norm());
    let (phi, lambda) = if m[0][0].norm() < TOLERANCE {
        // θ = π: only φ + λ's split is free; take φ = 0, so the phase is
        // fixed by the lower-left entry and λ by the upper-right.
        let phase = m[1][0].arg();
        (0.0, wrap((-m[0][1]).arg() - phase))
    } else {
        // Remove the global phase that makes the top-left entry real.
        let phase = m[0][0].arg();
        (wrap(m[1][0].arg() - phase), wrap((-m[0][1]).arg() - phase))
    };
    Some(GateKind::U {
        theta: Param::concrete(theta),
        phi: Param::concrete(phi),
        lambda: Param::concrete(lambda),
    })
}

/// An angle wrapped into (-π, π].
fn wrap(angle: f64) -> f64 {
    let wrapped = angle.rem_euclid(2.0 * PI);
    if wrapped > PI {
        wrapped - 2.0 * PI
    } else {
        wrapped
    }
}

fn identity() -> Matrix {
    let (one, zero) = (C::new(1.0, 0.0), C::new(0.0, 0.0));
    [[one, zero], [zero, one]]
}

fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [[C::new(0.0, 0.0); 2]; 2];
    for (row, out_row) in out.iter_mut().enumerate() {
        for (col, cell) in out_row.iter_mut().enumerate() {
            *cell = a[row][0] * b[0][col] + a[row][1] * b[1][col];
        }
    }
    out
}

/// A single-qubit gate's matrix, from its definition. `None` for anything
/// that is not a concrete single-qubit gate (never reached for a run).
fn matrix(kind: &GateKind) -> Option<Matrix> {
    use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_4};
    let c = C::new;
    let zero = c(0.0, 0.0);
    let one = c(1.0, 0.0);
    let diag = |lower: C| [[one, zero], [zero, lower]];
    let angle = |p: &Param| p.as_concrete().map(crate::ir::Angle::radians);
    Some(match kind {
        GateKind::I => identity(),
        GateKind::X => [[zero, one], [one, zero]],
        GateKind::Y => [[zero, c(0.0, -1.0)], [c(0.0, 1.0), zero]],
        GateKind::Z => diag(c(-1.0, 0.0)),
        GateKind::H => {
            let h = c(FRAC_1_SQRT_2, 0.0);
            [[h, h], [h, -h]]
        }
        GateKind::S => diag(c(0.0, 1.0)),
        GateKind::Sdg => diag(c(0.0, -1.0)),
        GateKind::T => diag(C::from_polar(1.0, FRAC_PI_4)),
        GateKind::Tdg => diag(C::from_polar(1.0, -FRAC_PI_4)),
        GateKind::SX => [[c(0.5, 0.5), c(0.5, -0.5)], [c(0.5, -0.5), c(0.5, 0.5)]],
        GateKind::SXdg => [[c(0.5, -0.5), c(0.5, 0.5)], [c(0.5, 0.5), c(0.5, -0.5)]],
        GateKind::Rx(p) => {
            let t = angle(p)? / 2.0;
            [
                [c(t.cos(), 0.0), c(0.0, -t.sin())],
                [c(0.0, -t.sin()), c(t.cos(), 0.0)],
            ]
        }
        GateKind::Ry(p) => {
            let t = angle(p)? / 2.0;
            [
                [c(t.cos(), 0.0), c(-t.sin(), 0.0)],
                [c(t.sin(), 0.0), c(t.cos(), 0.0)],
            ]
        }
        GateKind::Rz(p) => {
            let t = angle(p)? / 2.0;
            [
                [C::from_polar(1.0, -t), zero],
                [zero, C::from_polar(1.0, t)],
            ]
        }
        GateKind::P(p) => diag(C::from_polar(1.0, angle(p)?)),
        GateKind::U { theta, phi, lambda } => {
            let t = angle(theta)? / 2.0;
            let (p, l) = (angle(phi)?, angle(lambda)?);
            [
                [c(t.cos(), 0.0), -C::from_polar(t.sin(), l)],
                [C::from_polar(t.sin(), p), C::from_polar(t.cos(), p + l)],
            ]
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CircuitBuilder, ClbitId, QubitId};
    use crate::target::builtin;

    fn run(build: impl FnOnce(&mut CircuitBuilder)) -> PassOutput {
        let mut b = CircuitBuilder::new("f");
        b.alloc_qubits(2);
        b.alloc_clbits(1);
        build(&mut b);
        SingleQubitFusion
            .run(&b.build().unwrap(), &PassContext::none())
            .unwrap()
    }

    fn kinds(out: &PassOutput) -> Vec<String> {
        out.circuit
            .instructions()
            .iter()
            .map(|i| match i {
                Instruction::Gate { kind, .. } => kind.mnemonic().to_string(),
                Instruction::Measure { .. } => "measure".into(),
                Instruction::Reset { .. } => "reset".into(),
            })
            .collect()
    }

    #[test]
    fn a_self_inverse_run_disappears() {
        let out = run(|b| {
            b.h(QubitId(0)).h(QubitId(0));
        });
        assert!(out.changed);
        assert!(out.circuit.is_empty());
    }

    #[test]
    fn a_diagonal_run_becomes_one_rz() {
        let out = run(|b| {
            b.gate(GateKind::S, [QubitId(0)])
                .gate(GateKind::T, [QubitId(0)]);
        });
        assert_eq!(kinds(&out), vec!["rz"]);
    }

    #[test]
    fn a_general_run_becomes_one_u() {
        let out = run(|b| {
            b.h(QubitId(0))
                .gate(GateKind::T, [QubitId(0)])
                .h(QubitId(0));
        });
        assert_eq!(kinds(&out), vec!["u"]);
    }

    #[test]
    fn a_two_qubit_gate_measurement_or_symbol_ends_a_run() {
        let out = run(|b| {
            b.h(QubitId(0))
                .cx(QubitId(0), QubitId(1))
                .h(QubitId(0))
                .measure(QubitId(0), ClbitId(0))
                .h(QubitId(0))
                .rz(Param::symbol("t"), QubitId(0))
                .h(QubitId(0));
        });
        assert!(!out.changed, "no run of two exists: {:?}", kinds(&out));
    }

    #[test]
    fn gates_on_other_wires_do_not_end_a_run() {
        let out = run(|b| {
            b.h(QubitId(0)).x(QubitId(1)).h(QubitId(0));
        });
        assert_eq!(kinds(&out), vec!["x"]);
    }

    #[test]
    fn with_a_target_a_run_is_fused_only_when_it_gets_cheaper() {
        let profile = builtin::linear_nisq(2);
        let cost = crate::target::cost::resolve(profile.cost_model_id()).unwrap();
        let context = PassContext::with_profile(&profile).and_cost_model(&cost);
        let fuse = |build: &dyn Fn(&mut CircuitBuilder)| {
            let mut b = CircuitBuilder::new("f");
            b.alloc_qubits(1);
            build(&mut b);
            SingleQubitFusion
                .run(&b.build().unwrap(), &context)
                .unwrap()
        };
        // h; t costs 3 + 1 = 4 native ops on {rz, sx, x}; a U costs 5.
        assert!(
            !fuse(&|b| {
                b.h(QubitId(0)).gate(GateKind::T, [QubitId(0)]);
            })
            .changed
        );
        // h; s; h costs 7; a U costs 5.
        assert!(
            fuse(&|b| {
                b.h(QubitId(0))
                    .gate(GateKind::S, [QubitId(0)])
                    .h(QubitId(0));
            })
            .changed
        );
    }

    #[test]
    fn wrapping_keeps_angles_in_range() {
        assert!((wrap(3.0 * PI) - PI).abs() < 1e-12);
        assert!((wrap(-3.0 * PI / 2.0) - PI / 2.0).abs() < 1e-12);
    }
}
