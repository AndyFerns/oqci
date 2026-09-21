//! Target lowering: turning a target-independent circuit into one a specific
//! device will actually accept.
//!
//! [`crate::target`] *describes* a backend. This module *applies* that
//! description. Stage D §4 keeps the two apart deliberately — "abstract IR,
//! basis profile, target lowering, routing/mapping … must not be conflated" —
//! and the split shows up in behaviour: [`crate::target::check`] reports
//! problems and repairs nothing, while everything here exists to repair them.
//!
//! # The schedule
//!
//! ```text
//! D0  arity reduction     every gate -> at most two qubits
//! L   layout              logical -> physical, injective
//! R   routing             insert Swaps; program order; insertion-only
//! D1  basis decomposition rewrite everything outside the basis
//! O   orientation repair  single sweep over reversed two-qubit gates
//! D2  single-qubit cleanup
//! V   verify              check() plus what check() cannot see
//! ```
//!
//! Two orderings in there are load-bearing.
//!
//! **D0 comes before layout.** Routing only knows how to make *pairs*
//! adjacent; a three-qubit gate has no meaning on a coupling map. Worse,
//! [`crate::target::check`] skips connectivity for any operation that is not
//! exactly two qubits, so a surviving `Ccx` would be *reported legal* — a
//! complete path from a valid input to a "verified" output no device can run.
//! Reducing arity first closes it, and gives layout a real two-qubit
//! interaction graph to work with.
//!
//! **O is separate from D1 and D2, and there is no outer loop.** Putting
//! orientation repair inside decomposition creates an apparent cycle:
//! `swap -> cx`, `cx -> h` when reversed, `h -> rz, sx` — with `cx`
//! re-entering through orientation. That cycle is an artifact of conflating
//! two different measures. Decomposition reduces a gate's *mnemonic* toward
//! the basis; orientation repair reduces its *operand order*. Separated, each
//! terminates for its own reason:
//!
//! - D0, D1 and D2 terminate because [`RuleSet::new`] proved the rule graph
//!   acyclic, so every rewrite strictly decreases a well-founded rank.
//! - O terminates because it is a single sweep. The two-qubit gate it emits
//!   is natively oriented by construction and can never need repair again.
//! - D2 cannot send us back to O, because no one-qubit rule reaches a
//!   two-qubit operation — a property [`RuleSet::new`] checks rather than
//!   assumes.
//!
//! # The invariant chain
//!
//! What makes the output legal, rather than merely finished:
//!
//! - **I1**, after D0: every gate has arity at most two.
//! - **I2**, after R: every two-qubit gate sits on a coupled pair.
//! - **I3**, always: a decomposition rule may only permute the operands it
//!   was given, never name a new qubit. Enforced when the rule set is built.
//! - I2 and I3 together mean **I2 survives D1, O and D2**: no rewrite can
//!   move a gate onto an uncoupled pair.
//! - **I4**, after O and D2: every two-qubit gate is natively oriented and
//!   every mnemonic is in the basis.
//!
//! I3 also buys measurement terminality for free: since rules preserve
//! operand sets, "is this wire touched after instruction *n*" has the same
//! answer before and after decomposition.
//!
//! # Lowering is not a `Pass`
//!
//! A [`crate::pass::Pass`] is `Circuit -> Circuit`, and
//! `tests/pass_equivalence.rs` asserts that a pass leaves the state vector
//! alone up to global phase. That is a *false* specification for lowering,
//! whose output is deliberately a permutation of the input over a wider
//! register. Were lowering a `Pass`, someone would eventually register it in
//! `PassManager::default_pipeline` and the property suite would start
//! asserting something untrue about it. So lowering has its own entry point
//! and its own equivalence statement — see `tests/lowering_equivalence.rs`.

pub mod decompose;
pub mod layout;
pub mod routing;
pub mod rules;

use serde::Serialize;

use crate::ir::{Circuit, Instruction, IrError, Param, QubitId};
use crate::target::{BasisProfile, LegalityReport, PhysicalQubit, Violation, check};

pub use decompose::Scope;
pub use layout::{DenseLayout, Layout, LayoutError, LayoutStrategy, TrivialLayout};
pub use routing::{OrientationPolicy, RoutingStrategy, ShortestPathRouter};
pub use rules::{
    DecompositionRule, Exactness, ParamTransform, RuleError, RuleSet, RuleSetError, RuleStep,
};

/// Why no sequence of SWAPs could make two qubits adjacent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnroutableReason {
    /// The qubits are in different components of the routing graph. No amount
    /// of routing can fix this.
    NoPath,
    /// Every path runs through a wire that has already been measured, on a
    /// device that cannot measure mid-circuit.
    MeasurementFrozenWire,
}

impl std::fmt::Display for UnroutableReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnroutableReason::NoPath => f.write_str("no path in the routing graph"),
            UnroutableReason::MeasurementFrozenWire => {
                f.write_str("every path crosses a measured wire")
            }
        }
    }
}

/// Why a circuit could not be lowered onto a target.
///
/// Every variant is a *refusal*, not a failure to try. Lowering either
/// returns a circuit the target model itself certifies as legal, or one of
/// these naming exactly what it could not fix. There is deliberately no third
/// outcome — in particular no "best effort" circuit that might not run.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum LoweringError {
    /// The target's rule set is malformed, or cannot reach its own basis.
    #[error("target `{target}` has an unusable rule set: {source}")]
    RuleSet {
        /// The profile at fault.
        target: String,
        /// What is wrong with it.
        #[source]
        source: RuleSetError,
    },
    /// A layout could not be chosen.
    #[error(transparent)]
    Layout {
        /// The underlying layout problem.
        #[from]
        source: LayoutError,
    },
    /// A rule refused to apply.
    #[error(transparent)]
    Rule {
        /// The underlying rule problem.
        #[from]
        source: RuleError,
    },
    /// A rewritten circuit failed QC-IR validation.
    #[error("lowering produced an invalid circuit: {source}")]
    Ir {
        /// The invariant that was broken.
        #[source]
        source: IrError,
    },
    /// An operation is outside the basis and the target has no rule for it.
    #[error("target `{target}` neither supports `{mnemonic}` nor can decompose it")]
    NoDecompositionRule {
        /// The stranded operation.
        mnemonic: String,
        /// The target, with its version.
        target: String,
    },
    /// An opaque operation the target does not natively support.
    #[error(
        "target `{target}` does not support opaque operation `{name}`, and its matrix is unknown"
    )]
    OpaqueOperation {
        /// The opaque gate's name.
        name: String,
        /// The target.
        target: String,
    },
    /// Two operands could not be brought together.
    #[error("instruction {index}: cannot route {from} to {to} ({reason})")]
    Unroutable {
        /// Program index of the operation.
        index: usize,
        /// First operand's physical qubit.
        from: PhysicalQubit,
        /// Second operand's physical qubit.
        to: PhysicalQubit,
        /// Which of the two ways this happens.
        reason: UnroutableReason,
    },
    /// A two-qubit operation faces the wrong way down a one-way edge, and has
    /// no verified reversal.
    #[error(
        "instruction {index}: `{mnemonic}` faces the wrong way and cannot be reversed: {detail}"
    )]
    UnrepairableOrientation {
        /// Program index of the operation.
        index: usize,
        /// The operation.
        mnemonic: String,
        /// Why no repair applies.
        detail: String,
    },
    /// The device cannot measure, or cannot reset, and the circuit does.
    #[error("target `{target}` does not support `{operation}` (instruction {index})")]
    UnsupportedClassicalOperation {
        /// Program index.
        index: usize,
        /// `"measure"` or `"reset"`.
        operation: &'static str,
        /// The target.
        target: String,
    },
    /// The input program already measures mid-circuit, before routing had any
    /// say in it.
    ///
    /// Distinct from [`LoweringError::Unroutable`] with
    /// [`UnroutableReason::MeasurementFrozenWire`] on purpose: this is a
    /// property of the program as written, which no layout or routing choice
    /// could have avoided.
    #[error(
        "target `{target}` cannot measure mid-circuit, and {qubit} is used after instruction {index}"
    )]
    InputMeasuresMidCircuit {
        /// Program index of the measurement.
        index: usize,
        /// The qubit that keeps being used.
        qubit: QubitId,
        /// The target.
        target: String,
    },
    /// A logical qubit had no place on the device.
    #[error("instruction {index}: logical qubit {qubit} is not in the layout")]
    UnmappedQubit {
        /// Program index.
        index: usize,
        /// The unmapped qubit.
        qubit: QubitId,
    },
    /// Rewriting did not finish within its depth guard.
    ///
    /// Should be unreachable: [`RuleSet::new`] proves the rule graph acyclic,
    /// which makes termination a theorem rather than a hope. Reaching this
    /// means that validation has a hole, and it is reported rather than
    /// silently emitting a half-lowered circuit.
    #[error("decomposing `{mnemonic}` exceeded depth {depth}; rule-set validation has a gap")]
    DecompositionDidNotConverge {
        /// The operation being rewritten.
        mnemonic: String,
        /// The depth reached.
        depth: usize,
    },
    /// Lowering finished, but its own output does not satisfy the target.
    ///
    /// The backstop. Lowering re-runs [`crate::target::check`] on what it
    /// produced, so a bug anywhere above surfaces here as a refusal rather
    /// than as a circuit that looks fine and is not.
    #[error("lowering finished but its output is still illegal: {violations:?}")]
    VerificationFailed {
        /// Everything still wrong with the result.
        violations: Vec<Violation>,
    },
    /// A symbolic parameter reached an operation whose lowering would have
    /// had to transform it.
    #[error(
        "parameter `{symbol}` is still symbolic on `{mnemonic}`; bind parameters before lowering"
    )]
    UnboundParameter {
        /// The operation.
        mnemonic: String,
        /// The unbound symbol.
        symbol: String,
    },
}

/// How lowering should be performed.
#[derive(Debug, Clone)]
pub struct LoweringConfig {
    /// Which layout strategy to use.
    pub layout: LayoutChoice,
    /// Whether to insert SWAPs for non-local operations.
    ///
    /// Turning this off is an inspection aid, not a compilation mode: the
    /// result will generally not be legal, and the legality report says so
    /// rather than the function pretending otherwise.
    pub route: bool,
    /// Whether to rewrite operations outside the basis.
    pub decompose: bool,
}

impl Default for LoweringConfig {
    fn default() -> Self {
        LoweringConfig {
            layout: LayoutChoice::Trivial,
            route: true,
            decompose: true,
        }
    }
}

/// Which layout to use.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum LayoutChoice {
    /// Logical `n` on physical `n`.
    Trivial,
    /// Seat interacting qubits near each other.
    Dense,
    /// A layout supplied by the caller.
    Explicit(Layout),
}

impl LayoutChoice {
    /// This choice's stable identifier, for reporting.
    #[must_use]
    pub fn id(&self) -> &'static str {
        match self {
            LayoutChoice::Trivial => TrivialLayout.id(),
            LayoutChoice::Dense => DenseLayout.id(),
            LayoutChoice::Explicit(_) => "explicit",
        }
    }
}

/// One step of the lowering schedule, and what it did.
#[derive(Debug, Clone, Serialize)]
pub struct LoweringStep {
    /// Stable step identifier.
    pub id: &'static str,
    /// Operations after this step.
    pub op_count: usize,
    /// One-line summary.
    pub detail: String,
}

/// A circuit lowered onto a specific target.
#[derive(Debug, Clone)]
pub struct Lowered {
    /// The result, over **physical** qubit indices.
    pub circuit: Circuit,
    /// Where each logical qubit started.
    pub initial_layout: Layout,
    /// Where each logical qubit ended, after routing.
    pub final_layout: Layout,
    /// SWAPs inserted — §8.7's routing overhead.
    pub swaps_inserted: usize,
    /// Reversed two-qubit operations repaired.
    pub orientations_repaired: usize,
    /// Decomposition rules that fired, ascending.
    pub rules_applied: Vec<String>,
    /// The schedule, step by step.
    pub steps: Vec<LoweringStep>,
    /// The target, as `id@version`.
    pub profile_id: String,
    /// The legality report for the output.
    pub legality: LegalityReport,
}

/// Lowers a circuit onto a target.
///
/// # What success guarantees
///
/// The returned circuit is one [`crate::target::check`] reports as legal,
/// *and* satisfies what `check` cannot see: every gate has at most two
/// operands, and no symbolic parameter survives on an operation whose
/// lowering would have had to transform it. If any of that fails, so does
/// this function.
///
/// A symbolic parameter on a *transparent* path is not a failure — `rz(theta)`
/// on an `rz`-native target is a parameterized circuit awaiting binding, and
/// it is reported in [`Lowered::legality`] rather than refused.
///
/// # Errors
///
/// Any [`LoweringError`]. Each one names what could not be done; none leaves
/// a partially-lowered circuit in the caller's hands.
pub fn lower(
    circuit: &Circuit,
    profile: &BasisProfile,
    config: &LoweringConfig,
) -> Result<Lowered, LoweringError> {
    let rules = RuleSet::new(profile).map_err(|source| LoweringError::RuleSet {
        target: profile.qualified_id(),
        source,
    })?;
    let mut steps = Vec::new();

    reject_unsupported_classical(circuit, profile)?;

    // --- D0: arity reduction, before anything consults the coupling map ---
    let mut instructions = circuit.instructions().to_vec();
    let mut rules_applied = std::collections::BTreeSet::new();
    if config.decompose {
        let reduced = decompose::decompose(&instructions, profile, &rules, Scope::ArityOnly)?;
        rules_applied.extend(reduced.rules_applied.iter().cloned());
        steps.push(LoweringStep {
            id: "arity-reduction",
            op_count: reduced.instructions.len(),
            detail: format!("{} wide gate(s) reduced", reduced.rewritten),
        });
        instructions = reduced.instructions;
    }
    let reduced_circuit = decompose::rebuild(circuit, instructions)?;

    // --- L: layout ---
    let topology = profile.topology();
    let initial = match &config.layout {
        LayoutChoice::Trivial => TrivialLayout.plan(&reduced_circuit, topology)?,
        LayoutChoice::Dense => DenseLayout.plan(&reduced_circuit, topology)?,
        LayoutChoice::Explicit(layout) => {
            // A caller-supplied layout is the one path that bypasses the
            // strategies' own fit check, so it is validated here instead.
            // Without this, a layout shorter than the circuit surfaces much
            // later as `UnmappedQubit` against whichever instruction happened
            // to touch the missing qubit first.
            if (layout.len() as u32) < reduced_circuit.num_qubits() {
                return Err(LoweringError::Layout {
                    source: LayoutError::DeviceTooSmall {
                        needed: reduced_circuit.num_qubits(),
                        available: layout.len() as u32,
                    },
                });
            }
            layout.clone()
        }
    };
    steps.push(LoweringStep {
        id: "layout",
        op_count: reduced_circuit.len(),
        detail: format!("{} layout: {initial}", config.layout.id()),
    });

    // --- R: routing ---
    // Whether a one-way edge is usable depends on whether a reversed
    // interaction can be *expressed* on this target, which is a question
    // about the rule closure rather than the basis list. `linear-nisq` does
    // not list `h`, but reaches it through `h-to-rz-sx`; deciding from
    // `supports_operation("h")` would make it refuse every directed edge —
    // a false negative that looks like caution and is a bug.
    let can_reverse = reaches(profile, &rules, "h");
    let routed = if config.route {
        ShortestPathRouter.route(&reduced_circuit, initial.clone(), profile, can_reverse)?
    } else {
        routing::RoutingOutput {
            instructions: reduced_circuit.instructions().to_vec(),
            initial_layout: initial.clone(),
            final_layout: initial,
            swaps_inserted: 0,
        }
    };
    steps.push(LoweringStep {
        id: "routing",
        op_count: routed.instructions.len(),
        detail: if config.route {
            format!("{} swap(s) inserted", routed.swaps_inserted)
        } else {
            "skipped".to_string()
        },
    });

    let width = physical_width(&routed.instructions, &routed.final_layout);
    let mut current = decompose::rebuild_with_width(circuit, routed.instructions, width)?;
    let mut orientations_repaired = 0;

    if config.decompose {
        // --- D1: basis decomposition ---
        let expanded = decompose::decompose(current.instructions(), profile, &rules, Scope::Basis)?;
        rules_applied.extend(expanded.rules_applied.iter().cloned());
        steps.push(LoweringStep {
            id: "basis-decomposition",
            op_count: expanded.instructions.len(),
            detail: format!("{} operation(s) rewritten", expanded.rewritten),
        });
        current = decompose::rebuild_with_width(&current, expanded.instructions, width)?;

        // --- O: orientation repair, one sweep ---
        let (repaired, count) = routing::repair_orientation(current.instructions(), profile)?;
        orientations_repaired = count;
        steps.push(LoweringStep {
            id: "orientation-repair",
            op_count: repaired.len(),
            detail: format!("{count} reversed operation(s) repaired"),
        });
        current = decompose::rebuild_with_width(&current, repaired, width)?;

        // --- D2: single-qubit cleanup, for what O just emitted ---
        let cleaned = decompose::decompose(
            current.instructions(),
            profile,
            &rules,
            Scope::SingleQubitOnly,
        )?;
        rules_applied.extend(cleaned.rules_applied.iter().cloned());
        steps.push(LoweringStep {
            id: "single-qubit-cleanup",
            op_count: cleaned.instructions.len(),
            detail: format!("{} operation(s) rewritten", cleaned.rewritten),
        });
        current = decompose::rebuild_with_width(&current, cleaned.instructions, width)?;
    }

    // --- V: verify ---
    let legality = verify(&current, profile, &rules, config)?;
    steps.push(LoweringStep {
        id: "verify",
        op_count: current.len(),
        detail: if legality.is_legal() {
            "legal".to_string()
        } else {
            format!("{} violation(s)", legality.violation_count())
        },
    });

    Ok(Lowered {
        circuit: current,
        initial_layout: routed.initial_layout,
        final_layout: routed.final_layout,
        swaps_inserted: routed.swaps_inserted,
        orientations_repaired,
        rules_applied: rules_applied.into_iter().collect(),
        steps,
        profile_id: profile.qualified_id(),
        legality,
    })
}

/// The final check, plus the assertions `check` is structurally unable to
/// make.
fn verify(
    circuit: &Circuit,
    profile: &BasisProfile,
    rules: &RuleSet,
    config: &LoweringConfig,
) -> Result<LegalityReport, LoweringError> {
    let fully_lowered = config.route && config.decompose;

    for (index, instruction) in circuit.instructions().iter().enumerate() {
        let Instruction::Gate { kind, qubits } = instruction else {
            continue;
        };
        // `check` skips connectivity for anything that is not exactly two
        // qubits, so a surviving wide gate would be reported legal.
        if qubits.len() > 2 && config.decompose {
            return Err(LoweringError::VerificationFailed {
                violations: vec![Violation::UnsupportedOperation {
                    index,
                    mnemonic: format!("{} (arity {})", kind.mnemonic(), qubits.len()),
                }],
            });
        }
        // A symbolic parameter is fine only where nothing had to transform
        // it. Reported as its own error because `check` says "unbound"
        // without saying what to do about it.
        for param in kind.params() {
            if let Param::Symbol(symbol) = param
                && !rules.is_parameter_transparent(kind.mnemonic())
            {
                return Err(LoweringError::UnboundParameter {
                    mnemonic: kind.mnemonic().to_string(),
                    symbol,
                });
            }
        }
    }

    let report = check(circuit, profile);
    // An unbound parameter on a transparent path is a legitimate state for a
    // parameterized circuit, so it is reported but does not fail lowering.
    let blocking: Vec<Violation> = report
        .violations
        .iter()
        .filter(|violation| !matches!(violation, Violation::UnboundParameter { .. }))
        .cloned()
        .collect();

    if !blocking.is_empty() && fully_lowered {
        return Err(LoweringError::VerificationFailed {
            violations: blocking,
        });
    }
    Ok(report)
}

/// Rejects measurement and reset the device cannot perform.
///
/// Checked up front against the *input*, so the diagnostic names the program
/// as written rather than an instruction that only became a problem after
/// routing moved things around.
fn reject_unsupported_classical(
    circuit: &Circuit,
    profile: &BasisProfile,
) -> Result<(), LoweringError> {
    let support = profile.measurement();
    for (index, instruction) in circuit.instructions().iter().enumerate() {
        match instruction {
            Instruction::Measure { qubit, .. } => {
                if !support.measurement {
                    return Err(LoweringError::UnsupportedClassicalOperation {
                        index,
                        operation: "measure",
                        target: profile.qualified_id(),
                    });
                }
                if !support.mid_circuit_measurement
                    && circuit
                        .instructions()
                        .iter()
                        .skip(index + 1)
                        .any(|later| later.qubits().contains(qubit))
                {
                    return Err(LoweringError::InputMeasuresMidCircuit {
                        index,
                        qubit: *qubit,
                        target: profile.qualified_id(),
                    });
                }
            }
            Instruction::Reset { .. } if !support.reset => {
                return Err(LoweringError::UnsupportedClassicalOperation {
                    index,
                    operation: "reset",
                    target: profile.qualified_id(),
                });
            }
            _ => {}
        }
    }
    Ok(())
}

/// Whether a mnemonic is native, or reachable through the rule closure.
///
/// The right test for "can this target express `h`?", as opposed to
/// `supports_operation`, which only answers "is `h` in the basis?".
fn reaches(profile: &BasisProfile, rules: &RuleSet, mnemonic: &str) -> bool {
    profile.supports_operation(mnemonic) || rules.rule_for(mnemonic).is_some()
}

/// How wide the physical register has to be.
///
/// Sized from the highest physical qubit actually used, not from the device's
/// full width: `ideal-simulator` declares 32 qubits, and a two-qubit program
/// should not come back as a 32-qubit circuit that no state-vector simulator
/// could touch.
fn physical_width(instructions: &[Instruction], layout: &Layout) -> u32 {
    let from_instructions = instructions
        .iter()
        .flat_map(Instruction::qubits)
        .map(|q| q.index() + 1)
        .max()
        .unwrap_or(0);
    let from_layout = layout.highest_occupied().map_or(0, |p| p.index() + 1);
    from_instructions.max(from_layout)
}
