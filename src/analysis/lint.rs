//! The static-analysis (lint) boundary.
//!
//! `final-deliverables-spec.md` §13.5 asks for "a modular analysis boundary
//! even if the initial lint rule set is small". This is that boundary: a
//! [`LintRule`] trait, a [`Linter`] that runs a set of rules, and five rules.
//! It is **not** a static-analysis research system, and nothing here should
//! be read as claiming one (§13.5, last line).
//!
//! # Lint reports; it never rewrites
//!
//! A rule only describes a circuit. It never changes one and never feeds a
//! pass, so a wrong diagnostic can mislead a reader but cannot change what a
//! program computes. Where a rule overlaps an optimization pass, it reuses
//! that pass's own predicate (`cancellable-pair` calls the cancellation
//! pass's matcher, `identity-operation` the canonicalizer's), so lint and the
//! optimizer agree on purpose rather than by coincidence.
//!
//! # The rules
//!
//! | Rule | Severity | Fires on |
//! |---|---|---|
//! | `cancellable-pair` | warning | an adjacent gate/inverse pair the gate-cancellation pass would remove |
//! | `identity-operation` | warning / info | `id`, a rotation by exactly zero, or by an exact multiple of the angle at which it is the identity (info when only up to global phase) |
//! | `unobservable-gate` | warning | in a circuit that measures, a gate no measured outcome depends on |
//! | `unused-resource` | info / warning | a qubit never touched or a classical bit never written (info); a classical bit written twice, losing the first outcome (warning) |
//! | `target-unsupported` | error / info | with a target: an operation, measurement pattern or reset the target cannot run (error), or a gate outside the basis that lowering will decompose (info) |

use std::collections::BTreeSet;
use std::f64::consts::TAU;
use std::sync::Arc;

use serde::Serialize;

use crate::ir::{Circuit, GateKind, Instruction, Param};
use crate::lowering::RuleSet;
use crate::target::{BasisProfile, Violation, check};

/// How much a diagnostic matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Worth knowing; not a problem.
    Info,
    /// Probably not what was meant.
    Warning,
    /// The circuit cannot run as asked.
    Error,
}

impl Severity {
    /// A stable identifier.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

/// One finding.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnostic {
    /// The rule that produced it.
    pub rule: String,
    /// How much it matters.
    pub severity: Severity,
    /// The program indices involved, ascending.
    pub instructions: Vec<usize>,
    /// The qubits involved, ascending.
    pub qubits: Vec<u32>,
    /// What was found, in a sentence.
    pub message: String,
}

/// What a rule may look at.
pub struct LintContext<'a> {
    /// The circuit being linted.
    pub circuit: &'a Circuit,
    /// The target it will be compiled for, when there is one.
    pub profile: Option<&'a BasisProfile>,
    /// That target's decomposition rules, when it has a valid set.
    pub rules: Option<&'a RuleSet>,
}

/// A lint rule.
///
/// Implementors must be deterministic and must not modify the circuit (they
/// cannot: they receive a shared reference).
pub trait LintRule: Send + Sync {
    /// Stable identifier, kebab-case.
    fn id(&self) -> &str;

    /// One-line description.
    fn description(&self) -> &str;

    /// The rule's findings on this circuit.
    fn check(&self, context: &LintContext<'_>) -> Vec<Diagnostic>;
}

/// The result of linting one circuit.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct LintReport {
    /// The rules that ran, in order.
    pub rules: Vec<String>,
    /// Every finding, sorted by first instruction, then rule id.
    pub diagnostics: Vec<Diagnostic>,
}

impl LintReport {
    /// How many findings have at least this severity.
    #[must_use]
    pub fn count_at_least(&self, severity: Severity) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity >= severity)
            .count()
    }
}

/// An ordered set of lint rules.
#[derive(Clone, Default)]
pub struct Linter {
    rules: Vec<Arc<dyn LintRule>>,
}

impl std::fmt::Debug for Linter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.rule_ids()).finish()
    }
}

impl Linter {
    /// An empty linter.
    #[must_use]
    pub fn new() -> Self {
        Linter::default()
    }

    /// The five built-in rules.
    #[must_use]
    pub fn default_rules() -> Self {
        let mut linter = Linter::new();
        linter
            .register(Arc::new(CancellablePair))
            .register(Arc::new(IdentityOperation))
            .register(Arc::new(UnobservableGate))
            .register(Arc::new(UnusedResource))
            .register(Arc::new(TargetUnsupported));
        linter
    }

    /// Appends a rule.
    pub fn register(&mut self, rule: Arc<dyn LintRule>) -> &mut Self {
        self.rules.push(rule);
        self
    }

    /// The ids of every rule, in order.
    #[must_use]
    pub fn rule_ids(&self) -> Vec<&str> {
        self.rules.iter().map(|r| r.id()).collect()
    }

    /// Runs every rule. Rules that need a target are silent without one.
    #[must_use]
    pub fn lint(&self, circuit: &Circuit, profile: Option<&BasisProfile>) -> LintReport {
        let rules = profile.and_then(|p| RuleSet::new(p).ok());
        let context = LintContext {
            circuit,
            profile,
            rules: rules.as_ref(),
        };
        let mut diagnostics: Vec<Diagnostic> = self
            .rules
            .iter()
            .flat_map(|rule| rule.check(&context))
            .collect();
        diagnostics.sort_by(|a, b| {
            (a.instructions.first(), &a.rule, &a.instructions).cmp(&(
                b.instructions.first(),
                &b.rule,
                &b.instructions,
            ))
        });
        LintReport {
            rules: self.rule_ids().into_iter().map(str::to_string).collect(),
            diagnostics,
        }
    }
}

/// The mnemonic of an instruction, for messages.
fn name_of(inst: &Instruction) -> String {
    match inst {
        Instruction::Gate { kind, .. } => kind.mnemonic().to_string(),
        Instruction::Measure { .. } => "measure".to_string(),
        Instruction::Reset { .. } => "reset".to_string(),
    }
}

fn diagnostic(
    rule: &str,
    severity: Severity,
    circuit: &Circuit,
    mut instructions: Vec<usize>,
    message: String,
) -> Diagnostic {
    instructions.sort_unstable();
    let qubits: BTreeSet<u32> = instructions
        .iter()
        .filter_map(|i| circuit.instructions().get(*i))
        .flat_map(|inst| inst.qubits().into_iter().map(|q| q.0))
        .collect();
    Diagnostic {
        rule: rule.to_string(),
        severity,
        instructions,
        qubits: qubits.into_iter().collect(),
        message,
    }
}

// --- cancellable-pair --------------------------------------------------------

/// Adjacent gate/inverse pairs, exactly as the gate-cancellation pass finds them.
pub struct CancellablePair;

impl LintRule for CancellablePair {
    fn id(&self) -> &str {
        "cancellable-pair"
    }

    fn description(&self) -> &str {
        "an adjacent gate/inverse pair the gate-cancellation pass would remove"
    }

    fn check(&self, context: &LintContext<'_>) -> Vec<Diagnostic> {
        let Ok(doomed) = crate::pass::cancellation::find_cancellable(context.circuit) else {
            return Vec::new();
        };
        let Ok(successors) = crate::pass::adjacency::unanimous_successors(context.circuit) else {
            return Vec::new();
        };
        let mut indices: Vec<usize> = doomed.iter().copied().collect();
        indices.sort_unstable();
        indices
            .into_iter()
            .filter_map(|first| {
                let second = *successors.get(&first)?;
                doomed.contains(&second).then_some((first, second))
            })
            .map(|(first, second)| {
                let name = name_of(&context.circuit.instructions()[first]);
                diagnostic(
                    self.id(),
                    Severity::Warning,
                    context.circuit,
                    vec![first, second],
                    format!(
                        "instructions {first} and {second} (`{name}` and its inverse) cancel; \
                         nothing between them touches their qubits"
                    ),
                )
            })
            .collect()
    }
}

// --- identity-operation ------------------------------------------------------

/// Operations that provably do nothing.
pub struct IdentityOperation;

/// An exact multiple of `period`, with no tolerance: `fract` of an exact
/// quotient is exactly zero only when the angle really is a multiple.
fn exact_multiple(radians: f64, period: f64) -> Option<i64> {
    let quotient = radians / period;
    #[allow(clippy::cast_possible_truncation)]
    (quotient.fract() == 0.0 && quotient != 0.0).then_some(quotient as i64)
}

impl LintRule for IdentityOperation {
    fn id(&self) -> &str {
        "identity-operation"
    }

    fn description(&self) -> &str {
        "an operation that is the identity (exactly, or up to global phase)"
    }

    fn check(&self, context: &LintContext<'_>) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for (index, inst) in context.circuit.instructions().iter().enumerate() {
            let Instruction::Gate { kind, .. } = inst else {
                continue;
            };
            let concrete = |p: &Param| p.as_concrete().map(crate::ir::Angle::radians);
            let finding = if crate::pass::canonicalize::is_identity(inst) {
                Some((Severity::Warning, "is exactly the identity".to_string()))
            } else {
                match kind {
                    GateKind::P(p) => concrete(p).and_then(|a| exact_multiple(a, TAU)).map(|_| {
                        (
                            Severity::Warning,
                            "is exactly the identity (angle 2πk)".to_string(),
                        )
                    }),
                    GateKind::Rx(p) | GateKind::Ry(p) | GateKind::Rz(p) => {
                        concrete(p).and_then(|a| {
                            exact_multiple(a, TAU).map(|k| {
                                if k % 2 == 0 {
                                    (
                                        Severity::Warning,
                                        "is exactly the identity (angle 4πk)".to_string(),
                                    )
                                } else {
                                    (
                                        Severity::Info,
                                        "is −I, the identity up to global phase (angle 2π(2k+1)); \
                                         removing it is safe only because no OQCI construct \
                                         applies a fragment under control"
                                            .to_string(),
                                    )
                                }
                            })
                        })
                    }
                    _ => None,
                }
            };
            if let Some((severity, what)) = finding {
                out.push(diagnostic(
                    self.id(),
                    severity,
                    context.circuit,
                    vec![index],
                    format!("instruction {index} (`{}`) {what}", kind.mnemonic()),
                ));
            }
        }
        out
    }
}

// --- unobservable-after-measurement ------------------------------------------

/// Gates that no measured outcome depends on.
///
/// A backward liveness walk: a measurement makes its qubit live; a gate is
/// live if any operand is live, and then makes all its operands live (it may
/// entangle them); a reset ends a qubit's liveness. A gate whose operands are
/// all dead when it is reached cannot affect any measurement — local
/// operations cannot signal — so it is reported. Only circuits that measure
/// at all are checked: a circuit with no measurement may be a subroutine
/// whose final state is the point.
pub struct UnobservableGate;

impl LintRule for UnobservableGate {
    fn id(&self) -> &str {
        "unobservable-gate"
    }

    fn description(&self) -> &str {
        "a gate that cannot influence any measured outcome"
    }

    fn check(&self, context: &LintContext<'_>) -> Vec<Diagnostic> {
        let instructions = context.circuit.instructions();
        if !instructions
            .iter()
            .any(|i| matches!(i, Instruction::Measure { .. }))
        {
            return Vec::new();
        }
        let mut live: BTreeSet<u32> = BTreeSet::new();
        let mut dead = Vec::new();
        for (index, inst) in instructions.iter().enumerate().rev() {
            match inst {
                Instruction::Measure { qubit, .. } => {
                    live.insert(qubit.0);
                }
                Instruction::Reset { qubit } => {
                    live.remove(&qubit.0);
                }
                Instruction::Gate { qubits, .. } => {
                    if qubits.iter().any(|q| live.contains(&q.0)) {
                        live.extend(qubits.iter().map(|q| q.0));
                    } else {
                        dead.push(index);
                    }
                }
            }
        }
        dead.reverse();
        dead.into_iter()
            .map(|index| {
                diagnostic(
                    self.id(),
                    Severity::Warning,
                    context.circuit,
                    vec![index],
                    format!(
                        "instruction {index} (`{}`) cannot influence any measured outcome: no \
                         later measurement depends on its qubits",
                        name_of(&instructions[index])
                    ),
                )
            })
            .collect()
    }
}

// --- unused-resource ---------------------------------------------------------

/// Declared resources that are never used, and classical bits written twice.
pub struct UnusedResource;

impl LintRule for UnusedResource {
    fn id(&self) -> &str {
        "unused-resource"
    }

    fn description(&self) -> &str {
        "a qubit never touched, a classical bit never written, or one written twice"
    }

    fn check(&self, context: &LintContext<'_>) -> Vec<Diagnostic> {
        let circuit = context.circuit;
        let mut touched = vec![false; circuit.num_qubits() as usize];
        let mut writes: Vec<Vec<usize>> = vec![Vec::new(); circuit.num_clbits() as usize];
        for (index, inst) in circuit.instructions().iter().enumerate() {
            for qubit in inst.qubits() {
                touched[qubit.0 as usize] = true;
            }
            if let Instruction::Measure { target, .. } = inst {
                writes[target.0 as usize].push(index);
            }
        }
        let mut out = Vec::new();
        for (qubit, used) in touched.iter().enumerate() {
            if !used {
                out.push(Diagnostic {
                    rule: self.id().to_string(),
                    severity: Severity::Info,
                    instructions: Vec::new(),
                    qubits: vec![u32::try_from(qubit).unwrap_or(u32::MAX)],
                    message: format!("qubit {qubit} is declared but never used"),
                });
            }
        }
        for (clbit, written) in writes.iter().enumerate() {
            match written.len() {
                0 => out.push(Diagnostic {
                    rule: self.id().to_string(),
                    severity: Severity::Info,
                    instructions: Vec::new(),
                    qubits: Vec::new(),
                    message: format!("classical bit {clbit} is declared but never written"),
                }),
                1 => {}
                _ => out.push(diagnostic(
                    self.id(),
                    Severity::Warning,
                    circuit,
                    written.clone(),
                    format!(
                        "classical bit {clbit} is written {} times; only the last outcome \
                         survives, since classical bits are write-only",
                        written.len()
                    ),
                )),
            }
        }
        out
    }
}

// --- target-unsupported ------------------------------------------------------

/// What the selected target cannot run, before lowering is attempted.
pub struct TargetUnsupported;

impl LintRule for TargetUnsupported {
    fn id(&self) -> &str {
        "target-unsupported"
    }

    fn description(&self) -> &str {
        "something the selected target cannot run, or will run only after decomposition"
    }

    fn check(&self, context: &LintContext<'_>) -> Vec<Diagnostic> {
        let Some(profile) = context.profile else {
            return Vec::new();
        };
        let circuit = context.circuit;
        let target = profile.qualified_id();
        check(circuit, profile)
            .violations
            .into_iter()
            .filter_map(|violation| {
                let (severity, index, message) = match violation {
                    Violation::UnsupportedOperation { index, mnemonic } => {
                        match context.rules.and_then(|rules| rules.rule_for(&mnemonic)) {
                            Some(rule) => (
                                Severity::Info,
                                index,
                                format!(
                                    "`{mnemonic}` is outside `{target}`'s basis and will be \
                                     lowered via `{}`",
                                    rule.id
                                ),
                            ),
                            None => (
                                Severity::Error,
                                index,
                                format!(
                                    "`{mnemonic}` is outside `{target}`'s basis and no \
                                     decomposition rule reaches it"
                                ),
                            ),
                        }
                    }
                    Violation::QubitOutOfRange {
                        index, qubit_count, ..
                    } => (
                        Severity::Error,
                        index,
                        format!("the circuit is wider than `{target}`'s {qubit_count} qubits"),
                    ),
                    Violation::ParameterOutOfRange {
                        index,
                        mnemonic,
                        value,
                        min,
                        max,
                    } => (
                        Severity::Error,
                        index,
                        format!(
                            "`{mnemonic}` parameter {value} is outside `{target}`'s domain \
                             [{min}, {max}]"
                        ),
                    ),
                    Violation::MeasurementUnsupported { index } => {
                        (Severity::Error, index, format!("`{target}` cannot measure"))
                    }
                    Violation::MidCircuitMeasurementUnsupported { index, qubit } => (
                        Severity::Error,
                        index,
                        format!(
                            "`{target}` measures only at the end, but qubit {} is used again \
                             after this measurement",
                            qubit.0
                        ),
                    ),
                    Violation::ResetUnsupported { index } => {
                        (Severity::Error, index, format!("`{target}` cannot reset"))
                    }
                    // Routing fixes connectivity, and an unbound parameter is
                    // a binding problem reported elsewhere, not a target one.
                    Violation::ConnectivityViolation { .. }
                    | Violation::UnboundParameter { .. } => {
                        return None;
                    }
                };
                Some(diagnostic(
                    self.id(),
                    severity,
                    circuit,
                    vec![index],
                    message,
                ))
            })
            .collect()
    }
}
