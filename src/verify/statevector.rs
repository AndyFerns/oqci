//! The state-vector kernel behind the semantic checker.
//!
//! Every matrix here is written out from its definition — OpenQASM 3's
//! `stdgates.inc` and the convention documented on [`GateKind::U`] — rather
//! than read from the decomposition rules or the pass tables. That is the
//! point: the checker audits those tables, so it must not share them. A
//! mis-signed angle in a rule shows up here as a residual, not as agreement.
//!
//! Qubit `i` is bit `i` of a basis-state index (little-endian). Only this
//! module and its caller depend on that convention.

use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_4};

use num_complex::Complex64 as C;

use crate::ir::{GateKind, Param};

/// A one-qubit matrix, row-major.
type Matrix = [[C; 2]; 2];

/// Why a gate cannot be simulated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Unsimulable {
    /// A parameter is still a symbol.
    Symbolic(String),
    /// An unregistered gate whose semantics OQCI does not define.
    Opaque(String),
}

/// A dense state vector over `n` qubits.
#[derive(Debug, Clone)]
pub(crate) struct State {
    amplitudes: Vec<C>,
}

impl State {
    /// The computational basis state `|index>` over `n` qubits.
    pub(crate) fn basis(n: usize, index: usize) -> State {
        let mut amplitudes = vec![C::new(0.0, 0.0); 1 << n];
        amplitudes[index] = C::new(1.0, 0.0);
        State { amplitudes }
    }

    pub(crate) fn amplitudes(&self) -> &[C] {
        &self.amplitudes
    }

    /// Applies `m` to `target` on every basis state whose `controls` bits are
    /// all set (no controls: unconditionally).
    fn apply(&mut self, controls: &[usize], target: usize, m: &Matrix) {
        let control_mask: usize = controls.iter().map(|c| 1 << c).sum();
        let bit = 1 << target;
        for index in 0..self.amplitudes.len() {
            if index & bit != 0 || index & control_mask != control_mask {
                continue;
            }
            let (a, b) = (self.amplitudes[index], self.amplitudes[index | bit]);
            self.amplitudes[index] = m[0][0] * a + m[0][1] * b;
            self.amplitudes[index | bit] = m[1][0] * a + m[1][1] * b;
        }
    }

    fn swap(&mut self, a: usize, b: usize) {
        let (bit_a, bit_b) = (1 << a, 1 << b);
        for index in 0..self.amplitudes.len() {
            // Visit each exchanged pair once: from the |..1_a..0_b..> side.
            if index & bit_a != 0 && index & bit_b == 0 {
                self.amplitudes.swap(index, index ^ bit_a ^ bit_b);
            }
        }
    }

    /// Applies one gate on the given (compact) qubit indices.
    pub(crate) fn gate(&mut self, kind: &GateKind, qubits: &[usize]) -> Result<(), Unsimulable> {
        match kind {
            GateKind::Cx => self.apply(&qubits[..1], qubits[1], &x()),
            GateKind::Cy => self.apply(&qubits[..1], qubits[1], &y()),
            GateKind::Cz => self.apply(&qubits[..1], qubits[1], &z()),
            GateKind::Ccx => self.apply(&qubits[..2], qubits[2], &x()),
            GateKind::Swap => self.swap(qubits[0], qubits[1]),
            GateKind::Opaque { name, .. } => return Err(Unsimulable::Opaque(name.clone())),
            single => self.apply(&[], qubits[0], &one_qubit(single)?),
        }
        Ok(())
    }
}

fn c(re: f64, im: f64) -> C {
    C::new(re, im)
}

fn phase(theta: f64) -> C {
    C::from_polar(1.0, theta)
}

fn angle(param: &Param) -> Result<f64, Unsimulable> {
    param
        .as_concrete()
        .map(crate::ir::Angle::radians)
        .ok_or_else(|| Unsimulable::Symbolic(param.as_symbol().unwrap_or_default().to_string()))
}

fn x() -> Matrix {
    [[c(0.0, 0.0), c(1.0, 0.0)], [c(1.0, 0.0), c(0.0, 0.0)]]
}

fn y() -> Matrix {
    [[c(0.0, 0.0), c(0.0, -1.0)], [c(0.0, 1.0), c(0.0, 0.0)]]
}

fn z() -> Matrix {
    [[c(1.0, 0.0), c(0.0, 0.0)], [c(0.0, 0.0), c(-1.0, 0.0)]]
}

fn diagonal(lower: C) -> Matrix {
    [[c(1.0, 0.0), c(0.0, 0.0)], [c(0.0, 0.0), lower]]
}

/// The matrix of a one-qubit gate, from its definition.
fn one_qubit(kind: &GateKind) -> Result<Matrix, Unsimulable> {
    let zero = c(0.0, 0.0);
    Ok(match kind {
        GateKind::I => diagonal(c(1.0, 0.0)),
        GateKind::X => x(),
        GateKind::Y => y(),
        GateKind::Z => z(),
        GateKind::H => {
            let h = FRAC_1_SQRT_2;
            [[c(h, 0.0), c(h, 0.0)], [c(h, 0.0), c(-h, 0.0)]]
        }
        GateKind::S => diagonal(c(0.0, 1.0)),
        GateKind::Sdg => diagonal(c(0.0, -1.0)),
        GateKind::T => diagonal(phase(FRAC_PI_4)),
        GateKind::Tdg => diagonal(phase(-FRAC_PI_4)),
        // sqrt(X) = (1/2)[[1+i, 1-i], [1-i, 1+i]], and its adjoint.
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
            [[phase(-t), zero], [zero, phase(t)]]
        }
        GateKind::P(p) => diagonal(phase(angle(p)?)),
        GateKind::U { theta, phi, lambda } => {
            let t = angle(theta)? / 2.0;
            let (p, l) = (angle(phi)?, angle(lambda)?);
            [
                [c(t.cos(), 0.0), -phase(l) * t.sin()],
                [phase(p) * t.sin(), phase(p + l) * t.cos()],
            ]
        }
        GateKind::Opaque { name, .. } => return Err(Unsimulable::Opaque(name.clone())),
        // Multi-qubit gates are handled by `State::gate` before this is
        // reached; arity is guaranteed by IR validation (I4).
        GateKind::Cx | GateKind::Cy | GateKind::Cz | GateKind::Swap | GateKind::Ccx => {
            unreachable!("multi-qubit gates are dispatched before one_qubit")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: C, b: C) -> bool {
        (a - b).norm() < 1e-12
    }

    #[test]
    fn hadamard_makes_an_equal_superposition() {
        let mut state = State::basis(1, 0);
        state.gate(&GateKind::H, &[0]).unwrap();
        assert!(close(state.amplitudes()[0], c(FRAC_1_SQRT_2, 0.0)));
        assert!(close(state.amplitudes()[1], c(FRAC_1_SQRT_2, 0.0)));
    }

    #[test]
    fn cx_flips_the_target_only_when_the_control_is_set() {
        // Qubit 0 is bit 0: |01> (index 1) has the control set.
        let mut state = State::basis(2, 0b01);
        state.gate(&GateKind::Cx, &[0, 1]).unwrap();
        assert!(close(state.amplitudes()[0b11], c(1.0, 0.0)));

        let mut state = State::basis(2, 0b10);
        state.gate(&GateKind::Cx, &[0, 1]).unwrap();
        assert!(close(state.amplitudes()[0b10], c(1.0, 0.0)));
    }

    #[test]
    fn sx_squared_is_x() {
        let mut state = State::basis(1, 0);
        state.gate(&GateKind::SX, &[0]).unwrap();
        state.gate(&GateKind::SX, &[0]).unwrap();
        assert!(close(state.amplitudes()[1], c(1.0, 0.0)));
    }

    #[test]
    fn swap_exchanges_qubits() {
        let mut state = State::basis(3, 0b001);
        state.gate(&GateKind::Swap, &[0, 2]).unwrap();
        assert!(close(state.amplitudes()[0b100], c(1.0, 0.0)));
    }

    #[test]
    fn a_symbolic_parameter_is_unsimulable_not_guessed() {
        let mut state = State::basis(1, 0);
        let err = state
            .gate(&GateKind::Rz(Param::symbol("theta")), &[0])
            .unwrap_err();
        assert_eq!(err, Unsimulable::Symbolic("theta".into()));
    }
}
