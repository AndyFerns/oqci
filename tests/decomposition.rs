//! Numeric verification of the decomposition-rule library.
//!
//! Stage D exit criterion 5 is "decomposition rules are tested". This is that
//! test, and it is the only thing standing between a mistyped angle and a
//! compiler that silently changes what a program computes.
//!
//! # Why operators, not states
//!
//! Every rule is checked as a **matrix**, by simulating it once per
//! computational basis state and comparing the resulting columns. Checking a
//! rule on `|0...0>` alone would be far too weak: a wrong `Swap` or `Cx`
//! decomposition can agree with the real thing on `|00>` and disagree
//! everywhere else.
//!
//! # Why the overlaps are summed before taking a modulus
//!
//! The comparison is `|tr(A* B)| ~ d`, not "every column matches up to
//! phase". Those are different claims, and only the first one is right. If
//! each column were allowed its *own* phase, a rewrite that multiplied basis
//! state `|01>` by `-1` and left the others alone would pass — and that
//! rewrite is wrong, because it changes relative phases and so changes what
//! superpositions do. Summing first forces one shared phase across the whole
//! operator.
//!
//! # Independence
//!
//! `support::statevector` writes out each gate's matrix from scratch rather
//! than consulting the tables the rules are built from, so a mis-signed angle
//! in a rule shows up as a diverging column. The same identities are
//! additionally checked against Qiskit's `quantum_info.Operator` in
//! `python/tests/test_rules.py` — two independent implementations, so an
//! error can only hide by being made identically in both.

mod support;

use num_complex::Complex64;

use oqci::ir::{CircuitBuilder, GateKind, Instruction, Param, QubitId};
use oqci::lowering::rules::{self, Exactness, ParamTransform, RuleError, RuleSetError};
use oqci::lowering::{DecompositionRule, RuleSet};
use oqci::target::{BasisProfileBuilder, Topology, builtin};
use support::statevector::{overlap_trace, simulate};

/// Angles chosen to include the awkward ones: zero, a negative, values that
/// land on axis boundaries, and an irrational-looking one that cannot hide a
/// sign error behind symmetry.
const ANGLES: &[f64] = &[
    0.0,
    0.37,
    -0.91,
    std::f64::consts::FRAC_PI_2,
    std::f64::consts::PI,
    -std::f64::consts::FRAC_PI_4,
    // Deliberately not a named constant: an angle with no symmetry cannot
    // hide a sign error the way pi/2 and its friends can.
    2.7183456,
];

/// Builds the operator a rule's *source* implements, column by column.
fn source_operator(rule: &DecompositionRule, params: &[Param]) -> Vec<Vec<Complex64>> {
    let kind = oqci::frontend::map_gate(rule.source, params.to_vec()).expect("registered gate");
    operator(
        rule.source_arity,
        &[Instruction::Gate {
            kind,
            qubits: (0..rule.source_arity as u32).map(QubitId).collect(),
        }],
    )
}

/// Builds the operator a rule's *expansion* implements.
fn expansion_operator(rule: &DecompositionRule, params: &[Param]) -> Vec<Vec<Complex64>> {
    let operands: Vec<QubitId> = (0..rule.source_arity as u32).map(QubitId).collect();
    let steps = rule.expand(params, &operands).expect("expansion");
    operator(rule.source_arity, &steps)
}

/// Simulates `instructions` from every basis state, returning the columns of
/// the operator they implement.
fn operator(arity: usize, instructions: &[Instruction]) -> Vec<Vec<Complex64>> {
    (0..1usize << arity)
        .map(|basis| {
            let mut b = CircuitBuilder::new("op");
            let qubits = b.alloc_qubits(arity as u32);
            // Prepare |basis> with X gates — exact, and phase-free.
            for (bit, qubit) in qubits.iter().enumerate() {
                if basis & (1 << bit) != 0 {
                    b.x(*qubit);
                }
            }
            for instruction in instructions {
                match instruction {
                    Instruction::Gate { kind, qubits } => {
                        b.gate(kind.clone(), qubits.clone());
                    }
                    other => panic!("a rule emitted a non-gate instruction: {other:?}"),
                }
            }
            simulate(&b.build().expect("valid circuit"))
        })
        .collect()
}

/// Concrete parameters for a rule, drawn from `ANGLES`.
fn params_for(rule: &DecompositionRule, seed: usize) -> Vec<Param> {
    (0..rule.source_params)
        .map(|i| Param::concrete(ANGLES[(seed + i * 3) % ANGLES.len()]))
        .collect()
}

/// `|tr(A* B)|` against the dimension, and whether the two agree exactly.
fn compare(a: &[Vec<Complex64>], b: &[Vec<Complex64>]) -> (f64, bool) {
    let dimension = a.len() as f64;
    let trace = overlap_trace(a, b);
    let exact = a
        .iter()
        .zip(b)
        .all(|(x, y)| x.iter().zip(y).all(|(p, q)| (p - q).norm() < 1e-12));
    (((trace.norm() - dimension) as f64).abs(), exact)
}

// --- Every rule computes what it says ---------------------------------------

#[test]
fn every_builtin_rule_reproduces_its_source_operator() {
    for rule in rules::all_builtin() {
        for seed in 0..ANGLES.len() {
            let params = params_for(rule, seed);
            let source = source_operator(rule, &params);
            let expansion = expansion_operator(rule, &params);
            let (deviation, _) = compare(&source, &expansion);
            assert!(
                deviation < 1e-9,
                "rule `{}` does not implement `{}` (params {params:?}): \
                 |tr(A*B)| off by {deviation:e}",
                rule.id,
                rule.source
            );
        }
    }
}

#[test]
fn every_rule_declares_the_exactness_it_actually_has() {
    // The declared `Exactness` is a claim about the rule, and a claim nobody
    // checks is a comment. Here it is re-derived: if the expansion equals the
    // source entry-for-entry it is `Exact`, and if it only matches up to a
    // shared phase it is `UpToGlobalPhase`. A rule that drifts from its
    // declaration fails the build.
    for rule in rules::all_builtin() {
        let params = params_for(rule, 1);
        let (_, entrywise_equal) = compare(
            &source_operator(rule, &params),
            &expansion_operator(rule, &params),
        );
        let observed = if entrywise_equal {
            Exactness::Exact
        } else {
            Exactness::UpToGlobalPhase
        };
        assert_eq!(
            rule.exactness, observed,
            "rule `{}` declares {:?} but measures {observed:?}",
            rule.id, rule.exactness
        );
    }
}

#[test]
fn a_deliberately_broken_rule_is_caught() {
    // A harness that has never failed is not evidence. This is `cz-to-cx`
    // with one of its conjugating H gates dropped — the exact mistake the
    // real rule exists to avoid — and the check must reject it.
    let broken = DecompositionRule {
        id: "broken-cz",
        source: "cz",
        source_arity: 2,
        source_params: 0,
        steps: &[
            rules::RuleStep {
                op: "h",
                operands: &[1],
                params: &[],
            },
            rules::RuleStep {
                op: "cx",
                operands: &[0, 1],
                params: &[],
            },
        ],
        exactness: Exactness::Exact,
        note: "deliberately wrong",
    };
    let (deviation, _) = compare(
        &source_operator(&broken, &[]),
        &expansion_operator(&broken, &[]),
    );
    assert!(
        deviation > 1e-6,
        "the operator comparison failed to notice a missing H"
    );
}

#[test]
fn a_per_column_comparison_would_have_missed_a_relative_phase() {
    // Justifies summing the overlaps before taking the modulus. This operator
    // is the identity with `|1>` negated: every column matches the identity's
    // up to *its own* phase, yet the operator is Z, which is not the
    // identity. The trace form catches it; a per-column one would not.
    let identity = vec![
        vec![Complex64::new(1.0, 0.0), Complex64::new(0.0, 0.0)],
        vec![Complex64::new(0.0, 0.0), Complex64::new(1.0, 0.0)],
    ];
    let z = vec![
        vec![Complex64::new(1.0, 0.0), Complex64::new(0.0, 0.0)],
        vec![Complex64::new(0.0, 0.0), Complex64::new(-1.0, 0.0)],
    ];

    let per_column_would_pass = identity.iter().zip(&z).all(|(a, b)| {
        (overlap_trace(std::slice::from_ref(a), std::slice::from_ref(b)).norm() - 1.0).abs() < 1e-9
    });
    assert!(per_column_would_pass, "each column does match up to phase");

    let (deviation, _) = compare(&identity, &z);
    assert!(
        deviation > 1e-6,
        "but the operators differ, and the trace form says so"
    );
}

// --- The symbolic-parameter contract ----------------------------------------

#[test]
fn a_transparent_rule_passes_a_symbol_through_verbatim() {
    // `p-to-rz` only re-targets its angle, so a symbolic circuit survives it.
    // This is the Stage F guarantee: a symbolic parameter is not evaluated,
    // substituted or renamed.
    let rule = rules::builtin("p-to-rz").unwrap();
    assert!(rule.is_parameter_transparent());

    let expanded = rule
        .expand(&[Param::symbol("theta")], &[QubitId(0)])
        .unwrap();
    assert_eq!(expanded.len(), 1);
    let Instruction::Gate {
        kind: GateKind::Rz(param),
        ..
    } = &expanded[0]
    else {
        panic!("expected an rz, got {:?}", expanded[0]);
    };
    assert_eq!(param, &Param::symbol("theta"));
}

#[test]
fn a_transforming_rule_refuses_a_symbol_rather_than_guessing() {
    // `rx-to-rz-sx` needs `theta + pi`, and `Param` has no arithmetic. The
    // only safe answer is to refuse: emitting `Rz(theta)` unchanged would be
    // wrong, and no oracle in the project could catch it, because the
    // state-vector harness cannot simulate a symbolic parameter at all.
    let rule = rules::builtin("rx-to-rz-sx").unwrap();
    assert!(!rule.is_parameter_transparent());

    let err = rule
        .expand(&[Param::symbol("theta")], &[QubitId(0)])
        .unwrap_err();
    assert!(matches!(
        err,
        RuleError::SymbolicParameterRequiresTransformation { ref symbol, .. } if symbol == "theta"
    ));
}

#[test]
fn transparency_is_computed_over_the_whole_closure() {
    let profile = builtin::linear_nisq(4);
    let set = RuleSet::new(&profile).unwrap();

    // `rz` is native, so nothing touches its parameter.
    assert!(set.is_parameter_transparent("rz"));
    // `p` becomes `rz` with the angle unchanged.
    assert!(set.is_parameter_transparent("p"));
    // `rx` and `u` both carry `+pi` offsets.
    assert!(!set.is_parameter_transparent("rx"));
    assert!(!set.is_parameter_transparent("u"));
}

#[test]
fn a_concrete_parameter_is_transformed_as_declared() {
    let transform = ParamTransform::Affine {
        index: 0,
        scale: -1.0,
        offset: std::f64::consts::PI,
    };
    let got = transform.apply(&[Param::concrete(0.5)], "test", 0).unwrap();
    let Param::Concrete(angle) = got else {
        panic!("expected a concrete angle");
    };
    assert!((angle.radians() - (std::f64::consts::PI - 0.5)).abs() < 1e-12);
}

// --- Rule-set validation ----------------------------------------------------

#[test]
fn the_linear_nisq_rule_set_closes_over_its_basis() {
    // The property that makes the profile usable: every registered gate has a
    // route into {rz, sx, x, cx}. Before this phase the profile named two
    // rules and would have got stuck on the first `Swap` routing inserted.
    let profile = builtin::linear_nisq(5);
    let set = RuleSet::new(&profile).expect("linear-nisq must be lowerable");

    for mnemonic in [
        "id", "y", "z", "h", "s", "sdg", "t", "tdg", "sxdg", "rx", "ry", "p", "u", "cy", "cz",
        "swap", "ccx",
    ] {
        assert!(
            set.rule_for(mnemonic).is_some(),
            "`{mnemonic}` is outside the basis and has no rule"
        );
        for reached in set.closure(mnemonic) {
            assert!(
                profile.supports_operation(reached) || set.rule_for(reached).is_some(),
                "lowering `{mnemonic}` reaches `{reached}`, which is a dead end"
            );
        }
    }
}

#[test]
fn a_profile_naming_no_rules_is_valid_when_it_needs_none() {
    // `ideal-simulator` supports every gate, so an empty rule set closes
    // trivially. "No rules" and "incomplete rules" are different things.
    let set = RuleSet::new(&builtin::ideal_simulator()).unwrap();
    assert_eq!(set.rules().count(), 0);
}

#[test]
fn a_rule_set_that_cannot_reach_the_basis_is_rejected_at_construction() {
    // This is the shape `linear_nisq` had before this phase: it names the
    // rule for `h` but nothing for `swap`, so the first routed circuit would
    // have got stuck. Catching it here means the failure names the profile,
    // not some unlucky circuit compiled hours later.
    let profile = BasisProfileBuilder::new("incomplete", "1", "test", Topology::linear(3))
        .operations(["rz", "sx", "cx"])
        .decomposition_rules(["swap-to-cx", "h-to-rz-sx", "ccx-to-cx"])
        .cost_model("uniform")
        .build()
        .unwrap();

    // `ccx-to-cx` emits `t` and `tdg`, which are neither native here nor
    // covered by a rule.
    let err = RuleSet::new(&profile).unwrap_err();
    assert!(
        matches!(&err, RuleSetError::NotClosed { mnemonic, .. } if mnemonic == "t" || mnemonic == "tdg"),
        "got {err:?}"
    );
}

#[test]
fn an_unknown_rule_identifier_is_rejected() {
    let profile = BasisProfileBuilder::new("bogus", "1", "test", Topology::linear(2))
        .operations(["cx"])
        .decomposition_rule("h-to-magic-dust")
        .cost_model("uniform")
        .build()
        .unwrap();
    assert!(matches!(
        RuleSet::new(&profile).unwrap_err(),
        RuleSetError::UnknownRule { .. }
    ));
}

#[test]
fn rank_strictly_decreases_along_every_rewrite() {
    // The termination argument, as an executable assertion. Each rewrite
    // replaces an operation of rank `r` with operations of rank strictly less
    // than `r`, so the multiset of ranks decreases in a well-founded order
    // and rewriting cannot run forever.
    let set = RuleSet::new(&builtin::linear_nisq(5)).unwrap();
    for rule in set.rules() {
        let source_rank = set.rank(rule.source);
        for emitted in rule.emits() {
            assert!(
                set.rank(emitted) < source_rank,
                "`{}` emits `{emitted}` at rank {} but has rank {source_rank}",
                rule.source,
                set.rank(emitted)
            );
        }
    }
}

#[test]
fn no_single_qubit_rule_reaches_a_two_qubit_operation() {
    // What licenses the lowering schedule to be a sequence rather than a
    // loop: single-qubit cleanup runs after orientation repair, and this is
    // the guarantee that it cannot introduce a fresh two-qubit gate and
    // reopen the connectivity question.
    let set = RuleSet::new(&builtin::linear_nisq(5)).unwrap();
    for rule in set.rules() {
        if rule.source_arity != 1 {
            continue;
        }
        for reached in set.closure(rule.source) {
            let kind = oqci::frontend::map_gate(reached, params_placeholder(reached)).unwrap();
            assert_eq!(
                kind.arity(),
                Some(1),
                "single-qubit rule `{}` reaches `{reached}`",
                rule.id
            );
        }
    }
}

fn params_placeholder(mnemonic: &str) -> Vec<Param> {
    let count = match mnemonic {
        "rx" | "ry" | "rz" | "p" => 1,
        "u" => 3,
        _ => 0,
    };
    vec![Param::concrete(0.0); count]
}

#[test]
fn effective_exactness_is_folded_over_the_closure() {
    // `cz-to-cx` is exact on its own, but on a target without `h` its
    // expansion runs through `h-to-rz-sx`, which is not. Reading the
    // declaration alone would report a fidelity the lowered circuit does not
    // have.
    let set = RuleSet::new(&builtin::linear_nisq(4)).unwrap();
    assert_eq!(
        rules::builtin("cz-to-cx").unwrap().exactness,
        Exactness::Exact
    );
    assert_eq!(set.effective_exactness("cz"), Exactness::UpToGlobalPhase);

    // `swap` reaches only `cx`, which is native, so it stays exact.
    assert_eq!(set.effective_exactness("swap"), Exactness::Exact);
    // A native operation is trivially exact.
    assert_eq!(set.effective_exactness("rz"), Exactness::Exact);
}

#[test]
fn every_builtin_rule_only_permutes_the_operands_it_was_given() {
    // Operand closure. Routing's correctness depends on it: if a rule could
    // name a qubit outside its source's operands, decomposition could place a
    // two-qubit gate on a non-adjacent pair and silently undo routing.
    for rule in rules::all_builtin() {
        for step in rule.steps {
            for operand in step.operands {
                assert!(
                    *operand < rule.source_arity,
                    "rule `{}` step uses operand {operand} of {}",
                    rule.id,
                    rule.source_arity
                );
            }
        }
        let operands: Vec<QubitId> = (0..rule.source_arity as u32).map(QubitId).collect();
        let expanded = rule.expand(&params_for(rule, 0), &operands).unwrap();
        for instruction in &expanded {
            for qubit in instruction.qubits() {
                assert!(qubit.index() < rule.source_arity as u32);
            }
        }
    }
}

#[test]
fn every_rule_identifier_names_the_operation_it_rewrites() {
    // A naming convention worth enforcing rather than hoping for: a profile
    // lists rules by identifier, so `swap-to-cx` sitting in a profile that
    // does not support `swap` should be obvious on sight. It also lets
    // `builtin.rs` check its own rule list by construction.
    for rule in rules::all_builtin() {
        let prefix = format!("{}-to-", rule.source);
        assert!(
            rule.id.starts_with(&prefix),
            "rule `{}` rewrites `{}` but is not named `{prefix}...`",
            rule.id,
            rule.source
        );
    }
}

#[test]
fn every_builtin_rule_is_reachable_by_its_identifier() {
    for id in rules::builtin_ids() {
        assert!(rules::builtin(id).is_some(), "`{id}` is not resolvable");
    }
    assert!(rules::builtin("no-such-rule").is_none());
}
