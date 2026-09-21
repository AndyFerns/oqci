//! The pass manager and the optimization passes it runs.
//!
//! `final-deliverables-spec.md` §7 requires explicit pass registration and
//! ordering, enable/disable capability, deterministic execution, per-pass
//! metadata, error propagation, and the ability to run an ablation with
//! selected passes disabled. This module provides all of that, and treats
//! pass *ordering* as meaningful rather than assuming that running everything
//! is automatically best.
//!
//! # Why passes take and return [`Circuit`], not [`crate::ir::QcoCircuit`]
//!
//! QCO-IR is the representation that *exposes* dependency structure, and
//! passes do consult it — but its mutation methods are crate-internal by
//! design (Stage A §4: "construction remains centralized through
//! `CircuitBuilder`"). Rather than widening that hole for passes, a [`Pass`]
//! is a `Circuit → Circuit` function: one that needs true adjacency calls
//! [`crate::ir::qc_to_qco`] internally, analyses the graph, then replays the
//! result through a fresh [`crate::ir::CircuitBuilder`].
//!
//! That replay is load-bearing. Because every pass's output goes through
//! `CircuitBuilder::build`, a pass **cannot** emit a circuit that violates a
//! QC-IR invariant — it gets an [`crate::ir::IrError`] instead, surfaced as
//! [`PassError::Ir`]. A buggy pass fails loudly rather than corrupting the IR
//! for everything downstream.
//!
//! # Safety of the transformations
//!
//! Each pass is deliberately conservative, and each documents exactly what it
//! will *not* do. The governing rule (§33.14) is that no operation may be
//! optimized away across a measurement/reset barrier without proving the
//! transformation safe — which the QCO-IR's [`crate::ir::DepKind::Control`]
//! edges make mechanical rather than a matter of care.
//!
//! See `docs/pass_manager.md`, and `tests/pass_equivalence.rs` for the
//! property-based check that these rewrites preserve circuit semantics.

pub(crate) mod adjacency;
pub mod cancellation;
pub mod canonicalize;
pub mod rotation_merge;
pub mod schedule;

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::analysis::{ResourceReport, analyze};
use crate::ir::{Circuit, IrError};
use crate::target::{BasisProfile, CostModel};

pub use cancellation::GateCancellation;
pub use canonicalize::Canonicalize;
pub use rotation_merge::RotationMerge;
pub use schedule::Schedule;

/// Anything that can go wrong running a pass.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum PassError {
    /// The circuit a pass produced failed QC-IR validation, or an internal IR
    /// operation failed. Either way the pass is at fault, not the input.
    #[error("pass `{pass}` produced an invalid circuit: {source}")]
    Ir {
        /// The pass that failed.
        pass: String,
        /// The underlying validation failure.
        #[source]
        source: IrError,
    },
}

impl PassError {
    fn from_ir(pass: &str, source: IrError) -> Self {
        PassError::Ir {
            pass: pass.to_string(),
            source,
        }
    }
}

/// What a pass produced.
#[derive(Debug, Clone)]
pub struct PassOutput {
    /// The resulting circuit — the input unchanged if the pass did nothing.
    pub circuit: Circuit,
    /// Whether the pass modified anything. Analysis passes always report
    /// `false`.
    pub changed: bool,
    /// Human-readable detail about what the pass did or measured, e.g.
    /// `"cancelled 2 pair(s)"`. Shown verbatim by the CLI.
    pub notes: Vec<String>,
}

impl PassOutput {
    /// A no-op result: the circuit is returned untouched.
    #[must_use]
    pub fn unchanged(circuit: Circuit) -> Self {
        PassOutput {
            circuit,
            changed: false,
            notes: Vec::new(),
        }
    }

    /// Attaches a note.
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

/// What a pass may consult about the target it is compiling for.
///
/// Stage E exit criterion 3 requires that "optimization can consult
/// backend-defined costs", and Stage E §8 immediately qualifies it: target
/// awareness "must not make every pass backend-specific". A context every
/// pass receives and most passes ignore is the shape that takes — generic
/// transformations stay generic, and a pass that genuinely needs target data
/// has somewhere to get it from.
///
/// Both fields are optional because target-independent compilation is a
/// first-class mode, not a degraded one. A pass that requires target data and
/// finds none must say so rather than guess a default: [`crate::target`] is
/// the single source of what a backend accepts, and a pass inventing a
/// stand-in profile would be a second one.
///
/// # Why there is no `run(circuit)` convenience overload
///
/// Callers construct [`PassContext::none`] explicitly. "This pipeline ran
/// without target information" is then visible at the call site instead of
/// implied by an absent argument — which matters because the same pass can
/// legitimately produce different output in the two cases.
#[derive(Default, Clone, Copy)]
pub struct PassContext<'a> {
    profile: Option<&'a BasisProfile>,
    cost_model: Option<&'a dyn CostModel>,
}

impl<'a> PassContext<'a> {
    /// No target information — target-independent compilation.
    #[must_use]
    pub fn none() -> Self {
        PassContext::default()
    }

    /// A context carrying the selected target profile.
    #[must_use]
    pub fn with_profile(profile: &'a BasisProfile) -> Self {
        PassContext {
            profile: Some(profile),
            cost_model: None,
        }
    }

    /// Adds the target's cost model.
    #[must_use]
    pub fn and_cost_model(mut self, cost_model: &'a dyn CostModel) -> Self {
        self.cost_model = Some(cost_model);
        self
    }

    /// The selected target profile, if compilation is target-aware.
    #[must_use]
    pub fn profile(&self) -> Option<&'a BasisProfile> {
        self.profile
    }

    /// The target's cost model, if one was supplied.
    #[must_use]
    pub fn cost_model(&self) -> Option<&'a dyn CostModel> {
        self.cost_model
    }

    /// Whether any target information is available.
    #[must_use]
    pub fn is_target_aware(&self) -> bool {
        self.profile.is_some() || self.cost_model.is_some()
    }
}

impl std::fmt::Debug for PassContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PassContext")
            .field("profile", &self.profile.map(BasisProfile::qualified_id))
            .field("cost_model", &self.cost_model.map(CostModel::id))
            .finish()
    }
}

/// A single compiler pass.
///
/// Implementors must be deterministic: the same input circuit must always
/// produce the same output, since reproducibility is a project-wide
/// requirement (§22) and ablation studies are meaningless without it.
pub trait Pass: Send + Sync {
    /// Stable identifier, used to enable/disable the pass and to label its
    /// record. Kebab-case by convention, e.g. `"gate-cancellation"`.
    fn id(&self) -> &'static str;

    /// One-line description, shown by `oqci optimize --help` and in reports.
    fn description(&self) -> &'static str;

    /// Runs the pass.
    ///
    /// `context` carries the target being compiled for, when there is one.
    /// A target-independent pass ignores it; see [`PassContext`] for why it
    /// is threaded through every pass rather than only the ones that use it.
    ///
    /// # Errors
    ///
    /// Returns [`PassError::Ir`] if the pass produced a circuit that fails
    /// QC-IR validation, or if an internal IR operation failed.
    fn run(&self, circuit: &Circuit, context: &PassContext<'_>) -> Result<PassOutput, PassError>;
}

/// Which passes a pipeline run should execute.
#[derive(Debug, Clone, Default)]
pub enum PassSelection {
    /// Run every registered pass.
    #[default]
    All,
    /// Run only these passes, in the pipeline's registered order.
    Only(HashSet<String>),
    /// Run every pass except these — the shape an ablation study takes.
    AllExcept(HashSet<String>),
}

impl PassSelection {
    /// Builds a [`PassSelection::Only`] from any iterable of ids.
    pub fn only<I: IntoIterator<Item = S>, S: Into<String>>(ids: I) -> Self {
        PassSelection::Only(ids.into_iter().map(Into::into).collect())
    }

    /// Builds a [`PassSelection::AllExcept`] from any iterable of ids.
    pub fn all_except<I: IntoIterator<Item = S>, S: Into<String>>(ids: I) -> Self {
        PassSelection::AllExcept(ids.into_iter().map(Into::into).collect())
    }

    fn includes(&self, id: &str) -> bool {
        match self {
            PassSelection::All => true,
            PassSelection::Only(ids) => ids.contains(id),
            PassSelection::AllExcept(ids) => !ids.contains(id),
        }
    }
}

/// What one pass did, including when it was skipped.
///
/// A disabled pass still gets a record so an ablation run shows what was
/// *deliberately* left out rather than leaving a silent gap in the report.
#[derive(Debug, Clone)]
pub struct PassRecord {
    /// The pass's [`Pass::id`].
    pub id: String,
    /// The pass's [`Pass::description`].
    pub description: String,
    /// Whether the selection enabled this pass.
    pub enabled: bool,
    /// Whether it changed the circuit. Always `false` when disabled.
    pub changed: bool,
    /// Metrics before the pass ran.
    pub before: ResourceReport,
    /// Metrics after. Equal to `before` when the pass was disabled or made no
    /// change.
    pub after: ResourceReport,
    /// Wall-clock time spent in the pass. Zero when disabled.
    pub duration: Duration,
    /// The pass's own notes.
    pub notes: Vec<String>,
}

/// The result of running a pipeline.
#[derive(Debug, Clone)]
pub struct PassPipelineResult {
    /// The optimized circuit.
    pub circuit: Circuit,
    /// One record per registered pass, in execution order.
    pub records: Vec<PassRecord>,
}

impl PassPipelineResult {
    /// `true` if any pass changed the circuit.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.records.iter().any(|r| r.changed)
    }
}

/// An ordered, explicitly-registered sequence of passes.
///
/// ```
/// use oqci::ir::CircuitBuilder;
/// use oqci::pass::{PassContext, PassManager, PassSelection};
///
/// let mut b = CircuitBuilder::new("cancels");
/// let q0 = b.alloc_qubit();
/// b.h(q0).h(q0);                      // H ; H is the identity
///
/// // `PassContext::none()` is target-independent compilation, stated rather
/// // than implied — see `PassContext`.
/// let result = PassManager::default_pipeline()
///     .run(&b.build().unwrap(), &PassSelection::All, &PassContext::none())
///     .unwrap();
///
/// assert!(result.circuit.is_empty());
/// assert!(result.changed());
/// ```
pub struct PassManager {
    passes: Vec<Box<dyn Pass>>,
}

impl PassManager {
    /// An empty pipeline.
    #[must_use]
    pub fn new() -> Self {
        PassManager { passes: Vec::new() }
    }

    /// Appends a pass. Registration order **is** execution order.
    pub fn register(&mut self, pass: Box<dyn Pass>) -> &mut Self {
        self.passes.push(pass);
        self
    }

    /// The default optimization pipeline:
    ///
    /// ```text
    /// canonicalize → gate-cancellation → rotation-merge → canonicalize → schedule
    /// ```
    ///
    /// The ordering is deliberate, not incidental:
    ///
    /// - `canonicalize` runs first so later passes see a normalized circuit
    ///   (an explicit identity gate between two `H`s would otherwise hide a
    ///   cancellable pair).
    /// - `canonicalize` runs *again* after `rotation-merge`, because merging
    ///   `Rz(a); Rz(-a)` yields `Rz(0)`, which canonicalization already knows
    ///   how to remove exactly. This is a fixed, finite order rather than a
    ///   fixed-point loop — §7 asks for meaningful ordering, not iteration to
    ///   convergence, and a finite pipeline cannot fail to terminate.
    /// - `schedule` is last and read-only: it measures the circuit that
    ///   actually comes out.
    #[must_use]
    pub fn default_pipeline() -> Self {
        let mut manager = PassManager::new();
        manager
            .register(Box::new(Canonicalize))
            .register(Box::new(GateCancellation))
            .register(Box::new(RotationMerge))
            .register(Box::new(Canonicalize))
            .register(Box::new(Schedule));
        manager
    }

    /// The ids of every registered pass, in execution order. Contains repeats
    /// when a pass is registered more than once.
    #[must_use]
    pub fn pass_ids(&self) -> Vec<&'static str> {
        self.passes.iter().map(|p| p.id()).collect()
    }

    /// Runs the pipeline, threading the circuit through each enabled pass and
    /// recording what each one did.
    ///
    /// # Errors
    ///
    /// Returns the first [`PassError`] a pass raises, naming that pass. Later
    /// passes do not run.
    pub fn run(
        &self,
        circuit: &Circuit,
        selection: &PassSelection,
        context: &PassContext<'_>,
    ) -> Result<PassPipelineResult, PassError> {
        let mut current = circuit.clone();
        let mut records = Vec::with_capacity(self.passes.len());

        for pass in &self.passes {
            let id = pass.id();
            let before = analyze(&current).map_err(|e| PassError::from_ir(id, e))?;

            if !selection.includes(id) {
                records.push(PassRecord {
                    id: id.to_string(),
                    description: pass.description().to_string(),
                    enabled: false,
                    changed: false,
                    after: before.clone(),
                    before,
                    duration: Duration::ZERO,
                    notes: Vec::new(),
                });
                continue;
            }

            let started = Instant::now();
            let output = pass.run(&current, context)?;
            let duration = started.elapsed();

            current = output.circuit;
            let after = analyze(&current).map_err(|e| PassError::from_ir(id, e))?;

            records.push(PassRecord {
                id: id.to_string(),
                description: pass.description().to_string(),
                enabled: true,
                changed: output.changed,
                before,
                after,
                duration,
                notes: output.notes,
            });
        }

        Ok(PassPipelineResult {
            circuit: current,
            records,
        })
    }
}

impl Default for PassManager {
    fn default() -> Self {
        PassManager::new()
    }
}

/// Rebuilds a circuit from an instruction sequence, preserving its name and
/// register widths, and re-validating through [`crate::ir::CircuitBuilder`].
///
/// Every transform pass funnels its result through here, which is what makes
/// "a pass cannot produce an invalid circuit" true by construction rather
/// than by discipline.
///
/// # Errors
///
/// Returns [`PassError::Ir`] if the rewritten instruction sequence violates a
/// QC-IR invariant.
pub(crate) fn rebuild(
    pass: &str,
    original: &Circuit,
    instructions: Vec<crate::ir::Instruction>,
) -> Result<Circuit, PassError> {
    use crate::ir::{CircuitBuilder, Instruction};

    let mut builder = CircuitBuilder::new(original.name());
    builder.alloc_qubits(original.num_qubits());
    builder.alloc_clbits(original.num_clbits());

    for inst in instructions {
        match inst {
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

    builder.build().map_err(|e| PassError::from_ir(pass, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::CircuitBuilder;

    /// A pass that records what the context told it, so the channel itself
    /// can be tested rather than assumed. No shipped pass is target-aware
    /// yet; this proves the wiring is real before one is.
    struct ReportsTarget;

    impl Pass for ReportsTarget {
        fn id(&self) -> &'static str {
            "reports-target"
        }
        fn description(&self) -> &'static str {
            "test pass: reports the target it was given"
        }
        fn run(
            &self,
            circuit: &Circuit,
            context: &PassContext<'_>,
        ) -> Result<PassOutput, PassError> {
            let note = match (context.profile(), context.cost_model()) {
                (Some(profile), Some(model)) => {
                    format!("{} via {}", profile.qualified_id(), model.id())
                }
                (Some(profile), None) => profile.qualified_id(),
                _ => "target-independent".to_string(),
            };
            Ok(PassOutput::unchanged(circuit.clone()).with_note(note))
        }
    }

    #[test]
    fn a_pass_can_consult_the_selected_target() {
        // Stage E exit criterion 3, as an executable assertion.
        let profile = crate::target::builtin::linear_nisq(3);
        let model = crate::target::cost::resolve(profile.cost_model_id()).unwrap();
        let context = PassContext::with_profile(&profile).and_cost_model(&model);

        assert!(context.is_target_aware());

        let mut manager = PassManager::new();
        manager.register(Box::new(ReportsTarget));
        let result = manager
            .run(&cancellable(), &PassSelection::All, &context)
            .unwrap();

        assert_eq!(
            result.records[0].notes,
            vec!["linear-nisq@1 via nisq-weighted"]
        );
    }

    #[test]
    fn an_absent_target_is_visible_to_a_pass_rather_than_faked() {
        // A pass that needs target data must be able to tell that it has
        // none, instead of receiving an invented stand-in profile.
        let context = PassContext::none();
        assert!(!context.is_target_aware());
        assert!(context.profile().is_none());
        assert!(context.cost_model().is_none());

        let mut manager = PassManager::new();
        manager.register(Box::new(ReportsTarget));
        let result = manager
            .run(&cancellable(), &PassSelection::All, &context)
            .unwrap();
        assert_eq!(result.records[0].notes, vec!["target-independent"]);
    }

    #[test]
    fn a_context_can_carry_a_profile_without_a_cost_model() {
        let profile = crate::target::builtin::ideal_simulator();
        let context = PassContext::with_profile(&profile);
        assert!(context.is_target_aware());
        assert!(context.cost_model().is_none());
    }

    /// A pass that does nothing, for exercising the manager itself.
    struct Noop;
    impl Pass for Noop {
        fn id(&self) -> &'static str {
            "noop"
        }
        fn description(&self) -> &'static str {
            "does nothing"
        }
        fn run(
            &self,
            circuit: &Circuit,
            _context: &PassContext<'_>,
        ) -> Result<PassOutput, PassError> {
            Ok(PassOutput::unchanged(circuit.clone()))
        }
    }

    /// A pass that always fails, for exercising error propagation.
    struct Failing;
    impl Pass for Failing {
        fn id(&self) -> &'static str {
            "failing"
        }
        fn description(&self) -> &'static str {
            "always fails"
        }
        fn run(&self, _: &Circuit, _: &PassContext<'_>) -> Result<PassOutput, PassError> {
            Err(PassError::from_ir("failing", IrError::CyclicGraph))
        }
    }

    fn cancellable() -> Circuit {
        let mut b = CircuitBuilder::new("c");
        let q0 = b.alloc_qubit();
        b.h(q0).h(q0);
        b.build().unwrap()
    }

    #[test]
    fn registration_order_is_execution_order() {
        let mut manager = PassManager::new();
        manager
            .register(Box::new(Canonicalize))
            .register(Box::new(Noop))
            .register(Box::new(Schedule));
        assert_eq!(manager.pass_ids(), vec!["canonicalize", "noop", "schedule"]);

        let result = manager
            .run(&cancellable(), &PassSelection::All, &PassContext::none())
            .unwrap();
        let ids: Vec<&str> = result.records.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["canonicalize", "noop", "schedule"]);
    }

    #[test]
    fn default_pipeline_has_the_documented_order() {
        assert_eq!(
            PassManager::default_pipeline().pass_ids(),
            vec![
                "canonicalize",
                "gate-cancellation",
                "rotation-merge",
                "canonicalize",
                "schedule",
            ]
        );
    }

    #[test]
    fn disabled_passes_still_get_a_record() {
        let result = PassManager::default_pipeline()
            .run(
                &cancellable(),
                &PassSelection::all_except(["gate-cancellation"]),
                &PassContext::none(),
            )
            .unwrap();

        let cancellation = result
            .records
            .iter()
            .find(|r| r.id == "gate-cancellation")
            .expect("a record exists even though the pass was disabled");
        assert!(!cancellation.enabled);
        assert!(!cancellation.changed);
        assert_eq!(cancellation.before, cancellation.after);
        assert_eq!(cancellation.duration, Duration::ZERO);

        // With cancellation disabled, the H;H pair survives.
        assert_eq!(result.circuit.len(), 2);
    }

    #[test]
    fn only_selection_runs_just_those_passes() {
        let result = PassManager::default_pipeline()
            .run(
                &cancellable(),
                &PassSelection::only(["schedule"]),
                &PassContext::none(),
            )
            .unwrap();

        assert_eq!(result.records.len(), 5, "every pass is still reported");
        let enabled: Vec<&str> = result
            .records
            .iter()
            .filter(|r| r.enabled)
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(enabled, vec!["schedule"]);
        assert_eq!(result.circuit.len(), 2, "nothing was optimized away");
    }

    #[test]
    fn the_pipeline_is_deterministic() {
        let circuit = cancellable();
        let manager = PassManager::default_pipeline();
        let first = manager
            .run(&circuit, &PassSelection::All, &PassContext::none())
            .unwrap();
        let second = manager
            .run(&circuit, &PassSelection::All, &PassContext::none())
            .unwrap();
        assert_eq!(first.circuit, second.circuit);
        assert_eq!(
            first.records.iter().map(|r| r.changed).collect::<Vec<_>>(),
            second.records.iter().map(|r| r.changed).collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_already_optimal_circuit_reports_no_changes() {
        let mut b = CircuitBuilder::new("optimal");
        let q0 = b.alloc_qubit();
        let q1 = b.alloc_qubit();
        b.h(q0).cx(q0, q1);
        let circuit = b.build().unwrap();

        let result = PassManager::default_pipeline()
            .run(&circuit, &PassSelection::All, &PassContext::none())
            .unwrap();
        assert!(!result.changed());
        assert_eq!(result.circuit, circuit);
    }

    #[test]
    fn a_failing_pass_names_itself_and_stops_the_pipeline() {
        let mut manager = PassManager::new();
        manager.register(Box::new(Failing)).register(Box::new(Noop));

        let error = manager
            .run(&cancellable(), &PassSelection::All, &PassContext::none())
            .unwrap_err();
        assert!(matches!(&error, PassError::Ir { pass, .. } if pass == "failing"));
        assert!(error.to_string().contains("failing"));
    }

    #[test]
    fn records_carry_before_and_after_metrics() {
        let result = PassManager::default_pipeline()
            .run(&cancellable(), &PassSelection::All, &PassContext::none())
            .unwrap();

        let cancellation = result
            .records
            .iter()
            .find(|r| r.id == "gate-cancellation")
            .unwrap();
        assert_eq!(cancellation.before.op_count, 2);
        assert_eq!(cancellation.after.op_count, 0);
        assert!(cancellation.changed);
    }
}
