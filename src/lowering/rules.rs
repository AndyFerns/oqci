//! Decomposition rules: how a non-native operation becomes native ones.
//!
//! Stage D §5 requires every non-native operation reaching target lowering to
//! have a documented strategy, defining its source operation, target sequence,
//! parameter transformation, operand mapping, classical behaviour,
//! semantic-preservation expectation, exactness, and cost implications. This
//! module is that data model, and [`RuleSet`] is what validates it.
//!
//! Until now a [`crate::target::BasisProfile`] recorded rules as bare
//! identifier strings that nothing read. Those identifiers are now keys into
//! this library, which is why `builtin::linear_nisq` had to grow: a profile
//! that names `h-to-rz-sx` but no rule for `swap` cannot be lowered to once
//! routing starts inserting `Swap`s, and [`RuleSet::new`] now says so instead
//! of failing later on a specific circuit.
//!
//! # Two Stage D §5 fields are deliberately not stored
//!
//! **Target-specific cost implications.** Storing a cost number here would
//! create a second source of truth that could contradict the
//! [`crate::target::CostModel`] — the failure `BasisProfile` already avoids
//! for `supports_operation("measure")`. A rule's cost is *derived* by costing
//! the sequence it emits, which is Stage E's rule that the target owns cost.
//!
//! **Classical/result behaviour.** Every rule source and target is a unitary
//! gate, so rather than modelling a one-variant enum this is made
//! structurally true: a [`RuleStep`] can only name a gate, and `Measure` and
//! `Reset` are documented as not decomposable. They are non-unitary; no
//! sequence of gates produces them.
//!
//! # Exactness is declared per rule and derived over the closure
//!
//! A rule's [`Exactness`] describes that rule *in isolation*, which is what a
//! per-rule test can check. But exactness does not compose the way one would
//! hope: `cz-to-cx` is exact, yet on a target without `h` its closure runs
//! through `h-to-rz-sx`, which is not. So the *effective* exactness of
//! lowering an operation is folded over its whole closure by
//! [`RuleSet::effective_exactness`] rather than read off the rule. A derived
//! value cannot be a lie; a declared one could.

use std::collections::{BTreeMap, BTreeSet};

use crate::ir::{GateKind, Instruction, Param, QubitId};
use crate::target::BasisProfile;

/// How a rule computes one parameter of one emitted operation.
///
/// Deliberately limited to affine functions of a single source parameter.
/// That covers identity `(1, 0)`, negation `(-1, 0)`, halving `(0.5, 0)` and
/// `theta + pi` `(1, pi)` — every transformation the standard Euler
/// decompositions need — without an expression language, and without
/// [`Param`] growing arithmetic it does not have.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamTransform {
    /// A fixed angle, independent of the source operation.
    Constant(f64),
    /// `scale * source[index] + offset`.
    Affine {
        /// Which of the source operation's parameters to read.
        index: usize,
        /// Multiplier applied to it.
        scale: f64,
        /// Constant added afterwards.
        offset: f64,
    },
}

impl ParamTransform {
    /// Passes a source parameter through unchanged.
    #[must_use]
    pub const fn passthrough(index: usize) -> Self {
        ParamTransform::Affine {
            index,
            scale: 1.0,
            offset: 0.0,
        }
    }

    /// Whether this transform can carry a symbolic parameter through intact.
    ///
    /// True in two cases, for different reasons. An identity
    /// [`ParamTransform::Affine`] passes its source through unchanged. A
    /// [`ParamTransform::Constant`] reads no source parameter at all, so
    /// there is no symbol for it to damage — treating it as opaque would make
    /// `RuleSet::is_parameter_transparent("h")` report `false` for a gate
    /// that has no parameters to lose, and any caller gating on that
    /// predicate would then refuse circuits it should accept.
    ///
    /// This is the property that decides whether a symbolic parameter can
    /// survive a rule — see [`ParamTransform::apply`].
    #[must_use]
    pub fn is_transparent(&self) -> bool {
        match self {
            ParamTransform::Constant(_) => true,
            ParamTransform::Affine { scale, offset, .. } => *scale == 1.0 && *offset == 0.0,
        }
    }

    /// Which source parameter this reads, if any.
    #[must_use]
    pub const fn source_index(&self) -> Option<usize> {
        match self {
            ParamTransform::Constant(_) => None,
            ParamTransform::Affine { index, .. } => Some(*index),
        }
    }

    /// Computes the emitted parameter.
    ///
    /// # Errors
    ///
    /// [`RuleError::SymbolicParameterRequiresTransformation`] when the source
    /// parameter is symbolic and this transform is not the identity.
    ///
    /// This refusal is the single most important line in the module.
    /// [`Param`] has no arithmetic, so there is no way to represent
    /// `-theta` or `theta/2` for a symbol. The alternatives are both silently
    /// wrong: applying the transform as if it were the identity emits
    /// `Rz(theta)` where `Rz(-theta)` was meant, and neither
    /// [`crate::target::check`] nor the state-vector harness can catch that —
    /// the harness refuses to simulate symbolic parameters at all, so the bug
    /// would have *no* oracle. Mangling the symbol's name into `"theta/2"`
    /// is worse still: `Circuit::parameters` would then report a new free
    /// parameter that nothing knows the meaning of, which Stage F §2 and §6
    /// forbid outright.
    ///
    /// So a rule that must transform a symbolic parameter refuses, and the
    /// caller is told to bind parameters first. That is exactly the Stage F
    /// §8 boundary: parameter binding is an explicit compiler step.
    pub fn apply(&self, source: &[Param], rule: &str, step: usize) -> Result<Param, RuleError> {
        match self {
            ParamTransform::Constant(value) => Ok(Param::concrete(*value)),
            ParamTransform::Affine {
                index,
                scale,
                offset,
            } => {
                let param = source.get(*index).ok_or(RuleError::ParameterIndex {
                    rule: rule.to_string(),
                    index: *index,
                    available: source.len(),
                })?;
                match param {
                    Param::Concrete(angle) => Ok(Param::concrete(scale * angle.radians() + offset)),
                    Param::Symbol(symbol) if self.is_transparent() => {
                        Ok(Param::symbol(symbol.clone()))
                    }
                    Param::Symbol(symbol) => {
                        Err(RuleError::SymbolicParameterRequiresTransformation {
                            rule: rule.to_string(),
                            step,
                            symbol: symbol.clone(),
                        })
                    }
                }
            }
        }
    }
}

/// One operation a rule emits.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleStep {
    /// The emitted operation's mnemonic, resolved through
    /// [`crate::frontend::map_gate`] so this library cannot drift from the
    /// frontends' understanding of a gate name.
    pub op: &'static str,
    /// Indices into the **source** operation's operand list.
    ///
    /// A rule may only permute and reuse the operands it was given; it can
    /// never introduce a qubit. [`RuleSet::new`] enforces that, and the whole
    /// routing correctness argument depends on it: if decomposition could
    /// name a new qubit, it could place a two-qubit gate on a non-adjacent
    /// pair and undo routing's work.
    pub operands: &'static [usize],
    /// How to compute each of the emitted operation's parameters.
    pub params: &'static [ParamTransform],
}

/// How faithfully a rule reproduces its source operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Exactness {
    /// The emitted sequence equals the source unitary exactly.
    Exact,
    /// Equal up to an unobservable global phase.
    ///
    /// # The invariant this depends on
    ///
    /// Nothing in OQCI applies a circuit fragment conditionally or under
    /// control: [`Instruction`] has exactly three variants, [`GateKind`] is a
    /// closed enum with no control modifier, classical bits are write-only so
    /// there is no feed-forward, and Stage F places measurement-conditioned
    /// branches out of scope. A rewrite that multiplies one fragment's
    /// unitary by a scalar therefore multiplies the whole circuit by that
    /// scalar, which no experiment can observe. Measurement and reset do not
    /// break this either: `rho = |psi><psi|` is unchanged by a global phase,
    /// so every CPTP map is phase-blind.
    ///
    /// **Every rule marked this way depends on that invariant.** Adding a
    /// controlled-composite gate, a classically-conditioned operation, a
    /// `ctrl @` modifier, or any subroutine that could later be controlled
    /// invalidates all of them at once, and would require either exact rules
    /// or explicit global-phase tracking.
    UpToGlobalPhase,
}

impl Exactness {
    /// The weaker of two exactness claims.
    ///
    /// Used to fold a claim over a rule closure: one inexact step anywhere
    /// makes the whole expansion inexact.
    #[must_use]
    pub fn combine(self, other: Exactness) -> Exactness {
        self.max(other)
    }
}

/// A rewrite from one abstract operation into a sequence of others.
#[derive(Debug, Clone, PartialEq)]
pub struct DecompositionRule {
    /// Stable identifier, matching a [`BasisProfile::decomposition_rules`]
    /// entry.
    pub id: &'static str,
    /// The mnemonic this rule rewrites.
    pub source: &'static str,
    /// How many qubits the source operation takes.
    pub source_arity: usize,
    /// How many parameters the source operation takes.
    pub source_params: usize,
    /// The sequence it becomes.
    pub steps: &'static [RuleStep],
    /// This rule's fidelity, in isolation. See the module docs on why the
    /// effective value is derived rather than read from here.
    pub exactness: Exactness,
    /// Why this rule is correct, in one line — shown in reports.
    pub note: &'static str,
}

impl DecompositionRule {
    /// Expands one instance of the source operation.
    ///
    /// # Errors
    ///
    /// [`RuleError::OperandArity`] if the operand count does not match the
    /// rule, [`RuleError::SymbolicParameterRequiresTransformation`] if a
    /// symbolic parameter would have to be transformed, or
    /// [`RuleError::UnknownOperation`] if a step names a gate the shared gate
    /// table does not know.
    pub fn expand(
        &self,
        params: &[Param],
        operands: &[QubitId],
    ) -> Result<Vec<Instruction>, RuleError> {
        if operands.len() != self.source_arity {
            return Err(RuleError::OperandArity {
                rule: self.id.to_string(),
                expected: self.source_arity,
                found: operands.len(),
            });
        }
        let mut out = Vec::with_capacity(self.steps.len());
        for (index, step) in self.steps.iter().enumerate() {
            let mut step_params = Vec::with_capacity(step.params.len());
            for transform in step.params {
                step_params.push(transform.apply(params, self.id, index)?);
            }
            let kind = resolve(step.op, step_params, self.id)?;
            let qubits: Vec<QubitId> = step
                .operands
                .iter()
                .map(|i| {
                    operands.get(*i).copied().ok_or(RuleError::OperandIndex {
                        rule: self.id.to_string(),
                        index: *i,
                        available: operands.len(),
                    })
                })
                .collect::<Result<_, _>>()?;
            out.push(Instruction::Gate { kind, qubits });
        }
        Ok(out)
    }

    /// Whether every parameter this rule emits passes its source through
    /// unchanged, so a symbolic parameter survives the rewrite.
    #[must_use]
    pub fn is_parameter_transparent(&self) -> bool {
        self.steps
            .iter()
            .flat_map(|step| step.params)
            .all(ParamTransform::is_transparent)
    }

    /// The distinct mnemonics this rule emits, ascending.
    #[must_use]
    pub fn emits(&self) -> BTreeSet<&'static str> {
        self.steps.iter().map(|step| step.op).collect()
    }
}

/// Resolves a mnemonic through the shared gate table, refusing the opaque
/// fallback.
///
/// `map_gate` answers an unknown name with [`GateKind::Opaque`], which is the
/// right behaviour for a frontend reading someone else's program and the
/// wrong behaviour here: a rule naming a gate that does not exist is an
/// authoring mistake, and turning it into an opaque operation would produce a
/// circuit no backend can run.
fn resolve(op: &str, params: Vec<Param>, rule: &str) -> Result<GateKind, RuleError> {
    let kind =
        crate::frontend::map_gate(op, params).map_err(|source| RuleError::UnknownOperation {
            rule: rule.to_string(),
            op: op.to_string(),
            detail: source.to_string(),
        })?;
    if matches!(kind, GateKind::Opaque { .. }) {
        return Err(RuleError::UnknownOperation {
            rule: rule.to_string(),
            op: op.to_string(),
            detail: "not a registered gate".to_string(),
        });
    }
    Ok(kind)
}

/// A rule that could not be applied to a particular operation.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum RuleError {
    /// A symbolic parameter would have to be transformed, and [`Param`] has
    /// no arithmetic to transform it with.
    #[error(
        "rule `{rule}` step {step} must transform parameter `{symbol}`, \
         which is still symbolic; bind parameters before lowering"
    )]
    SymbolicParameterRequiresTransformation {
        /// The rule that refused.
        rule: String,
        /// Which emitted step needed the transformation.
        step: usize,
        /// The unbound symbol.
        symbol: String,
    },
    /// The operation had the wrong number of qubits for this rule.
    #[error("rule `{rule}` expects {expected} qubit(s), got {found}")]
    OperandArity {
        /// The rule.
        rule: String,
        /// Operands the rule declares.
        expected: usize,
        /// Operands it was given.
        found: usize,
    },
    /// A step referenced an operand the source does not have.
    #[error("rule `{rule}` references operand {index} of {available}")]
    OperandIndex {
        /// The rule.
        rule: String,
        /// The out-of-range index.
        index: usize,
        /// How many operands exist.
        available: usize,
    },
    /// A transform referenced a parameter the source does not have.
    #[error("rule `{rule}` references parameter {index} of {available}")]
    ParameterIndex {
        /// The rule.
        rule: String,
        /// The out-of-range index.
        index: usize,
        /// How many parameters exist.
        available: usize,
    },
    /// A step named a gate the shared gate table does not register.
    #[error("rule `{rule}` emits unknown operation `{op}`: {detail}")]
    UnknownOperation {
        /// The rule.
        rule: String,
        /// The unrecognised mnemonic.
        op: String,
        /// What the gate table said.
        detail: String,
    },
}

/// A rule set that is malformed, or that cannot reach the target's basis.
///
/// Every variant here is detected when the set is built, before any circuit
/// is touched — the same discipline `BasisProfileBuilder::build` follows.
/// Discovering mid-compilation that a rule set cannot terminate, or leaves an
/// operation non-native, would mean failing deep inside a rewrite with a
/// half-lowered circuit in hand.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum RuleSetError {
    /// The profile named a rule this library does not contain.
    #[error("target names unknown decomposition rule `{id}`")]
    UnknownRule {
        /// The unrecognised identifier.
        id: String,
    },
    /// Two rules rewrite the same operation.
    #[error("rules `{first}` and `{second}` both rewrite `{mnemonic}`")]
    DuplicateSource {
        /// The first rule.
        first: String,
        /// The second.
        second: String,
        /// The operation they contest.
        mnemonic: String,
    },
    /// A rule is internally malformed.
    #[error("rule `{rule}` is malformed: {detail}")]
    MalformedRule {
        /// The rule.
        rule: String,
        /// What is wrong with it.
        detail: String,
    },
    /// Rewriting could not terminate.
    #[error("decomposition rules form a cycle through `{mnemonic}`")]
    CyclicRuleSet {
        /// An operation on the cycle.
        mnemonic: String,
    },
    /// An operation the rules reach is neither native nor further reducible.
    #[error(
        "rule `{rule}` emits `{mnemonic}`, which the target neither supports nor can decompose"
    )]
    NotClosed {
        /// The rule whose expansion gets stuck.
        rule: String,
        /// The operation with nowhere to go.
        mnemonic: String,
    },
    /// A rule reaches an operation wider than its own source.
    #[error("rule `{rule}` reaches `{mnemonic}`, which is wider than the operation it rewrites")]
    ArityIncrease {
        /// The offending rule.
        rule: String,
        /// The multi-qubit operation it reaches.
        mnemonic: String,
    },
}

/// The decomposition rules a particular target uses, validated.
///
/// Construction proves everything the lowering schedule relies on, so that
/// rewriting itself needs no runtime termination check.
#[derive(Debug, Clone)]
pub struct RuleSet {
    by_source: BTreeMap<&'static str, DecompositionRule>,
    /// Longest path from each mnemonic in the expansion graph. Strictly
    /// decreasing along every rewrite, which is what proves termination.
    rank: BTreeMap<&'static str, usize>,
}

impl RuleSet {
    /// Builds and validates the rules a profile names.
    ///
    /// # Errors
    ///
    /// Any [`RuleSetError`]. The checks, and what each one buys:
    ///
    /// 1. **Every named rule exists**, and no two rewrite the same operation —
    ///    otherwise "the rule for `cx`" would be ambiguous.
    /// 2. **Each rule is well formed**: its steps name registered gates, with
    ///    matching operand and parameter counts, and every operand index is
    ///    within the source's arity. That last check — *operand closure* — is
    ///    what guarantees a rule can only permute the qubits it was handed.
    /// 3. **Acyclicity.** The expansion graph over mnemonics must be acyclic,
    ///    which makes `rank(m) = longest path from m` well defined. Each
    ///    rewrite then replaces an operation of rank `r` with operations of
    ///    rank `< r`, so the multiset of ranks strictly decreases in the
    ///    multiset order — a well-founded ordering, so rewriting terminates.
    ///    This is a proof, not a heuristic, which is why the rewriter needs no
    ///    iteration cap to be safe.
    /// 4. **Closure.** Every mnemonic reachable from a rule source is either
    ///    native to this profile or has a rule of its own. This is what makes
    ///    the result *legal* rather than merely finished.
    /// 5. **Arity non-increase for one-qubit sources.** Nothing reachable from
    ///    a one-qubit rule may be a multi-qubit operation. The lowering
    ///    schedule runs single-qubit cleanup *after* orientation repair, and
    ///    this is what guarantees that cleanup cannot introduce a fresh
    ///    two-qubit gate and reopen the loop.
    pub fn new(profile: &BasisProfile) -> Result<Self, RuleSetError> {
        let mut by_source: BTreeMap<&'static str, DecompositionRule> = BTreeMap::new();

        for id in profile.decomposition_rules() {
            let rule =
                builtin(id).ok_or_else(|| RuleSetError::UnknownRule { id: id.to_string() })?;
            validate_shape(&rule)?;
            if let Some(existing) = by_source.get(rule.source) {
                return Err(RuleSetError::DuplicateSource {
                    first: existing.id.to_string(),
                    second: rule.id.to_string(),
                    mnemonic: rule.source.to_string(),
                });
            }
            by_source.insert(rule.source, rule);
        }

        let rank = rank_all(&by_source)?;
        let set = RuleSet { by_source, rank };
        set.check_closure(profile)?;
        set.check_arity()?;
        Ok(set)
    }

    /// The rule rewriting an operation, if this target has one.
    #[must_use]
    pub fn rule_for(&self, mnemonic: &str) -> Option<&DecompositionRule> {
        self.by_source.get(mnemonic)
    }

    /// Every rule in the set, ordered by the operation it rewrites.
    pub fn rules(&self) -> impl Iterator<Item = &DecompositionRule> {
        self.by_source.values()
    }

    /// How many rewrites deep an operation can go. Zero for a leaf.
    #[must_use]
    pub fn rank(&self, mnemonic: &str) -> usize {
        self.rank.get(mnemonic).copied().unwrap_or(0)
    }

    /// Whether an operation can be lowered without losing a symbolic
    /// parameter — that is, whether every rule in its closure passes
    /// parameters through unchanged.
    ///
    /// Answered from the rule graph, before any circuit is examined, so
    /// lowering can refuse a symbolic circuit up front instead of part way
    /// through a rewrite.
    #[must_use]
    pub fn is_parameter_transparent(&self, mnemonic: &str) -> bool {
        let Some(rule) = self.rule_for(mnemonic) else {
            return true; // Native: nothing transforms it.
        };
        rule.is_parameter_transparent()
            && rule
                .emits()
                .iter()
                .all(|emitted| self.is_parameter_transparent(emitted))
    }

    /// How faithfully an operation is reproduced once fully lowered.
    ///
    /// Folded over the closure rather than read from a single rule, because
    /// exactness does not compose: an exact rule whose expansion runs through
    /// an up-to-phase one is itself only exact up to phase.
    #[must_use]
    pub fn effective_exactness(&self, mnemonic: &str) -> Exactness {
        let Some(rule) = self.rule_for(mnemonic) else {
            return Exactness::Exact;
        };
        rule.emits().iter().fold(rule.exactness, |acc, emitted| {
            acc.combine(self.effective_exactness(emitted))
        })
    }

    /// Every mnemonic reachable from `mnemonic` by rewriting, excluding
    /// itself.
    #[must_use]
    pub fn closure(&self, mnemonic: &str) -> BTreeSet<&'static str> {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<&'static str> = self
            .rule_for(mnemonic)
            .map(|r| r.emits().into_iter().collect())
            .unwrap_or_default();
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                continue;
            }
            if let Some(rule) = self.rule_for(current) {
                stack.extend(rule.emits());
            }
        }
        seen
    }

    /// Check 4: everything the rules reach is native or further reducible.
    fn check_closure(&self, profile: &BasisProfile) -> Result<(), RuleSetError> {
        for rule in self.by_source.values() {
            for emitted in rule.emits() {
                if !profile.supports_operation(emitted) && !self.by_source.contains_key(emitted) {
                    return Err(RuleSetError::NotClosed {
                        rule: rule.id.to_string(),
                        mnemonic: emitted.to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Check 5: no rule ever reaches an operation wider than its own source.
    ///
    /// Stated for every arity rather than only for one-qubit rules. Operand
    /// closure already makes a *single* step no wider than its source — a
    /// step can only name operands the source has — but the transitive claim
    /// is what the lowering schedule actually relies on, and asserting it
    /// directly is cheaper than re-deriving the induction each time someone
    /// reads the code.
    ///
    /// The one-qubit case is the load-bearing one: it is what guarantees that
    /// single-qubit cleanup, which runs *after* orientation repair, cannot
    /// introduce a fresh two-qubit gate and reopen the connectivity question.
    fn check_arity(&self) -> Result<(), RuleSetError> {
        for rule in self.by_source.values() {
            for reachable in self.closure(rule.source) {
                let reached = arity_of(reachable);
                if reached.is_none_or(|arity| arity > rule.source_arity) {
                    return Err(RuleSetError::ArityIncrease {
                        rule: rule.id.to_string(),
                        mnemonic: reachable.to_string(),
                    });
                }
            }
        }
        Ok(())
    }
}

/// The arity of a registered mnemonic, via the shared gate table.
fn arity_of(mnemonic: &str) -> Option<usize> {
    crate::frontend::map_gate(mnemonic, placeholder_params(mnemonic))
        .ok()
        .and_then(|kind| kind.arity())
}

/// Enough concrete parameters to let `map_gate` resolve a name.
///
/// The values are irrelevant — only the resulting [`GateKind`]'s shape is
/// being asked about — but the *count* must be right or `map_gate` rejects it.
fn placeholder_params(mnemonic: &str) -> Vec<Param> {
    let count = match mnemonic {
        "rx" | "ry" | "rz" | "p" | "u1" | "phase" => 1,
        "u2" => 2,
        "u" | "u3" => 3,
        _ => 0,
    };
    vec![Param::concrete(0.0); count]
}

/// Check 2: a rule's steps must be internally consistent.
fn validate_shape(rule: &DecompositionRule) -> Result<(), RuleSetError> {
    let malformed = |detail: String| RuleSetError::MalformedRule {
        rule: rule.id.to_string(),
        detail,
    };

    if arity_of(rule.source) != Some(rule.source_arity) {
        return Err(malformed(format!(
            "declares arity {} but `{}` takes {:?}",
            rule.source_arity,
            rule.source,
            arity_of(rule.source)
        )));
    }

    for (index, step) in rule.steps.iter().enumerate() {
        // Operand closure: a rule may only permute what it was given.
        for operand in step.operands {
            if *operand >= rule.source_arity {
                return Err(malformed(format!(
                    "step {index} uses operand {operand}, but the source has {}",
                    rule.source_arity
                )));
            }
        }
        for transform in step.params {
            if let Some(source) = transform.source_index()
                && source >= rule.source_params
            {
                return Err(malformed(format!(
                    "step {index} reads parameter {source}, but the source has {}",
                    rule.source_params
                )));
            }
        }
        // The emitted gate must exist, and take exactly these operands and
        // parameters.
        let params: Vec<Param> = step.params.iter().map(|_| Param::concrete(0.0)).collect();
        let kind = resolve(step.op, params, rule.id)
            .map_err(|e| malformed(format!("step {index}: {e}")))?;
        if kind.arity() != Some(step.operands.len()) {
            return Err(malformed(format!(
                "step {index} gives `{}` {} operand(s), it takes {:?}",
                step.op,
                step.operands.len(),
                kind.arity()
            )));
        }
    }
    Ok(())
}

/// Check 3: acyclicity, and the rank that proves termination.
///
/// `rank(m)` is the longest path from `m` in the expansion graph. Computed by
/// depth-first search with an on-stack marker, so a cycle is reported by the
/// mnemonic that closes it rather than as a stack overflow.
fn rank_all(
    by_source: &BTreeMap<&'static str, DecompositionRule>,
) -> Result<BTreeMap<&'static str, usize>, RuleSetError> {
    let mut rank = BTreeMap::new();
    let mut on_stack = BTreeSet::new();
    for source in by_source.keys() {
        rank_of(source, by_source, &mut rank, &mut on_stack)?;
    }
    Ok(rank)
}

fn rank_of(
    mnemonic: &'static str,
    by_source: &BTreeMap<&'static str, DecompositionRule>,
    rank: &mut BTreeMap<&'static str, usize>,
    on_stack: &mut BTreeSet<&'static str>,
) -> Result<usize, RuleSetError> {
    if let Some(known) = rank.get(mnemonic) {
        return Ok(*known);
    }
    if !on_stack.insert(mnemonic) {
        return Err(RuleSetError::CyclicRuleSet {
            mnemonic: mnemonic.to_string(),
        });
    }
    let mut depth = 0;
    if let Some(rule) = by_source.get(mnemonic) {
        for emitted in rule.emits() {
            depth = depth.max(1 + rank_of(emitted, by_source, rank, on_stack)?);
        }
    }
    on_stack.remove(mnemonic);
    rank.insert(mnemonic, depth);
    Ok(depth)
}

// ---------------------------------------------------------------------------
// The built-in rule library.
//
// Every identity below was checked numerically against Qiskit's independent
// `quantum_info.Operator` before being written down, and is re-checked on
// every run by `tests/decomposition.rs` against the project's own
// state-vector harness — an implementation that writes each gate's matrix out
// from scratch rather than consulting these tables, so an error here shows up
// as a diverging column.
//
// The Qiskit cross-check is a standing test, not a one-off:
// `python/tests/test_rules.py` reads this table out of the compiler and
// re-checks every identity against `quantum_info.Operator`, deriving each
// rule's exactness from Qiskit's verdict rather than trusting the column
// below. So an error here has to be made identically by two independently
// written implementations of gate semantics to survive.
//
// The `exactness` field is the *verified* answer, not an assumption:
// `Operator(a) == Operator(b)` gives `Exact`, `.equiv(...)` alone gives
// `UpToGlobalPhase`.
// ---------------------------------------------------------------------------

use ParamTransform::{Affine, Constant};

const PI: f64 = std::f64::consts::PI;
const FRAC_PI_2: f64 = std::f64::consts::FRAC_PI_2;
const FRAC_PI_4: f64 = std::f64::consts::FRAC_PI_4;

/// Looks up a rule by the identifier a profile names it with.
#[must_use]
pub fn builtin(id: &str) -> Option<DecompositionRule> {
    BUILTIN.iter().find(|rule| rule.id == id).cloned()
}

/// Every rule this library knows, by identifier.
#[must_use]
pub fn builtin_ids() -> Vec<&'static str> {
    BUILTIN.iter().map(|rule| rule.id).collect()
}

/// Every rule this library knows.
#[must_use]
pub fn all_builtin() -> &'static [DecompositionRule] {
    BUILTIN
}

/// A step with no parameters.
const fn step(op: &'static str, operands: &'static [usize]) -> RuleStep {
    RuleStep {
        op,
        operands,
        params: &[],
    }
}

/// A single-qubit `rz` by a fixed angle.
///
/// Spelled out as consts rather than built by a `const fn` because
/// `RuleStep::params` is `&'static [ParamTransform]`, and a function cannot
/// return a reference to a temporary it just built.
macro_rules! rz_const {
    ($name:ident, $angle:expr) => {
        const $name: RuleStep = RuleStep {
            op: "rz",
            operands: &[0],
            params: &[Constant($angle)],
        };
    };
}

rz_const!(RZ_HALF_PI, FRAC_PI_2);
rz_const!(RZ_NEG_HALF_PI, -FRAC_PI_2);
rz_const!(RZ_PI, PI);
rz_const!(RZ_QUARTER_PI, FRAC_PI_4);
rz_const!(RZ_NEG_QUARTER_PI, -FRAC_PI_4);
rz_const!(RZ_FIVE_HALVES_PI, 5.0 * FRAC_PI_2);

static BUILTIN: &[DecompositionRule] = &[
    DecompositionRule {
        id: "id-to-nothing",
        source: "id",
        source_arity: 1,
        source_params: 0,
        steps: &[],
        exactness: Exactness::Exact,
        note: "the identity gate is the identity matrix",
    },
    DecompositionRule {
        id: "swap-to-cx",
        source: "swap",
        source_arity: 2,
        source_params: 0,
        steps: &[
            step("cx", &[0, 1]),
            step("cx", &[1, 0]),
            step("cx", &[0, 1]),
        ],
        exactness: Exactness::Exact,
        note: "three alternating CNOTs exchange two qubits",
    },
    DecompositionRule {
        id: "cz-to-cx",
        source: "cz",
        source_arity: 2,
        source_params: 0,
        steps: &[step("h", &[1]), step("cx", &[0, 1]), step("h", &[1])],
        exactness: Exactness::Exact,
        note: "conjugating the target by H turns CX into CZ",
    },
    DecompositionRule {
        id: "cy-to-cx",
        source: "cy",
        source_arity: 2,
        source_params: 0,
        steps: &[step("sdg", &[1]), step("cx", &[0, 1]), step("s", &[1])],
        exactness: Exactness::Exact,
        note: "conjugating the target by S turns CX into CY",
    },
    DecompositionRule {
        id: "ccx-to-cx",
        source: "ccx",
        source_arity: 3,
        source_params: 0,
        steps: &[
            step("h", &[2]),
            step("cx", &[1, 2]),
            step("tdg", &[2]),
            step("cx", &[0, 2]),
            step("t", &[2]),
            step("cx", &[1, 2]),
            step("tdg", &[2]),
            step("cx", &[0, 2]),
            step("t", &[1]),
            step("t", &[2]),
            step("h", &[2]),
            step("cx", &[0, 1]),
            step("t", &[0]),
            step("tdg", &[1]),
            step("cx", &[0, 1]),
        ],
        exactness: Exactness::Exact,
        note: "the standard six-CNOT Toffoli",
    },
    DecompositionRule {
        id: "x-to-sx",
        source: "x",
        source_arity: 1,
        source_params: 0,
        steps: &[step("sx", &[0]), step("sx", &[0])],
        exactness: Exactness::Exact,
        note: "SX is the square root of X, so SX twice is X",
    },
    DecompositionRule {
        id: "sxdg-to-sx",
        source: "sxdg",
        source_arity: 1,
        source_params: 0,
        steps: &[step("sx", &[0]), step("sx", &[0]), step("sx", &[0])],
        exactness: Exactness::Exact,
        note: "SX has order four, so SX cubed is its inverse",
    },
    DecompositionRule {
        id: "h-to-rz-sx",
        source: "h",
        source_arity: 1,
        source_params: 0,
        steps: &[RZ_HALF_PI, step("sx", &[0]), RZ_HALF_PI],
        exactness: Exactness::UpToGlobalPhase,
        note: "Rz(pi/2) SX Rz(pi/2) = e^(-i pi/4) H",
    },
    DecompositionRule {
        id: "y-to-rz-x",
        source: "y",
        source_arity: 1,
        source_params: 0,
        steps: &[RZ_PI, step("x", &[0])],
        exactness: Exactness::UpToGlobalPhase,
        note: "Y = i X Z, and the factor is global",
    },
    DecompositionRule {
        id: "z-to-rz",
        source: "z",
        source_arity: 1,
        source_params: 0,
        steps: &[RZ_PI],
        exactness: Exactness::UpToGlobalPhase,
        note: "Z = e^(i pi/2) Rz(pi)",
    },
    DecompositionRule {
        id: "s-to-rz",
        source: "s",
        source_arity: 1,
        source_params: 0,
        steps: &[RZ_HALF_PI],
        exactness: Exactness::UpToGlobalPhase,
        note: "S is Rz(pi/2) up to phase",
    },
    DecompositionRule {
        id: "sdg-to-rz",
        source: "sdg",
        source_arity: 1,
        source_params: 0,
        steps: &[RZ_NEG_HALF_PI],
        exactness: Exactness::UpToGlobalPhase,
        note: "Sdg is Rz(-pi/2) up to phase",
    },
    DecompositionRule {
        id: "t-to-rz",
        source: "t",
        source_arity: 1,
        source_params: 0,
        steps: &[RZ_QUARTER_PI],
        exactness: Exactness::UpToGlobalPhase,
        note: "T is Rz(pi/4) up to phase",
    },
    DecompositionRule {
        id: "tdg-to-rz",
        source: "tdg",
        source_arity: 1,
        source_params: 0,
        steps: &[RZ_NEG_QUARTER_PI],
        exactness: Exactness::UpToGlobalPhase,
        note: "Tdg is Rz(-pi/4) up to phase",
    },
    DecompositionRule {
        id: "p-to-rz",
        source: "p",
        source_arity: 1,
        source_params: 1,
        steps: &[RuleStep {
            op: "rz",
            operands: &[0],
            params: &[ParamTransform::passthrough(0)],
        }],
        exactness: Exactness::UpToGlobalPhase,
        note: "P(lambda) and Rz(lambda) differ by a global phase; the angle passes through unchanged, so a symbolic parameter survives",
    },
    DecompositionRule {
        id: "rx-to-rz-sx",
        source: "rx",
        source_arity: 1,
        source_params: 1,
        steps: &[
            RZ_HALF_PI,
            step("sx", &[0]),
            RuleStep {
                op: "rz",
                operands: &[0],
                params: &[Affine {
                    index: 0,
                    scale: 1.0,
                    offset: PI,
                }],
            },
            step("sx", &[0]),
            RZ_FIVE_HALVES_PI,
        ],
        exactness: Exactness::UpToGlobalPhase,
        note: "Euler ZXZXZ form; the theta+pi offset means a symbolic angle cannot survive this rule",
    },
    DecompositionRule {
        id: "ry-to-rz-sx",
        source: "ry",
        source_arity: 1,
        source_params: 1,
        steps: &[
            step("sx", &[0]),
            RuleStep {
                op: "rz",
                operands: &[0],
                params: &[Affine {
                    index: 0,
                    scale: 1.0,
                    offset: PI,
                }],
            },
            step("sx", &[0]),
            RZ_PI,
        ],
        exactness: Exactness::UpToGlobalPhase,
        note: "Euler XZX form; the theta+pi offset means a symbolic angle cannot survive this rule",
    },
    DecompositionRule {
        id: "u-to-rz-sx",
        source: "u",
        source_arity: 1,
        source_params: 3,
        steps: &[
            RuleStep {
                op: "rz",
                operands: &[0],
                params: &[ParamTransform::passthrough(2)],
            },
            step("sx", &[0]),
            RuleStep {
                op: "rz",
                operands: &[0],
                params: &[Affine {
                    index: 0,
                    scale: 1.0,
                    offset: PI,
                }],
            },
            step("sx", &[0]),
            RuleStep {
                op: "rz",
                operands: &[0],
                params: &[Affine {
                    index: 1,
                    scale: 1.0,
                    offset: PI,
                }],
            },
        ],
        exactness: Exactness::UpToGlobalPhase,
        note: "the standard ZSXZSXZ form for an arbitrary one-qubit unitary",
    },
];
