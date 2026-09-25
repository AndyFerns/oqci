//! Fine-grained replay: reconstructing per-pass and per-lowering-step
//! circuit states by calling the compiler's own public functions again,
//! never by modifying them.
//!
//! Every function here does two things, in order: (1) call real, unmodified
//! `oqci` functions to compute an intermediate state, and (2) cross-check
//! the final result against one real, complete, unmodified compiler call
//! (`ground_truth`, passed in by the caller from a single `compile_named`
//! invocation). If the cross-check ever fails, the replay reports itself as
//! `degraded` rather than presenting a reconstruction that might be wrong —
//! the caller falls back to the coarse, always-correct summary that ground
//! truth call already produced.
//!
//! Nothing in this file changes what any compiler function does. It only
//! calls them more times, with finer bookkeeping around each call.

use oqci::analysis::analyze;
use oqci::cli::snapshot::InstructionView;
use oqci::ir::{Circuit, CircuitBuilder, Instruction};
use oqci::lowering::layout::LayoutStrategy;
use oqci::lowering::routing::{RoutingOutput, RoutingStrategy};
use oqci::lowering::{DenseLayout, Layout, Lowered, LoweringConfig, RuleSet, Scope, ShortestPathRouter, TrivialLayout, decompose, routing};
use oqci::pass::{Canonicalize, GateCancellation, Pass, PassContext, PassManager, PassRecord, PassSelection, RotationMerge, Schedule};
use oqci::target::{BasisProfile, PhysicalQubit};

use crate::events::{LoweringReplay, LoweringStepReplay, PassReplay, PassStepEvent, RuleFiringEvent, SwapEvent};

// --- Pass-by-pass replay ----------------------------------------------------

/// The default pipeline's registered sequence, exactly as
/// `PassManager::default_pipeline()` builds it. This is not compiler logic —
/// it is a published fact (that function's own doc comment, and its own
/// `default_pipeline_has_the_documented_order` test in `src/pass/mod.rs`
/// pin this exact order). It is cross-checked against the real pipeline's
/// `pass_ids()` below before ever being trusted.
fn default_pipeline_prefix(n: usize) -> PassManager {
    let all: Vec<Box<dyn Pass>> = vec![
        Box::new(Canonicalize),
        Box::new(GateCancellation),
        Box::new(RotationMerge),
        Box::new(Canonicalize),
        Box::new(Schedule),
    ];
    let mut manager = PassManager::new();
    for pass in all.into_iter().take(n) {
        manager.register(pass);
    }
    manager
}

fn pass_degraded(reason: impl Into<String>) -> PassReplay {
    PassReplay {
        steps: Vec::new(),
        degraded: true,
        degraded_reason: Some(reason.into()),
    }
}

/// Reconstructs the circuit before and after every pass in the pipeline that
/// produced `ground_truth`, by re-running successively longer prefixes of
/// the real `PassManager::default_pipeline()` — the exact same `Pass`
/// implementations the real run used, just orchestrated from outside to
/// keep an intermediate result the real run itself discards.
pub fn replay_passes(
    source_circuit: &Circuit,
    selection: &PassSelection,
    context: &PassContext<'_>,
    ground_truth: &[PassRecord],
) -> PassReplay {
    let real_ids = PassManager::default_pipeline().pass_ids();
    if real_ids.len() != ground_truth.len() {
        return pass_degraded(format!(
            "assumed pipeline has {} passes, ground truth has {}",
            real_ids.len(),
            ground_truth.len()
        ));
    }

    let mut steps = Vec::with_capacity(ground_truth.len());
    for (i, record) in ground_truth.iter().enumerate() {
        if real_ids[i] != record.id {
            return pass_degraded(format!(
                "pass {i}: assumed id `{}`, ground truth reports `{}`",
                real_ids[i], record.id
            ));
        }

        let before_run = default_pipeline_prefix(i).run(source_circuit, selection, context);
        let after_run = default_pipeline_prefix(i + 1).run(source_circuit, selection, context);
        let (Ok(before_run), Ok(after_run)) = (before_run, after_run) else {
            return pass_degraded(format!("prefix replay failed to run at pass {i}"));
        };

        let after_metrics = match analyze(&after_run.circuit) {
            Ok(m) => m,
            Err(e) => return pass_degraded(format!("could not analyze replayed circuit: {e}")),
        };
        if after_metrics != record.after {
            return pass_degraded(format!(
                "pass {i} (`{}`) replay metrics do not match the ground truth",
                record.id
            ));
        }

        steps.push(PassStepEvent {
            pass_index: i,
            id: record.id.clone(),
            enabled: record.enabled,
            changed: record.changed,
            before: record.before.clone(),
            after: record.after.clone(),
            instructions_before: instruction_views(&before_run.circuit),
            instructions_after: instruction_views(&after_run.circuit),
            duration_us: record.duration.as_micros(),
            notes: record.notes.clone(),
        });
    }

    PassReplay {
        steps,
        degraded: false,
        degraded_reason: None,
    }
}

fn instruction_views(circuit: &Circuit) -> Vec<InstructionView> {
    circuit
        .instructions()
        .iter()
        .enumerate()
        .map(|(i, inst)| InstructionView::new(i, inst))
        .collect()
}

// --- Swap-by-swap and rule-by-rule lowering replay --------------------------

/// Reconstructs `lower()`'s seven-step schedule, with per-SWAP and
/// per-rule-firing detail inside the routing and decomposition steps, by
/// calling the same public functions `lower()` itself calls.
pub fn replay_lowering(
    circuit: &Circuit,
    profile: &BasisProfile,
    config: &LoweringConfig,
    ground_truth: &Lowered,
) -> LoweringReplay {
    match try_replay_lowering(circuit, profile, config, ground_truth) {
        Ok(steps) => LoweringReplay {
            steps,
            degraded: false,
            degraded_reason: None,
        },
        Err(reason) => LoweringReplay {
            steps: Vec::new(),
            degraded: true,
            degraded_reason: Some(reason),
        },
    }
}

fn view(instructions: &[Instruction]) -> Vec<InstructionView> {
    instructions
        .iter()
        .enumerate()
        .map(|(i, inst)| InstructionView::new(i, inst))
        .collect()
}

fn mnemonic_of(inst: &Instruction) -> String {
    match inst {
        Instruction::Gate { kind, .. } => kind.mnemonic().to_string(),
        Instruction::Measure { .. } => "measure".to_string(),
        Instruction::Reset { .. } => "reset".to_string(),
    }
}

fn qubits_of(inst: &Instruction) -> Vec<u32> {
    match inst {
        Instruction::Gate { qubits, .. } => qubits.iter().map(|q| q.index()).collect(),
        Instruction::Measure { qubit, .. } | Instruction::Reset { qubit } => vec![qubit.index()],
    }
}

/// Builds a validated `Circuit` from a raw instruction list, using the same
/// public `CircuitBuilder` path every frontend and every pass already uses.
/// This is the only place the replay constructs a full `Circuit` mid-schedule
/// (`decompose` and `repair_orientation` both work on raw instruction slices
/// directly and need no such wrapper) — it exists purely to satisfy
/// `LayoutStrategy::plan`/`RoutingStrategy::route`'s signatures, which take
/// `&Circuit`.
fn build_circuit(
    original: &Circuit,
    instructions: &[Instruction],
    num_qubits: u32,
) -> Result<Circuit, String> {
    let mut b = CircuitBuilder::new(original.name());
    b.alloc_qubits(num_qubits);
    b.alloc_clbits(original.num_clbits());
    for inst in instructions {
        match inst.clone() {
            Instruction::Gate { kind, qubits } => {
                b.gate(kind, qubits);
            }
            Instruction::Measure { qubit, target } => {
                b.measure(qubit, target);
            }
            Instruction::Reset { qubit } => {
                b.reset(qubit);
            }
        }
    }
    b.build().map_err(|e| e.to_string())
}

/// Runs `decompose()` once on the whole instruction list (the real
/// computation — ground truth for this scope) and, separately, once per
/// source instruction, to attribute each firing to the instruction that
/// caused it.
///
/// The per-instruction calls are provably equivalent to the whole-list call:
/// a decomposition rule may only permute the operands it was given and can
/// never introduce a new one (the project's own documented and tested I3
/// invariant), so decomposition has no cross-instruction interaction for
/// this split to get wrong — and the equivalence is cross-checked below
/// rather than merely assumed.
fn decompose_with_events(
    instructions: &[Instruction],
    profile: &BasisProfile,
    rules: &RuleSet,
    scope: Scope,
    scope_label: &str,
    sequence: &mut usize,
) -> Result<(Vec<Instruction>, Vec<RuleFiringEvent>), String> {
    let whole = decompose::decompose(instructions, profile, rules, scope).map_err(|e| e.to_string())?;

    let mut firings = Vec::new();
    let mut concatenated = Vec::new();
    for (i, inst) in instructions.iter().enumerate() {
        let single = decompose::decompose(std::slice::from_ref(inst), profile, rules, scope)
            .map_err(|e| e.to_string())?;
        if !single.rules_applied.is_empty() {
            *sequence += 1;
            firings.push(RuleFiringEvent {
                sequence: *sequence,
                scope: scope_label.to_string(),
                source_index: i,
                source_mnemonic: mnemonic_of(inst),
                source_qubits: qubits_of(inst),
                expanded_into: view(&single.instructions),
                rules_applied: single.rules_applied.clone(),
            });
        }
        concatenated.extend(single.instructions);
    }

    if concatenated != whole.instructions {
        return Err(format!(
            "per-instruction decompose replay does not match the whole-list call at scope `{scope_label}`"
        ));
    }

    Ok((whole.instructions, firings))
}

/// Reconstructs one [`SwapEvent`] per SWAP the real router inserted, by
/// walking the router's own input and output in lock-step.
///
/// Sound only because routing is documented and tested as insertion-only, in
/// program order — it "deletes no operation, reorders no pair of original
/// instructions, and only inserts `Swap`s between existing instructions"
/// (`docs/lowering.md`). That invariant is exactly what makes a simple
/// lock-step walk (rather than a general diff) correct here: every mismatch
/// between the two lists **must** be an inserted `Swap`, never a reorder or
/// deletion this walk could misread.
fn swap_events_from_diff(
    pre_route: &Circuit,
    routed: &RoutingOutput,
    sequence: &mut usize,
) -> Result<Vec<SwapEvent>, String> {
    let before = pre_route.instructions();
    let after = &routed.instructions;

    let mut events = Vec::new();
    let mut layout = routed.initial_layout.clone();
    let mut bi = 0usize;
    for inst in after {
        if bi < before.len() && *inst == before[bi] {
            bi += 1;
            continue;
        }
        let Instruction::Gate { kind, qubits } = inst else {
            return Err("routing replay found a non-gate insertion".into());
        };
        if kind.mnemonic() != "swap" || qubits.len() != 2 {
            return Err(format!(
                "routing replay found an unexpected insertion (`{}`), not a swap",
                kind.mnemonic()
            ));
        }
        let a = qubits[0].index();
        let b = qubits[1].index();
        layout.swap_physical(PhysicalQubit(a), PhysicalQubit(b));
        *sequence += 1;
        events.push(SwapEvent {
            sequence: *sequence,
            physical_a: a,
            physical_b: b,
            layout_after: layout.permutation(),
        });
    }
    if bi != before.len() {
        return Err("routing replay did not account for every original instruction".into());
    }
    if layout.permutation() != routed.final_layout.permutation() {
        return Err("replayed layout trajectory does not match the router's own final layout".into());
    }
    Ok(events)
}

fn try_replay_lowering(
    circuit: &Circuit,
    profile: &BasisProfile,
    config: &LoweringConfig,
    ground_truth: &Lowered,
) -> Result<Vec<LoweringStepReplay>, String> {
    let rules = RuleSet::new(profile).map_err(|e| e.to_string())?;
    let mut steps = Vec::new();
    let mut sequence = 0usize;

    // --- D0: arity reduction ---
    let mut instructions = circuit.instructions().to_vec();
    if config.decompose {
        let (reduced, firings) = decompose_with_events(
            &instructions,
            profile,
            &rules,
            Scope::ArityOnly,
            "arity-reduction",
            &mut sequence,
        )?;
        steps.push(LoweringStepReplay {
            id: "arity-reduction".into(),
            op_count: reduced.len(),
            detail: format!("{} rule firing(s)", firings.len()),
            instructions: view(&reduced),
            swap_events: Vec::new(),
            rule_firings: firings,
        });
        instructions = reduced;
    } else {
        steps.push(LoweringStepReplay {
            id: "arity-reduction".into(),
            op_count: instructions.len(),
            detail: "skipped".into(),
            instructions: view(&instructions),
            swap_events: Vec::new(),
            rule_firings: Vec::new(),
        });
    }

    // --- L: layout ---
    let reduced_circuit = build_circuit(circuit, &instructions, circuit.num_qubits())?;
    let topology = profile.topology();
    let initial_layout: Layout = match &config.layout {
        oqci::lowering::LayoutChoice::Trivial => TrivialLayout.plan(&reduced_circuit, topology),
        oqci::lowering::LayoutChoice::Dense => DenseLayout.plan(&reduced_circuit, topology),
        oqci::lowering::LayoutChoice::Explicit(layout) => Ok(layout.clone()),
        other => {
            return Err(format!(
                "replay does not yet know how to plan layout choice `{other:?}`"
            ));
        }
    }
    .map_err(|e| e.to_string())?;
    steps.push(LoweringStepReplay {
        id: "layout".into(),
        op_count: reduced_circuit.len(),
        detail: format!("{} layout: {initial_layout}", config.layout.id()),
        instructions: view(reduced_circuit.instructions()),
        swap_events: Vec::new(),
        rule_firings: Vec::new(),
    });

    // --- R: routing ---
    // Whether a reversed two-qubit interaction is expressible on this target
    // is `profile.supports_operation(m) || rules.rule_for(m).is_some()` —
    // the exact formula `docs/lowering.md` documents for `reaches`, read
    // here through the same two public methods rather than a private
    // helper.
    let can_reverse = profile.supports_operation("h") || rules.rule_for("h").is_some();
    let (routed_instructions, swap_events, final_layout) = if config.route {
        let output = ShortestPathRouter
            .route(&reduced_circuit, initial_layout.clone(), profile, can_reverse)
            .map_err(|e| e.to_string())?;
        let events = swap_events_from_diff(&reduced_circuit, &output, &mut sequence)?;
        (output.instructions, events, output.final_layout)
    } else {
        (
            reduced_circuit.instructions().to_vec(),
            Vec::new(),
            initial_layout.clone(),
        )
    };
    steps.push(LoweringStepReplay {
        id: "routing".into(),
        op_count: routed_instructions.len(),
        detail: format!("{} swap(s) inserted", swap_events.len()),
        instructions: view(&routed_instructions),
        swap_events,
        rule_firings: Vec::new(),
    });
    let _ = final_layout;

    let mut current = routed_instructions;

    if config.decompose {
        // --- D1: basis decomposition ---
        let (expanded, firings) = decompose_with_events(
            &current,
            profile,
            &rules,
            Scope::Basis,
            "basis-decomposition",
            &mut sequence,
        )?;
        steps.push(LoweringStepReplay {
            id: "basis-decomposition".into(),
            op_count: expanded.len(),
            detail: format!("{} rule firing(s)", firings.len()),
            instructions: view(&expanded),
            swap_events: Vec::new(),
            rule_firings: firings,
        });
        current = expanded;

        // --- O: orientation repair, one sweep ---
        let (repaired, count) = routing::repair_orientation(&current, profile).map_err(|e| e.to_string())?;
        steps.push(LoweringStepReplay {
            id: "orientation-repair".into(),
            op_count: repaired.len(),
            detail: format!("{count} reversed operation(s) repaired"),
            instructions: view(&repaired),
            swap_events: Vec::new(),
            rule_firings: Vec::new(),
        });
        current = repaired;

        // --- D2: single-qubit cleanup ---
        let (cleaned, firings) = decompose_with_events(
            &current,
            profile,
            &rules,
            Scope::SingleQubitOnly,
            "single-qubit-cleanup",
            &mut sequence,
        )?;
        steps.push(LoweringStepReplay {
            id: "single-qubit-cleanup".into(),
            op_count: cleaned.len(),
            detail: format!("{} rule firing(s)", firings.len()),
            instructions: view(&cleaned),
            swap_events: Vec::new(),
            rule_firings: firings,
        });
        current = cleaned;
    }

    // --- V: verify — this replay reuses the ground truth's own legality
    // rather than re-deriving it, since `lower()`'s `verify` step calls a
    // private helper this crate cannot reach; the ground truth's
    // `Lowered::legality` already *is* that computation's real result.
    steps.push(LoweringStepReplay {
        id: "verify".into(),
        op_count: current.len(),
        detail: if ground_truth.legality.is_legal() {
            "legal".into()
        } else {
            format!("{} violation(s)", ground_truth.legality.violation_count())
        },
        instructions: view(&current),
        swap_events: Vec::new(),
        rule_firings: Vec::new(),
    });

    // --- Cross-validate the whole replay against the real lower() call ---
    if current != ground_truth.circuit.instructions().to_vec() {
        return Err("replayed instruction sequence does not match the real lower() output".into());
    }
    let total_swaps: usize = steps.iter().map(|s| s.swap_events.len()).sum();
    if total_swaps != ground_truth.swaps_inserted {
        return Err("replayed swap count does not match Lowered::swaps_inserted".into());
    }

    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oqci::compile::{CompileError, CompilerConfig, Stop};
    use oqci::pass::PassSelection;

    /// Exercises gate-cancellation (an `x;x` pair straddling an independent
    /// `h`) and, once lowered onto `simulator-nisq`, basis decomposition and
    /// routing — the corpus this test suite's cross-validation must survive
    /// against, on every change to either the compiler or this replay code.
    const GHZ3: &str = r#"
        OPENQASM 3.0;
        include "stdgates.inc";
        qubit[3] q;
        bit[3] c;
        h q[0];
        x q[2];
        h q[1];
        x q[2];
        cx q[0], q[1];
        cx q[1], q[2];
        c = measure q;
    "#;

    const BELL: &str = "qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;";

    fn compile(source: &str, backend: &str) -> Result<oqci::compile::CompilationArtifacts, CompileError> {
        let config = CompilerConfig {
            backend: Some(backend.to_string()),
            stop: Stop::Prepared,
            ..CompilerConfig::default()
        };
        oqci::compile::compile_named(source, "t", &config)
    }

    fn context_for<'a>(backend: &'a dyn oqci::backend::Backend) -> PassContext<'a> {
        PassContext::with_profile(backend.profile()).and_cost_model(backend.cost_model())
    }

    #[test]
    fn pass_replay_matches_ground_truth_and_is_not_degraded() {
        for source in [GHZ3, BELL] {
            let artifacts = compile(source, "simulator-nisq").unwrap();
            let backend = oqci::backend::by_id("simulator-nisq").unwrap();
            let context = context_for(backend.as_ref());

            let replay = replay_passes(
                &artifacts.source_circuit,
                &PassSelection::All,
                &context,
                &artifacts.pass_records,
            );

            assert!(!replay.degraded, "degraded: {:?}", replay.degraded_reason);
            assert_eq!(replay.steps.len(), artifacts.pass_records.len());

            // The very first step's "before" must equal the source circuit,
            // and the very last step's "after" must equal the ground
            // truth's optimized circuit — the two ends the chain of prefix
            // replays has to nail for every step in between to be trusted.
            let first = &replay.steps[0];
            assert_eq!(
                first.instructions_before.len(),
                artifacts.source_circuit.len(),
                "first step's `before` should be the unmodified source circuit"
            );
            let last = replay.steps.last().unwrap();
            assert_eq!(
                last.instructions_after.len(),
                artifacts.optimized.len(),
                "last step's `after` should be the final optimized circuit"
            );

            // gate-cancellation must show as `changed: true` with a visibly
            // smaller instruction count on GHZ3's redundant x;x pair.
            if source == GHZ3 {
                let cancellation = replay
                    .steps
                    .iter()
                    .find(|s| s.id == "gate-cancellation")
                    .unwrap();
                assert!(cancellation.changed);
                assert!(cancellation.instructions_after.len() < cancellation.instructions_before.len());
            }
        }
    }

    #[test]
    fn lowering_replay_matches_ground_truth_and_is_not_degraded() {
        for source in [GHZ3, BELL] {
            let artifacts = compile(source, "simulator-nisq").unwrap();
            let backend = oqci::backend::by_id("simulator-nisq").unwrap();
            let lowered = artifacts.lowered.as_ref().unwrap();

            let replay = replay_lowering(
                &artifacts.optimized,
                backend.profile(),
                &oqci::lowering::LoweringConfig::default(),
                lowered,
            );

            assert!(!replay.degraded, "degraded: {:?}", replay.degraded_reason);
            assert_eq!(replay.steps.len(), 7, "the seven-step schedule");
            assert_eq!(
                replay.steps.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
                vec![
                    "arity-reduction",
                    "layout",
                    "routing",
                    "basis-decomposition",
                    "orientation-repair",
                    "single-qubit-cleanup",
                    "verify",
                ]
            );

            // The final step's instructions must equal what `lower()` itself
            // produced, verbatim.
            let last = replay.steps.last().unwrap();
            assert_eq!(last.instructions.len(), lowered.circuit.len());

            // Every rule firing's expansion must be non-empty, and the
            // total swap-event count must equal what the real router
            // reported.
            let total_swaps: usize = replay.steps.iter().map(|s| s.swap_events.len()).sum();
            assert_eq!(total_swaps, lowered.swaps_inserted);
            for step in &replay.steps {
                for firing in &step.rule_firings {
                    assert!(!firing.expanded_into.is_empty());
                    assert!(!firing.rules_applied.is_empty());
                }
            }
        }
    }

    #[test]
    fn a_deliberately_wrong_ground_truth_is_reported_degraded_not_silently_accepted() {
        // Proves the cross-validation actually checks something: feed a
        // `Lowered` whose `swaps_inserted` is wrong and confirm the replay
        // notices rather than reporting success anyway.
        let artifacts = compile(GHZ3, "simulator-nisq").unwrap();
        let backend = oqci::backend::by_id("simulator-nisq").unwrap();
        let mut lowered = artifacts.lowered.clone().unwrap();
        lowered.swaps_inserted = lowered.swaps_inserted.wrapping_add(1);

        let replay = replay_lowering(
            &artifacts.optimized,
            backend.profile(),
            &oqci::lowering::LoweringConfig::default(),
            &lowered,
        );
        assert!(replay.degraded);
    }
}
