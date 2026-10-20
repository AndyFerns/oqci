//! The in-compiler semantic checker, checked from outside.
//!
//! Three things are established here:
//!
//! 1. **Agreement.** On generated circuit pairs, the checker's verdict
//!    matches the test-only oracle in `support::statevector` at every rung
//!    of its strength ladder: operator-equal, equal only from `|0…0⟩`, or
//!    neither.
//! 2. **It catches what it exists to catch.** A deliberately wrong extension
//!    pass makes compilation fail with `SemanticMismatch` and produce no
//!    executable; in report-only mode it is recorded instead.
//! 3. **It costs nothing when off,** and every shipped pass verifies on the
//!    examples.

mod support;

use std::sync::Arc;

use num_complex::Complex64;
use proptest::prelude::*;

use oqci::compile::{CompileError, CompilerConfig, compile};
use oqci::extension::{ExtensionOrigin, PassPlacement, Registry};
use oqci::ir::{Circuit, CircuitBuilder, GateKind, Instruction, Param, QubitId};
use oqci::pass::{Pass, PassContext, PassError, PassOutput};
use oqci::verify::{OnMismatch, Strength, Verdict, VerifyConfig, check_pass};
use support::statevector::{same_state_up_to_global_phase, simulate, simulate_from_basis_state};

const ANGLES: &[f64] = &[
    0.0,
    0.3,
    -0.3,
    std::f64::consts::FRAC_PI_2,
    std::f64::consts::PI,
];

fn gate() -> impl Strategy<Value = (GateKind, usize)> {
    let angle = prop::sample::select(ANGLES).prop_map(Param::concrete);
    prop_oneof![
        prop::sample::select(vec![
            GateKind::X,
            GateKind::Y,
            GateKind::Z,
            GateKind::H,
            GateKind::S,
            GateKind::Sdg,
            GateKind::T,
            GateKind::SX,
        ])
        .prop_map(|k| (k, 1)),
        (0usize..4, angle.clone()).prop_map(|(w, p)| {
            (
                match w {
                    0 => GateKind::Rx(p),
                    1 => GateKind::Ry(p),
                    2 => GateKind::Rz(p),
                    _ => GateKind::P(p),
                },
                1,
            )
        }),
        (angle.clone(), angle.clone(), angle).prop_map(|(t, p, l)| (
            GateKind::U {
                theta: t,
                phi: p,
                lambda: l
            },
            1
        )),
        prop::sample::select(vec![
            GateKind::Cx,
            GateKind::Cz,
            GateKind::Swap,
            GateKind::Cy
        ])
        .prop_map(|k| (k, 2)),
        Just((GateKind::Ccx, 3)),
    ]
}

/// A unitary circuit over three qubits.
fn circuit() -> impl Strategy<Value = Circuit> {
    prop::collection::vec((gate(), 0usize..3), 0..10).prop_map(|specs| {
        let mut b = CircuitBuilder::new("g");
        b.alloc_qubits(3);
        for ((kind, arity), first) in specs {
            let qubits: Vec<QubitId> = (0..arity)
                .map(|i| QubitId(((first + i) % 3) as u32))
                .collect();
            b.gate(kind, qubits);
        }
        b.build().unwrap()
    })
}

/// A second circuit related to the first: identical, with one gate dropped,
/// or with one gate prepended — so all three rungs of the ladder occur.
fn pair() -> impl Strategy<Value = (Circuit, Circuit)> {
    (circuit(), 0usize..3, 0usize..10, gate()).prop_map(|(a, how, at, (extra, arity))| {
        let mut b = CircuitBuilder::new("g");
        b.alloc_qubits(3);
        let instructions = a.instructions();
        if how == 2 && arity == 1 {
            b.gate(extra, [QubitId(0)]);
        }
        for (index, instruction) in instructions.iter().enumerate() {
            if how == 1 && !instructions.is_empty() && index == at % instructions.len() {
                continue;
            }
            if let Instruction::Gate { kind, qubits } = instruction {
                b.gate(kind.clone(), qubits.clone());
            }
        }
        (a, b.build().unwrap())
    })
}

fn oracle_operator_equal(a: &Circuit, b: &Circuit) -> bool {
    let n = a.num_qubits();
    let mut trace = Complex64::new(0.0, 0.0);
    for basis in 0..1u32 << n {
        let set: Vec<u32> = (0..n).filter(|q| basis & (1 << q) != 0).collect();
        let sa = simulate_from_basis_state(a, &set, 8);
        let sb = simulate_from_basis_state(b, &set, 8);
        trace += sa
            .iter()
            .zip(&sb)
            .map(|(x, y)| x.conj() * y)
            .sum::<Complex64>();
    }
    (trace.norm() / f64::from(1u32 << n) - 1.0).abs() < 1e-9
}

fn enabled() -> VerifyConfig {
    VerifyConfig {
        enabled: true,
        ..VerifyConfig::default()
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, .. ProptestConfig::default() })]

    #[test]
    fn the_checker_agrees_with_the_test_oracle((a, b) in pair()) {
        let verdict = check_pass(&a, &b, &enabled());
        let operator = oracle_operator_equal(&a, &b);
        let state = same_state_up_to_global_phase(&simulate(&a), &simulate(&b));
        match verdict {
            Verdict::Equivalent { strength: Strength::Identical } => prop_assert_eq!(&a, &b),
            Verdict::Equivalent { strength: Strength::Operator } => prop_assert!(operator),
            Verdict::Equivalent { strength: Strength::InitialState } => {
                prop_assert!(!operator && state);
            }
            Verdict::NotEquivalent { .. } => prop_assert!(!operator && !state),
            other => prop_assert!(false, "unexpected verdict {other:?}"),
        }
    }
}

/// A broken pass: negates the first rotation it finds. Plausible-looking,
/// and exactly the class of sign error the rotation-merge history records.
struct SignFlip;

impl Pass for SignFlip {
    fn id(&self) -> &'static str {
        "sign-flip"
    }

    fn description(&self) -> &'static str {
        "deliberately wrong: negates the first rotation"
    }

    fn run(&self, circuit: &Circuit, _: &PassContext<'_>) -> Result<PassOutput, PassError> {
        let mut b = CircuitBuilder::new(circuit.name());
        b.alloc_qubits(circuit.num_qubits());
        b.alloc_clbits(circuit.num_clbits());
        let mut flipped = false;
        for instruction in circuit.instructions() {
            match instruction {
                Instruction::Gate {
                    kind: GateKind::Rz(p),
                    qubits,
                } if !flipped => {
                    let value = p.as_concrete().unwrap().radians();
                    b.gate(GateKind::Rz(Param::concrete(-value)), qubits.clone());
                    flipped = true;
                }
                Instruction::Gate { kind, qubits } => {
                    b.gate(kind.clone(), qubits.clone());
                }
                Instruction::Measure { qubit, target } => {
                    b.measure(*qubit, *target);
                }
                Instruction::Reset { qubit } => {
                    b.reset(*qubit);
                }
            }
        }
        Ok(PassOutput {
            circuit: b.build().map_err(|e| PassError::from_ir("sign-flip", e))?,
            changed: flipped,
            notes: vec![],
        })
    }
}

// `sx` after `rz` turns the equatorial angle into a measurable Z component,
// so negating it changes the counts. (Negating θ in `h; rz(θ); h` would not:
// that is `rx(∓θ)`, whose distribution from |0> is the same either way — and
// the checker rightly calls such a mutation distribution-equivalent.)
const PROGRAM: &str =
    "qubit[2] q; bit[2] c; h q[0]; rz(0.7) q[0]; sx q[0]; cx q[0], q[1]; c = measure q;";

fn with_sign_flip(on_mismatch: OnMismatch) -> CompilerConfig {
    let mut registry = Registry::default();
    registry
        .register_pass(
            ExtensionOrigin::new("mutant", "0"),
            PassPlacement::default(),
            Arc::new(SignFlip),
        )
        .unwrap();
    CompilerConfig {
        backend: Some("simulator-nisq".into()),
        extensions: registry,
        verify: VerifyConfig {
            on_mismatch,
            ..enabled()
        },
        ..CompilerConfig::default()
    }
}

#[test]
fn a_wrong_pass_stops_compilation_before_an_executable_exists() {
    let err = compile(PROGRAM, &with_sign_flip(OnMismatch::Fail)).unwrap_err();
    match err {
        CompileError::SemanticMismatch { transformation, .. } => {
            assert!(
                transformation.starts_with("pass:sign-flip#"),
                "{transformation}"
            );
        }
        other => panic!("expected a semantic mismatch, got {other}"),
    }
}

#[test]
fn report_only_mode_records_the_mismatch_and_continues() {
    let artifacts = compile(PROGRAM, &with_sign_flip(OnMismatch::Report)).unwrap();
    let report = artifacts.verification.unwrap();
    assert_eq!(report.not_equivalent, 1, "{report:?}");
    let failing = report.first_mismatch().unwrap();
    assert!(failing.transformation.starts_with("pass:sign-flip#"));
}

#[test]
fn without_the_mutant_the_same_program_verifies() {
    let config = CompilerConfig {
        backend: Some("simulator-nisq".into()),
        verify: enabled(),
        ..CompilerConfig::default()
    };
    let report = compile(PROGRAM, &config).unwrap().verification.unwrap();
    assert_eq!(report.not_equivalent, 0);
    assert_eq!(report.inconclusive, 0);
    // Every enabled pass and lowering were checked.
    assert!(report.checks.iter().any(|c| c.transformation == "lowering"));
    assert!(
        report
            .checks
            .iter()
            .filter(|c| c.transformation.starts_with("pass:"))
            .count()
            >= 5
    );
    assert_eq!(report.not_checked, vec!["qir-emission".to_string()]);
}

#[test]
fn every_example_verifies_on_every_built_in_backend() {
    let mut checked = 0;
    for entry in std::fs::read_dir("examples").unwrap() {
        let path = entry.unwrap().path();
        let frontend = match path.extension().and_then(|e| e.to_str()) {
            Some("qasm") => oqci::compile::Frontend::OpenQasm3,
            Some("py") => oqci::compile::Frontend::CudaQ,
            _ => continue,
        };
        let source = std::fs::read_to_string(&path).unwrap();
        for backend in oqci::backend::all() {
            let config = CompilerConfig {
                frontend,
                backend: Some(backend.id().to_string()),
                verify: enabled(),
                ..CompilerConfig::default()
            };
            // Some examples are parameterized or too wide for a backend; those
            // fail for their own documented reasons, not verification's.
            let Ok(artifacts) = oqci::compile::compile_named(&source, "example", &config) else {
                continue;
            };
            let report = artifacts.verification.unwrap();
            assert_eq!(
                report.not_equivalent,
                0,
                "{} on {}: {report:?}",
                path.display(),
                backend.id()
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 8,
        "only {checked} example/backend pairs compiled"
    );
}

#[test]
fn verification_is_off_by_default() {
    let artifacts = compile(PROGRAM, &CompilerConfig::default()).unwrap();
    assert!(artifacts.verification.is_none());
    assert!(artifacts.timings.verification.is_none());
}
