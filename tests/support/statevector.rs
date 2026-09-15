//! A tiny dense state-vector simulator — **test infrastructure only**.
//!
//! This exists to answer one question: does an optimization pass change what
//! a circuit computes? It is not a backend, not a product feature, and not a
//! step toward one. The project's non-goals explicitly rule out "a custom
//! quantum simulator replacing established simulator frameworks"; this is a
//! test harness that never leaves `cargo test`, which is why `num-complex`
//! is a dev-dependency.
//!
//! # Why an independent implementation is the point
//!
//! The passes claim things like "`H` is self-inverse" and "`Rz(a); Rz(b)` is
//! `Rz(a+b)`". Checking those claims against the same table the pass consults
//! would be circular. Here the gate semantics are written out as matrices,
//! independently, so a mis-signed angle in a rewrite rule shows up as a
//! diverging state vector.
//!
//! Note what this does *not* require: that these matrices match any
//! particular textbook convention. Every comparison is before-vs-after of the
//! same circuit under the same conventions, so a self-consistent convention
//! is sufficient. What it does require — and what genuinely tests the passes
//! — is that the conventions are self-consistent, since a rewrite is only
//! correct if it holds under the semantics actually used.
//!
//! Qubit 0 is the least significant bit of a basis-state index.

use num_complex::Complex64;

use oqci::ir::{Circuit, GateKind, Instruction, Param};

/// A 2×2 single-qubit operator, row-major.
type Matrix2 = [[Complex64; 2]; 2];

fn c(re: f64, im: f64) -> Complex64 {
    Complex64::new(re, im)
}

/// Simulates a circuit on `|0…0⟩`, returning the final state vector.
///
/// # Panics
///
/// Panics on a measurement, reset, symbolic parameter, or `Opaque` gate —
/// none of which has a deterministic unitary action this harness can model.
/// Callers must only pass unitary, fully-bound circuits.
pub fn simulate(circuit: &Circuit) -> Vec<Complex64> {
    let n = circuit.num_qubits() as usize;
    let dim = 1usize << n;
    let mut state = vec![c(0.0, 0.0); dim];
    state[0] = c(1.0, 0.0);

    for inst in circuit.instructions() {
        let Instruction::Gate { kind, qubits } = inst else {
            panic!("the equivalence harness only handles unitary circuits, got {inst:?}");
        };
        let operands: Vec<usize> = qubits.iter().map(|q| q.index() as usize).collect();
        apply(&mut state, kind, &operands, n);
    }
    state
}

fn apply(state: &mut [Complex64], kind: &GateKind, qubits: &[usize], n: usize) {
    match kind {
        GateKind::Swap => swap_qubits(state, qubits[0], qubits[1], n),
        GateKind::Cx => controlled(state, &qubits[..1], qubits[1], pauli_x(), n),
        GateKind::Cy => controlled(state, &qubits[..1], qubits[1], pauli_y(), n),
        GateKind::Cz => controlled(state, &qubits[..1], qubits[1], pauli_z(), n),
        GateKind::Ccx => controlled(state, &qubits[..2], qubits[2], pauli_x(), n),
        GateKind::Opaque { name, .. } => {
            panic!("opaque gate `{name}` has no known matrix")
        }
        single => controlled(state, &[], qubits[0], single_qubit_matrix(single), n),
    }
}

fn single_qubit_matrix(kind: &GateKind) -> Matrix2 {
    let zero = c(0.0, 0.0);
    let one = c(1.0, 0.0);
    match kind {
        GateKind::I => [[one, zero], [zero, one]],
        GateKind::X => pauli_x(),
        GateKind::Y => pauli_y(),
        GateKind::Z => pauli_z(),
        GateKind::H => {
            let r = c(std::f64::consts::FRAC_1_SQRT_2, 0.0);
            [[r, r], [r, -r]]
        }
        GateKind::S => [[one, zero], [zero, c(0.0, 1.0)]],
        GateKind::Sdg => [[one, zero], [zero, c(0.0, -1.0)]],
        GateKind::T => [[one, zero], [zero, phase(std::f64::consts::FRAC_PI_4)]],
        GateKind::Tdg => [[one, zero], [zero, phase(-std::f64::consts::FRAC_PI_4)]],
        GateKind::Rx(theta) => {
            let (cos, sin) = half_angle(theta);
            [[c(cos, 0.0), c(0.0, -sin)], [c(0.0, -sin), c(cos, 0.0)]]
        }
        GateKind::Ry(theta) => {
            let (cos, sin) = half_angle(theta);
            [[c(cos, 0.0), c(-sin, 0.0)], [c(sin, 0.0), c(cos, 0.0)]]
        }
        GateKind::Rz(theta) => {
            let t = radians(theta);
            [[phase(-t / 2.0), zero], [zero, phase(t / 2.0)]]
        }
        GateKind::P(lambda) => [[one, zero], [zero, phase(radians(lambda))]],
        GateKind::U { theta, phi, lambda } => {
            let (cos, sin) = half_angle(theta);
            let (p, l) = (radians(phi), radians(lambda));
            [
                [c(cos, 0.0), -phase(l) * sin],
                [phase(p) * sin, phase(p + l) * cos],
            ]
        }
        other => panic!("no matrix for {other:?}"),
    }
}

fn pauli_x() -> Matrix2 {
    [[c(0.0, 0.0), c(1.0, 0.0)], [c(1.0, 0.0), c(0.0, 0.0)]]
}

fn pauli_y() -> Matrix2 {
    [[c(0.0, 0.0), c(0.0, -1.0)], [c(0.0, 1.0), c(0.0, 0.0)]]
}

fn pauli_z() -> Matrix2 {
    [[c(1.0, 0.0), c(0.0, 0.0)], [c(0.0, 0.0), c(-1.0, 0.0)]]
}

fn phase(angle: f64) -> Complex64 {
    Complex64::from_polar(1.0, angle)
}

fn radians(param: &Param) -> f64 {
    param
        .as_concrete()
        .expect("the equivalence harness requires fully-bound parameters")
        .radians()
}

fn half_angle(param: &Param) -> (f64, f64) {
    let half = radians(param) / 2.0;
    (half.cos(), half.sin())
}

/// Applies a 2×2 operator to `target`, on the subspace where every control
/// qubit is `|1⟩`. With no controls this is a plain single-qubit gate.
fn controlled(state: &mut [Complex64], controls: &[usize], target: usize, m: Matrix2, n: usize) {
    let dim = 1usize << n;
    let target_bit = 1usize << target;

    for i in 0..dim {
        // Visit each pair once, from its target-is-zero member.
        if i & target_bit != 0 {
            continue;
        }
        if !controls.iter().all(|c| i & (1usize << c) != 0) {
            continue;
        }
        let j = i | target_bit;
        let (a, b) = (state[i], state[j]);
        state[i] = m[0][0] * a + m[0][1] * b;
        state[j] = m[1][0] * a + m[1][1] * b;
    }
}

fn swap_qubits(state: &mut [Complex64], a: usize, b: usize, n: usize) {
    let dim = 1usize << n;
    let (bit_a, bit_b) = (1usize << a, 1usize << b);
    for i in 0..dim {
        // Only swap the |10⟩ ↔ |01⟩ pairs, once each.
        if i & bit_a != 0 && i & bit_b == 0 {
            state.swap(i, i ^ bit_a ^ bit_b);
        }
    }
}

/// Whether two state vectors describe the same physical state, i.e. are equal
/// up to an unobservable global phase.
///
/// Global phase must be quotiented out: `Z` and `P(π)` differ by one, and no
/// experiment can tell them apart. The test is `|⟨a|b⟩| ≈ 1`.
#[must_use]
pub fn same_state_up_to_global_phase(a: &[Complex64], b: &[Complex64]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let overlap: Complex64 = a.iter().zip(b).map(|(x, y)| x.conj() * y).sum();
    (overlap.norm() - 1.0).abs() < 1e-9
}

#[cfg(test)]
mod self_tests {
    use super::*;
    use oqci::ir::{CircuitBuilder, QubitId};

    fn state(build: impl FnOnce(&mut CircuitBuilder)) -> Vec<Complex64> {
        let mut b = CircuitBuilder::new("t");
        b.alloc_qubits(2);
        build(&mut b);
        simulate(&b.build().unwrap())
    }

    #[test]
    fn identity_circuit_stays_in_ground_state() {
        let s = state(|_| {});
        assert!((s[0].norm() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn x_flips_the_first_qubit() {
        let s = state(|b| {
            b.x(QubitId(0));
        });
        assert!((s[1].norm() - 1.0).abs() < 1e-12, "|01> in little-endian");
    }

    #[test]
    fn bell_state_is_evenly_split() {
        let s = state(|b| {
            b.h(QubitId(0)).cx(QubitId(0), QubitId(1));
        });
        assert!((s[0].norm() - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
        assert!((s[3].norm() - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
        assert!(s[1].norm() < 1e-12 && s[2].norm() < 1e-12);
    }

    #[test]
    fn global_phase_is_ignored_by_the_comparison() {
        // Z and P(pi) differ only by a global phase on the |1> branch... on a
        // superposition they are physically identical states.
        let z = state(|b| {
            b.h(QubitId(0)).z(QubitId(0));
        });
        let p = state(|b| {
            b.h(QubitId(0)).gate(
                GateKind::P(Param::concrete(std::f64::consts::PI)),
                [QubitId(0)],
            );
        });
        assert!(same_state_up_to_global_phase(&z, &p));
    }

    #[test]
    fn different_states_are_distinguished() {
        let a = state(|b| {
            b.h(QubitId(0));
        });
        let b_state = state(|b| {
            b.x(QubitId(0));
        });
        assert!(!same_state_up_to_global_phase(&a, &b_state));
    }

    #[test]
    fn swap_exchanges_qubits() {
        let swapped = state(|b| {
            b.x(QubitId(0)).swap(QubitId(0), QubitId(1));
        });
        let direct = state(|b| {
            b.x(QubitId(1));
        });
        assert!(same_state_up_to_global_phase(&swapped, &direct));
    }
}
