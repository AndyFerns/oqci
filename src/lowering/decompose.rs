//! Applying decomposition rules to a circuit.
//!
//! [`rules`](crate::lowering::rules) says *what* a rewrite is; this module
//! runs it. The interesting part is not the rewriting, which is
//! straightforward, but the schedule around it — see
//! [`crate::lowering`] for why lowering is three ordered phases rather than
//! one loop to a fixed point.
//!
//! # The exhaustive match is deliberate
//!
//! The per-instruction dispatch matches on [`Instruction`] and on [`GateKind`]
//! with **no wildcard arm**. That is not stylistic. Every rule marked
//! [`Exactness::UpToGlobalPhase`](crate::lowering::Exactness::UpToGlobalPhase)
//! is sound only because nothing in this IR applies a fragment conditionally
//! or under control — see that type's documentation for the full argument.
//! Adding a variant that *could* (a controlled-composite gate, a
//! classically-conditioned operation, a `ctrl @` modifier) silently
//! invalidates the entire rule library at once. A wildcard arm would let such
//! a variant compile. Without one, the build breaks here and a human has to
//! read the paragraph above before proceeding.

use crate::ir::{Circuit, GateKind, Instruction, Param};
use crate::lowering::LoweringError;
use crate::lowering::rules::RuleSet;
use crate::target::BasisProfile;

/// How much of the gate set a pass is trying to eliminate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Rewrite only operations of arity three or more.
    ///
    /// Runs **before** layout and routing. A coupling map describes pairs, so
    /// a three-qubit gate has no meaning on one: routing cannot make three
    /// qubits mutually adjacent, and would pass a `Ccx` through untouched.
    /// [`crate::target::check`] would then declare the result legal, because
    /// it only checks connectivity for operations with exactly two operands.
    /// That is a complete path from a valid input to a "verified" output that
    /// no device can run, and reducing arity first is what closes it.
    ArityOnly,
    /// Rewrite every operation the target does not support.
    Basis,
    /// Rewrite only one-qubit operations the target does not support.
    ///
    /// The cleanup phase after orientation repair. Restricted to one-qubit
    /// operations so it cannot introduce a two-qubit gate and reopen the
    /// connectivity question — a restriction the rule set's arity check
    /// independently guarantees, making this belt and braces.
    SingleQubitOnly,
}

impl Scope {
    /// Whether an operation is in scope for rewriting.
    fn admits(self, kind: &GateKind, profile: &BasisProfile, arity: usize) -> bool {
        match self {
            Scope::ArityOnly => arity > 2 && !profile.supports_operation(kind.mnemonic()),
            Scope::Basis => !profile.supports_operation(kind.mnemonic()),
            Scope::SingleQubitOnly => arity == 1 && !profile.supports_operation(kind.mnemonic()),
        }
    }
}

/// What a decomposition pass did.
#[derive(Debug, Clone, Default)]
pub struct DecomposeOutput {
    /// The rewritten instruction list.
    pub instructions: Vec<Instruction>,
    /// How many source operations were rewritten.
    pub rewritten: usize,
    /// Identifiers of the rules that fired, ascending and deduplicated.
    pub rules_applied: Vec<String>,
}

/// Rewrites every in-scope operation until nothing in scope remains.
///
/// # Termination
///
/// Guaranteed by construction, not by a timeout. [`RuleSet::new`] proved the
/// expansion graph acyclic, so `rank(m)` — the longest path from `m` — is
/// well defined, and every rewrite replaces an operation of rank `r` with
/// operations of strictly smaller rank. The multiset of ranks therefore
/// decreases in the multiset order, which is well founded, so the recursion
/// below bottoms out. The depth guard exists only to turn a hypothetical
/// hole in that reasoning into a named error instead of a stack overflow;
/// reaching it means [`RuleSet::new`] has a bug.
///
/// # Errors
///
/// [`LoweringError::NoDecompositionRule`] for an unsupported operation the
/// target has no rule for, [`LoweringError::Rule`] if a rule refuses (most
/// often a symbolic parameter it would have to transform), or
/// [`LoweringError::OpaqueOperation`] for an opaque gate the target does not
/// natively support.
pub fn decompose(
    instructions: &[Instruction],
    profile: &BasisProfile,
    rules: &RuleSet,
    scope: Scope,
) -> Result<DecomposeOutput, LoweringError> {
    let mut out = DecomposeOutput::default();
    let mut applied = std::collections::BTreeSet::new();

    for instruction in instructions {
        let before = out.instructions.len();
        decompose_instruction(
            instruction,
            profile,
            rules,
            scope,
            0,
            &mut out.instructions,
            &mut applied,
        )?;
        // One source operation counts as rewritten when what came out is not
        // the single instruction that went in.
        if out.instructions.len() != before + 1 || out.instructions.get(before) != Some(instruction)
        {
            out.rewritten += 1;
        }
    }

    out.rules_applied = applied.into_iter().collect();
    Ok(out)
}

/// The recursion depth at which the termination proof is assumed broken.
///
/// The deepest legitimate chain in the built-in library is three
/// (`ccx -> h -> rz`), so this is roomy by two orders of magnitude. It is an
/// assertion, not a policy: see [`decompose`].
const MAX_DEPTH: usize = 64;

fn decompose_instruction(
    instruction: &Instruction,
    profile: &BasisProfile,
    rules: &RuleSet,
    scope: Scope,
    depth: usize,
    out: &mut Vec<Instruction>,
    applied: &mut std::collections::BTreeSet<String>,
) -> Result<(), LoweringError> {
    if depth > MAX_DEPTH {
        return Err(LoweringError::DecompositionDidNotConverge {
            mnemonic: mnemonic_of(instruction).to_string(),
            depth,
        });
    }

    // See the module documentation: no wildcard arm, on purpose.
    let (kind, qubits) = match instruction {
        Instruction::Gate { kind, qubits } => (kind, qubits),
        // Measurement and reset are non-unitary. No sequence of gates
        // produces either, so there is nothing to decompose: a target that
        // cannot measure or reset is refused outright, by the legality check
        // and by routing, rather than papered over here.
        Instruction::Measure { .. } | Instruction::Reset { .. } => {
            out.push(instruction.clone());
            return Ok(());
        }
    };

    let arity = qubits.len();
    if !scope.admits(kind, profile, arity) {
        out.push(instruction.clone());
        return Ok(());
    }

    // An opaque gate could be anything — including a controlled-composite,
    // which is exactly what the global-phase invariant rules out. Refusing is
    // the only honest answer: this compiler does not know its matrix, so it
    // can neither decompose it nor certify that leaving it alone is safe.
    if let GateKind::Opaque { name, .. } = kind {
        return Err(LoweringError::OpaqueOperation {
            name: name.clone(),
            target: profile.id().to_string(),
        });
    }

    let mnemonic = kind.mnemonic();
    let Some(rule) = rules.rule_for(mnemonic) else {
        return Err(LoweringError::NoDecompositionRule {
            mnemonic: mnemonic.to_string(),
            target: profile.qualified_id(),
        });
    };

    let params: Vec<Param> = kind.params();
    let expanded = rule
        .expand(&params, qubits)
        .map_err(|source| LoweringError::Rule { source })?;
    applied.insert(rule.id.to_string());

    for step in &expanded {
        decompose_instruction(step, profile, rules, scope, depth + 1, out, applied)?;
    }
    Ok(())
}

/// A readable name for an instruction, for diagnostics.
fn mnemonic_of(instruction: &Instruction) -> &str {
    match instruction {
        Instruction::Gate { kind, .. } => kind.mnemonic(),
        Instruction::Measure { .. } => "measure",
        Instruction::Reset { .. } => "reset",
    }
}

/// Rebuilds a circuit from a rewritten instruction list, keeping its name and
/// register widths.
///
/// Going back through [`crate::ir::CircuitBuilder`] is load-bearing: it
/// re-validates every invariant, so a decomposition bug produces an
/// [`crate::ir::IrError`] rather than a corrupt circuit that flows downstream.
pub(crate) fn rebuild(
    original: &Circuit,
    instructions: Vec<Instruction>,
) -> Result<Circuit, LoweringError> {
    rebuild_with_width(original, instructions, original.num_qubits())
}

/// Rebuilds a circuit over a possibly different qubit register.
///
/// Routing needs this: its output is indexed by *physical* qubit, so the
/// register is generally wider than the logical circuit's.
/// [`crate::pass::rebuild`] copies the original's width and so cannot serve.
pub(crate) fn rebuild_with_width(
    original: &Circuit,
    instructions: Vec<Instruction>,
    num_qubits: u32,
) -> Result<Circuit, LoweringError> {
    let mut builder = crate::ir::CircuitBuilder::new(original.name());
    builder.alloc_qubits(num_qubits);
    builder.alloc_clbits(original.num_clbits());
    for instruction in instructions {
        match instruction {
            Instruction::Gate { kind, qubits } => {
                builder.gate(kind, qubits);
            }
            Instruction::Measure { qubit, target } => {
                builder.measure(qubit, target);
            }
            Instruction::Reset { qubit } => {
                builder.reset(qubit);
            }
        }
    }
    builder
        .build()
        .map_err(|source| LoweringError::Ir { source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CircuitBuilder, QubitId};
    use crate::target::builtin;

    fn nisq() -> (BasisProfile, RuleSet) {
        let profile = builtin::linear_nisq(5);
        let rules = RuleSet::new(&profile).unwrap();
        (profile, rules)
    }

    fn gate(kind: GateKind, qubits: &[u32]) -> Instruction {
        Instruction::Gate {
            kind,
            qubits: qubits.iter().copied().map(QubitId).collect(),
        }
    }

    #[test]
    fn a_native_operation_is_left_alone() {
        let (profile, rules) = nisq();
        let input = vec![gate(GateKind::Cx, &[0, 1])];
        let out = decompose(&input, &profile, &rules, Scope::Basis).unwrap();
        assert_eq!(out.instructions, input);
        assert_eq!(out.rewritten, 0);
        assert!(out.rules_applied.is_empty());
    }

    #[test]
    fn a_non_native_operation_is_rewritten_into_the_basis() {
        let (profile, rules) = nisq();
        let out = decompose(&[gate(GateKind::H, &[0])], &profile, &rules, Scope::Basis).unwrap();
        assert_eq!(out.rewritten, 1);
        assert_eq!(out.rules_applied, vec!["h-to-rz-sx"]);
        for instruction in &out.instructions {
            let Instruction::Gate { kind, .. } = instruction else {
                unreachable!()
            };
            assert!(profile.supports_operation(kind.mnemonic()));
        }
    }

    #[test]
    fn rewriting_recurses_until_everything_is_native() {
        // `cz -> h, cx` and then `h -> rz, sx`. A single pass would leave the
        // `h` behind, which is the bug this recursion exists to avoid.
        let (profile, rules) = nisq();
        let out = decompose(
            &[gate(GateKind::Cz, &[0, 1])],
            &profile,
            &rules,
            Scope::Basis,
        )
        .unwrap();
        assert_eq!(out.rules_applied, vec!["cz-to-cx", "h-to-rz-sx"]);
        assert!(out.instructions.iter().all(|i| {
            let Instruction::Gate { kind, .. } = i else {
                unreachable!()
            };
            profile.supports_operation(kind.mnemonic())
        }));
    }

    #[test]
    fn arity_reduction_touches_only_wide_gates() {
        // Before layout, only the three-qubit gate is in scope: `h` is not
        // native either, but rewriting it here would be premature work.
        let (profile, rules) = nisq();
        let input = vec![gate(GateKind::Ccx, &[0, 1, 2]), gate(GateKind::H, &[0])];
        let out = decompose(&input, &profile, &rules, Scope::ArityOnly).unwrap();

        assert_eq!(out.rules_applied, vec!["ccx-to-cx"]);
        assert!(out.instructions.contains(&gate(GateKind::H, &[0])));
        assert!(
            out.instructions.iter().all(|i| i.qubits().len() <= 2),
            "no operation may still take three qubits"
        );
    }

    #[test]
    fn single_qubit_cleanup_leaves_two_qubit_gates_alone() {
        // The phase after orientation repair: it must not touch the two-qubit
        // structure routing just established.
        let profile = crate::target::BasisProfileBuilder::new(
            "t",
            "1",
            "b",
            crate::target::Topology::linear(3),
        )
        .operations(["rz", "sx", "cx"])
        .decomposition_rules(["h-to-rz-sx", "swap-to-cx"])
        .cost_model("uniform")
        .build()
        .unwrap();
        let rules = RuleSet::new(&profile).unwrap();

        let input = vec![gate(GateKind::Swap, &[0, 1]), gate(GateKind::H, &[0])];
        let out = decompose(&input, &profile, &rules, Scope::SingleQubitOnly).unwrap();

        assert!(out.instructions.contains(&gate(GateKind::Swap, &[0, 1])));
        assert_eq!(out.rules_applied, vec!["h-to-rz-sx"]);
    }

    #[test]
    fn measurement_and_reset_pass_through_untouched() {
        // Non-unitary: no rule can produce either, so decomposition is not
        // where a target's inability to measure gets reported.
        let (profile, rules) = nisq();
        let input = vec![
            Instruction::Measure {
                qubit: QubitId(0),
                target: crate::ir::ClbitId(0),
            },
            Instruction::Reset { qubit: QubitId(0) },
        ];
        let out = decompose(&input, &profile, &rules, Scope::Basis).unwrap();
        assert_eq!(out.instructions, input);
        assert_eq!(out.rewritten, 0);
    }

    #[test]
    fn an_operation_with_no_rule_is_refused_by_name() {
        let profile = crate::target::BasisProfileBuilder::new(
            "bare",
            "1",
            "b",
            crate::target::Topology::linear(2),
        )
        .operations(["cx"])
        .cost_model("uniform")
        .build()
        .unwrap();
        let rules = RuleSet::new(&profile).unwrap();

        let err =
            decompose(&[gate(GateKind::H, &[0])], &profile, &rules, Scope::Basis).unwrap_err();
        assert!(
            matches!(&err, LoweringError::NoDecompositionRule { mnemonic, .. } if mnemonic == "h"),
            "got {err:?}"
        );
    }

    #[test]
    fn an_opaque_gate_is_refused_rather_than_guessed_at() {
        // An opaque gate could be a controlled-composite in disguise, which
        // is precisely what the global-phase invariant excludes. The compiler
        // does not know its matrix and must not pretend otherwise.
        let (profile, rules) = nisq();
        let opaque = gate(
            GateKind::Opaque {
                name: "iswap".into(),
                params: vec![],
            },
            &[0, 1],
        );
        let err = decompose(&[opaque], &profile, &rules, Scope::Basis).unwrap_err();
        assert!(
            matches!(&err, LoweringError::OpaqueOperation { name, .. } if name == "iswap"),
            "got {err:?}"
        );
    }

    #[test]
    fn an_opaque_gate_the_target_supports_is_left_alone() {
        // The escape hatch stays open: a backend that genuinely implements
        // `iswap` can declare it, and lowering then has nothing to do.
        let profile = crate::target::BasisProfileBuilder::new(
            "exotic",
            "1",
            "b",
            crate::target::Topology::linear(2),
        )
        .operations(["iswap", "cx"])
        .cost_model("uniform")
        .build()
        .unwrap();
        let rules = RuleSet::new(&profile).unwrap();
        let opaque = gate(
            GateKind::Opaque {
                name: "iswap".into(),
                params: vec![],
            },
            &[0, 1],
        );
        let out = decompose(
            std::slice::from_ref(&opaque),
            &profile,
            &rules,
            Scope::Basis,
        )
        .unwrap();
        assert_eq!(out.instructions, vec![opaque]);
    }

    #[test]
    fn a_symbolic_parameter_needing_arithmetic_is_refused() {
        let (profile, rules) = nisq();
        let err = decompose(
            &[gate(GateKind::Rx(Param::symbol("theta")), &[0])],
            &profile,
            &rules,
            Scope::Basis,
        )
        .unwrap_err();
        assert!(matches!(err, LoweringError::Rule { .. }), "got {err:?}");
    }

    #[test]
    fn a_symbolic_parameter_survives_a_transparent_rule() {
        let (profile, rules) = nisq();
        let out = decompose(
            &[gate(GateKind::P(Param::symbol("theta")), &[0])],
            &profile,
            &rules,
            Scope::Basis,
        )
        .unwrap();
        assert_eq!(
            out.instructions,
            vec![gate(GateKind::Rz(Param::symbol("theta")), &[0])]
        );
    }

    #[test]
    fn the_identity_gate_decomposes_to_nothing() {
        let (profile, rules) = nisq();
        let out = decompose(&[gate(GateKind::I, &[0])], &profile, &rules, Scope::Basis).unwrap();
        assert!(out.instructions.is_empty());
        assert_eq!(out.rewritten, 1);
    }

    #[test]
    fn rewriting_is_deterministic() {
        let (profile, rules) = nisq();
        let input = vec![
            gate(GateKind::Ccx, &[0, 1, 2]),
            gate(GateKind::Cz, &[0, 1]),
            gate(GateKind::H, &[2]),
        ];
        let once = decompose(&input, &profile, &rules, Scope::Basis).unwrap();
        let twice = decompose(&input, &profile, &rules, Scope::Basis).unwrap();
        assert_eq!(once.instructions, twice.instructions);
        assert_eq!(once.rules_applied, twice.rules_applied);
    }

    #[test]
    fn a_rebuilt_circuit_is_revalidated() {
        // The guarantee that a decomposition bug cannot produce a corrupt
        // circuit: it produces an error instead.
        let mut b = CircuitBuilder::new("t");
        b.alloc_qubits(2);
        let original = b.build().unwrap();

        // An operand outside the register the rebuild declares.
        let err = rebuild(&original, vec![gate(GateKind::X, &[7])]).unwrap_err();
        assert!(matches!(err, LoweringError::Ir { .. }), "got {err:?}");
    }

    #[test]
    fn rebuilding_can_widen_the_register_for_physical_qubits() {
        let mut b = CircuitBuilder::new("t");
        b.alloc_qubits(2);
        let original = b.build().unwrap();

        let wider = rebuild_with_width(&original, vec![gate(GateKind::X, &[4])], 5).unwrap();
        assert_eq!(wider.num_qubits(), 5);
        assert_eq!(wider.name(), "t");
    }
}
