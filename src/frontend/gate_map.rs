//! The shared gate-name → [`GateKind`] mapping.
//!
//! OpenQASM 3's `stdgates.inc` names and Qiskit's `Instruction.name` strings
//! coincide across the whole registered set, so both frontends resolve gates
//! through this one table. Keeping it in a single place is what makes the two
//! frontends agree by construction rather than by coincidence — see
//! `docs/gate_mapping.md`, which is generated from this same list.
//!
//! Names outside the table are **not** errors: they become
//! [`GateKind::Opaque`], the escape hatch mandated by Stage A §4 ("`GateKind`
//! remains a closed enum plus one `Opaque` escape hatch"). A frontend must
//! never grow the enum to accommodate a source language.

use std::f64::consts::FRAC_PI_2;

use crate::frontend::error::FrontendError;
use crate::ir::{GateKind, Param};

/// Resolves a gate mnemonic and its parameters to a [`GateKind`].
///
/// Names are matched case-insensitively (OpenQASM 3 spells the built-in
/// controlled-X as `CX`, Qiskit as `cx`). Unrecognised names produce
/// [`GateKind::Opaque`] carrying the original name and parameters.
///
/// ```
/// use oqci::frontend::map_gate;
/// use oqci::ir::{GateKind, Param};
///
/// assert_eq!(map_gate("h", vec![]).unwrap(), GateKind::H);
/// assert_eq!(
///     map_gate("rz", vec![Param::concrete(0.5)]).unwrap(),
///     GateKind::Rz(Param::concrete(0.5))
/// );
/// // Unknown names fall through to the opaque escape hatch.
/// assert!(matches!(
///     map_gate("iswap", vec![]).unwrap(),
///     GateKind::Opaque { .. }
/// ));
/// ```
///
/// # Errors
///
/// Returns [`FrontendError::ParamArity`] if a registered gate is given the
/// wrong number of parameters (e.g. `rz` with two angles).
pub fn map_gate(name: &str, params: Vec<Param>) -> Result<GateKind, FrontendError> {
    let lowered = name.to_ascii_lowercase();

    // Registered gates taking no parameters.
    let nullary = match lowered.as_str() {
        "id" | "i" => Some(GateKind::I),
        "x" => Some(GateKind::X),
        "y" => Some(GateKind::Y),
        "z" => Some(GateKind::Z),
        "h" => Some(GateKind::H),
        "s" => Some(GateKind::S),
        "sdg" => Some(GateKind::Sdg),
        "t" => Some(GateKind::T),
        "tdg" => Some(GateKind::Tdg),
        "cx" | "cnot" => Some(GateKind::Cx),
        "cy" => Some(GateKind::Cy),
        "cz" => Some(GateKind::Cz),
        "swap" => Some(GateKind::Swap),
        "ccx" | "toffoli" => Some(GateKind::Ccx),
        _ => None,
    };
    if let Some(kind) = nullary {
        check_arity(&lowered, 0, &params)?;
        return Ok(kind);
    }

    // Registered gates taking parameters.
    match lowered.as_str() {
        "rx" | "ry" | "rz" | "p" | "u1" | "phase" => {
            check_arity(&lowered, 1, &params)?;
            let angle = params.into_iter().next().expect("arity checked");
            Ok(match lowered.as_str() {
                "rx" => GateKind::Rx(angle),
                "ry" => GateKind::Ry(angle),
                "rz" => GateKind::Rz(angle),
                // `u1(λ)` and `phase(λ)` are spellings of the phase gate
                // `diag(1, e^{iλ})`, which is exactly `GateKind::P`.
                _ => GateKind::P(angle),
            })
        }
        "u" | "u3" => {
            check_arity(&lowered, 3, &params)?;
            let mut it = params.into_iter();
            Ok(GateKind::U {
                theta: it.next().expect("arity checked"),
                phi: it.next().expect("arity checked"),
                lambda: it.next().expect("arity checked"),
            })
        }
        // `u2(φ, λ) == U(π/2, φ, λ)`.
        "u2" => {
            check_arity(&lowered, 2, &params)?;
            let mut it = params.into_iter();
            Ok(GateKind::U {
                theta: Param::concrete(FRAC_PI_2),
                phi: it.next().expect("arity checked"),
                lambda: it.next().expect("arity checked"),
            })
        }
        _ => Ok(GateKind::Opaque {
            name: lowered,
            params,
        }),
    }
}

fn check_arity(gate: &str, expected: usize, params: &[Param]) -> Result<(), FrontendError> {
    if params.len() == expected {
        Ok(())
    } else {
        Err(FrontendError::ParamArity {
            gate: gate.to_string(),
            expected,
            found: params.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nullary_gates_map() {
        for (name, expected) in [
            ("id", GateKind::I),
            ("x", GateKind::X),
            ("h", GateKind::H),
            ("sdg", GateKind::Sdg),
            ("tdg", GateKind::Tdg),
            ("cx", GateKind::Cx),
            ("cy", GateKind::Cy),
            ("cz", GateKind::Cz),
            ("swap", GateKind::Swap),
            ("ccx", GateKind::Ccx),
        ] {
            assert_eq!(map_gate(name, vec![]).unwrap(), expected, "{name}");
        }
    }

    #[test]
    fn openqasm_uppercase_cx_maps() {
        assert_eq!(map_gate("CX", vec![]).unwrap(), GateKind::Cx);
    }

    #[test]
    fn rotations_map() {
        let p = Param::concrete(0.5);
        assert_eq!(
            map_gate("rx", vec![p.clone()]).unwrap(),
            GateKind::Rx(p.clone())
        );
        assert_eq!(
            map_gate("ry", vec![p.clone()]).unwrap(),
            GateKind::Ry(p.clone())
        );
        assert_eq!(
            map_gate("rz", vec![p.clone()]).unwrap(),
            GateKind::Rz(p.clone())
        );
    }

    #[test]
    fn phase_spellings_all_map_to_p() {
        let p = Param::concrete(0.5);
        for name in ["p", "u1", "phase"] {
            assert_eq!(
                map_gate(name, vec![p.clone()]).unwrap(),
                GateKind::P(p.clone()),
                "{name}"
            );
        }
    }

    #[test]
    fn u_and_u3_map_to_euler_u() {
        let expected = GateKind::U {
            theta: Param::concrete(0.1),
            phi: Param::concrete(0.2),
            lambda: Param::concrete(0.3),
        };
        for name in ["u", "u3"] {
            let params = vec![
                Param::concrete(0.1),
                Param::concrete(0.2),
                Param::concrete(0.3),
            ];
            assert_eq!(map_gate(name, params).unwrap(), expected, "{name}");
        }
    }

    #[test]
    fn u2_fills_theta_with_half_pi() {
        let kind = map_gate("u2", vec![Param::concrete(0.2), Param::concrete(0.3)]).unwrap();
        assert_eq!(
            kind,
            GateKind::U {
                theta: Param::concrete(FRAC_PI_2),
                phi: Param::concrete(0.2),
                lambda: Param::concrete(0.3),
            }
        );
    }

    #[test]
    fn symbolic_parameters_pass_through() {
        assert_eq!(
            map_gate("rz", vec![Param::symbol("theta")]).unwrap(),
            GateKind::Rz(Param::symbol("theta"))
        );
    }

    #[test]
    fn unknown_name_becomes_opaque() {
        let kind = map_gate("iswap", vec![Param::concrete(1.0)]).unwrap();
        assert_eq!(
            kind,
            GateKind::Opaque {
                name: "iswap".into(),
                params: vec![Param::concrete(1.0)]
            }
        );
    }

    #[test]
    fn wrong_parameter_count_is_rejected() {
        assert_eq!(
            map_gate("rz", vec![]),
            Err(FrontendError::ParamArity {
                gate: "rz".into(),
                expected: 1,
                found: 0
            })
        );
        assert_eq!(
            map_gate("h", vec![Param::concrete(0.0)]),
            Err(FrontendError::ParamArity {
                gate: "h".into(),
                expected: 0,
                found: 1
            })
        );
    }
}
