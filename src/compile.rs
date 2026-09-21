//! The compiler orchestrator.
//!
//! §6 asks for one layer that can perform
//! `Frontend -> QC-IR -> QCO-IR -> pass pipeline -> target lowering ->
//! backend preparation`, accepting an explicit configuration, selecting a
//! frontend, validating, constructing IR, running passes in explicit order,
//! recording pass metadata, selecting a backend, applying target-aware
//! lowering, and returning structured artifacts. This module is that layer.
//!
//! # No vendor names appear here
//!
//! §6 closes with "the orchestrator must not hard-code IBM-specific logic",
//! and Stage C §6 spells out the shape that must not appear: `if IBM … else
//! if Cirq …` branching through the compiler. Backend selection here is a
//! registry lookup returning a `Box<dyn Backend>`, so adding a device does not
//! touch this file at all. The word "IBM" appears in this module only in this
//! paragraph.
//!
//! # Why the CLI goes through here
//!
//! §19 requires the CLI not to "duplicate compiler logic that belongs in the
//! Rust library — it should invoke the same public compiler APIs". Routing
//! `src/cli` through [`compile`] makes that structurally true rather than a
//! convention someone eventually breaks: there is one path from source text
//! to artifacts, so the numbers a user sees are the numbers the compiler
//! computed.

use std::collections::HashMap;

use crate::analysis::{ResourceReport, analyze};
use crate::backend::{
    BackendError, Executable, ExecutionSettings, Provenance, simulator::provenance_for,
};
use crate::frontend::{FrontendError, parse_openqasm3_named};
use crate::ir::{Circuit, IrError, bind_parameters};
use crate::lowering::{Lowered, LoweringConfig};
use crate::pass::{PassContext, PassError, PassManager, PassRecord, PassSelection};
use crate::target::Cost;

/// Which frontend to read the source with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Frontend {
    /// The documented OpenQASM 3 subset.
    #[default]
    OpenQasm3,
}

/// How far through the pipeline to go.
///
/// Stopping early is a first-class request, not a degraded run: `oqci
/// optimize` genuinely does not want a target, and asking for one would force
/// a choice the user did not make.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stop {
    /// Frontend and optimization only.
    Optimized,
    /// ...and target lowering.
    Lowered,
    /// ...and execution preparation.
    #[default]
    Prepared,
}

/// Everything the orchestrator needs to know.
#[derive(Debug, Clone, Default)]
pub struct CompilerConfig {
    /// Which frontend reads the source.
    pub frontend: Frontend,
    /// Values for symbolic parameters, applied before optimization.
    pub bindings: HashMap<String, f64>,
    /// Which optimization passes to run.
    pub passes: PassSelection,
    /// Which backend to compile for. `None` means target-independent
    /// compilation, which stops after optimization whatever `stop` says.
    pub backend: Option<String>,
    /// How to lower.
    pub lowering: LoweringConfig,
    /// Shots, seed and so on.
    pub settings: ExecutionSettings,
    /// Where to stop.
    pub stop: Stop,
}

/// Why compilation failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum CompileError {
    /// The source could not be read.
    #[error(transparent)]
    Frontend {
        /// What the frontend said.
        #[from]
        source: FrontendError,
    },
    /// Parameter binding or an IR operation failed.
    #[error(transparent)]
    Ir {
        /// The IR problem.
        #[from]
        source: IrError,
    },
    /// A pass failed.
    #[error(transparent)]
    Pass {
        /// Which pass, and why.
        #[from]
        source: PassError,
    },
    /// Lowering or preparation failed.
    #[error(transparent)]
    Backend {
        /// What the backend refused.
        #[from]
        source: BackendError,
    },
    /// The configuration named a backend that does not exist.
    #[error("unknown backend `{requested}`; available: {available}")]
    UnknownBackend {
        /// What was asked for.
        requested: String,
        /// What exists.
        available: String,
    },
}

/// What a compilation produced.
///
/// Every stage's output is kept rather than only the last one: §13 requires
/// before/after analysis, and a result that reported only the final circuit
/// could not answer "what did optimization actually do".
#[derive(Debug, Clone)]
pub struct CompilationArtifacts {
    /// The circuit as the frontend read it, after parameter binding.
    pub source_circuit: Circuit,
    /// After the pass pipeline.
    pub optimized: Circuit,
    /// What each pass did.
    pub pass_records: Vec<PassRecord>,
    /// Metrics for the source circuit.
    pub source_metrics: ResourceReport,
    /// Metrics after optimization.
    pub optimized_metrics: ResourceReport,
    /// The lowered circuit, when a backend was selected.
    pub lowered: Option<Lowered>,
    /// The executable, when compilation ran to preparation.
    pub executable: Option<Executable>,
    /// What the target thinks the lowered circuit costs.
    pub cost: Option<Cost>,
    /// The backend this was compiled for.
    pub backend_id: Option<String>,
    /// Everything needed to cite a result, when a backend was selected.
    pub provenance: Option<Provenance>,
}

impl CompilationArtifacts {
    /// The furthest circuit compilation reached — lowered if it got that far,
    /// otherwise optimized.
    #[must_use]
    pub fn final_circuit(&self) -> &Circuit {
        self.lowered
            .as_ref()
            .map_or(&self.optimized, |lowered| &lowered.circuit)
    }
}

/// Runs the pipeline.
///
/// # Errors
///
/// Any [`CompileError`]. The pipeline stops at the first failure and reports
/// it; there is no partially-compiled artifact, because a caller cannot tell
/// from one how far it is safe to trust.
pub fn compile(
    source: &str,
    config: &CompilerConfig,
) -> Result<CompilationArtifacts, CompileError> {
    compile_named(source, "main", config)
}

/// As [`compile`], with an explicit circuit name.
///
/// The name ends up in the provenance record, so it is worth being able to
/// set it to something meaningful — a filename, usually — rather than
/// accepting a default that makes every result look alike.
///
/// # Errors
///
/// As [`compile`].
pub fn compile_named(
    source: &str,
    name: &str,
    config: &CompilerConfig,
) -> Result<CompilationArtifacts, CompileError> {
    // --- Frontend ---
    let parsed = match config.frontend {
        Frontend::OpenQasm3 => parse_openqasm3_named(source, name)?,
    };

    // --- Parameter binding, before optimization ---
    // Binding first means the passes see concrete angles and can actually
    // fire: `Rz(theta); Rz(-theta)` cancels only once both are numbers.
    let source_circuit = if config.bindings.is_empty() {
        parsed
    } else {
        bind_parameters(&parsed, &config.bindings)?
    };
    let source_metrics = analyze(&source_circuit)?;

    // --- Backend selection ---
    // A registry lookup, not a match on a vendor name (Stage C §6).
    let backend = match &config.backend {
        None => None,
        Some(id) => Some(crate::backend::by_id(id).ok_or_else(|| {
            CompileError::UnknownBackend {
                requested: id.clone(),
                available: crate::backend::all()
                    .iter()
                    .map(|b| b.id().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            }
        })?),
    };

    // --- Optimization ---
    // Passes get the target when there is one. None of the shipped generic
    // passes consults it — Stage E §8 is explicit that target awareness must
    // not make every pass backend-specific — but the channel is open, which
    // is what Stage E exit criterion 3 asks for.
    let context = match backend.as_ref() {
        Some(backend) => {
            PassContext::with_profile(backend.profile()).and_cost_model(backend.cost_model())
        }
        None => PassContext::none(),
    };
    let result = PassManager::default_pipeline().run(&source_circuit, &config.passes, &context)?;
    let optimized_metrics = analyze(&result.circuit)?;

    let mut artifacts = CompilationArtifacts {
        source_circuit,
        optimized: result.circuit,
        pass_records: result.records,
        source_metrics,
        optimized_metrics,
        lowered: None,
        executable: None,
        cost: None,
        backend_id: backend.as_ref().map(|b| b.id().to_string()),
        provenance: None,
    };

    let Some(backend) = backend else {
        // Target-independent compilation is a complete result, not a failure
        // to reach a target. There is nothing to lower *to*.
        return Ok(artifacts);
    };
    if config.stop == Stop::Optimized {
        return Ok(artifacts);
    }

    // --- Target lowering ---
    let lowered = backend.lower(&artifacts.optimized, &config.lowering)?;
    artifacts.cost = Some(
        backend
            .cost_model()
            .evaluate(&lowered.circuit, backend.profile())?,
    );

    let pass_pipeline: Vec<String> = artifacts
        .pass_records
        .iter()
        .filter(|record| record.enabled)
        .map(|record| record.id.clone())
        .collect();
    let provenance = provenance_for(
        backend.as_ref(),
        &artifacts.source_circuit,
        &lowered,
        &pass_pipeline,
        &config.settings,
    );
    artifacts.provenance = Some(provenance.clone());

    if config.stop == Stop::Lowered {
        artifacts.lowered = Some(lowered);
        return Ok(artifacts);
    }

    // --- Execution preparation ---
    // `prepare` re-validates before building the artifact, so an executable
    // cannot exist for a circuit the target rejects (Stage C exit criterion 5).
    artifacts.executable = Some(backend.prepare(&lowered, &config.settings, provenance)?);
    artifacts.lowered = Some(lowered);
    Ok(artifacts)
}

/// Every backend this build can compile for, as `(id, description)`.
#[must_use]
pub fn available_backends() -> Vec<(String, String)> {
    crate::backend::all()
        .iter()
        .map(|backend| (backend.id().to_string(), backend.description().to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BELL: &str = "qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;";

    fn config(backend: Option<&str>) -> CompilerConfig {
        CompilerConfig {
            backend: backend.map(str::to_string),
            ..CompilerConfig::default()
        }
    }

    #[test]
    fn target_independent_compilation_is_a_complete_result() {
        // Not a degraded run: with no backend there is nothing to lower to,
        // and that is a legitimate way to finish.
        let artifacts = compile(BELL, &config(None)).unwrap();
        assert!(artifacts.lowered.is_none());
        assert!(artifacts.executable.is_none());
        assert!(artifacts.provenance.is_none());
        assert_eq!(artifacts.source_metrics.num_qubits, 2);
        assert!(!artifacts.pass_records.is_empty());
    }

    #[test]
    fn a_full_compilation_reaches_an_executable() {
        let artifacts = compile(BELL, &config(Some("simulator-nisq"))).unwrap();

        let lowered = artifacts.lowered.as_ref().unwrap();
        assert!(lowered.legality.is_legal());
        let executable = artifacts.executable.as_ref().unwrap();
        assert_eq!(executable.backend_id, "simulator-nisq");
        assert!(executable.has_measurement());
        assert!(artifacts.cost.is_some());
    }

    #[test]
    fn stopping_early_stops_exactly_where_asked() {
        let mut config = config(Some("simulator"));

        config.stop = Stop::Optimized;
        let artifacts = compile(BELL, &config).unwrap();
        assert!(artifacts.lowered.is_none() && artifacts.executable.is_none());

        config.stop = Stop::Lowered;
        let artifacts = compile(BELL, &config).unwrap();
        assert!(artifacts.lowered.is_some() && artifacts.executable.is_none());

        config.stop = Stop::Prepared;
        let artifacts = compile(BELL, &config).unwrap();
        assert!(artifacts.lowered.is_some() && artifacts.executable.is_some());
    }

    #[test]
    fn provenance_names_the_whole_pipeline_that_produced_the_result() {
        let artifacts = compile(BELL, &config(Some("simulator-nisq"))).unwrap();
        let provenance = artifacts.provenance.as_ref().unwrap();

        assert_eq!(provenance.backend_id, "simulator-nisq");
        assert_eq!(provenance.profile_id, "linear-nisq@1");
        assert!(
            provenance
                .pass_pipeline
                .contains(&"canonicalize".to_string())
        );
        assert!(provenance.lowering_steps.contains(&"routing".to_string()));
        assert!(!provenance.cost_model_configuration.is_empty());
    }

    #[test]
    fn a_disabled_pass_is_not_claimed_to_have_run() {
        // Provenance records the passes that *ran*, not the ones registered —
        // an ablation study whose record said otherwise would be worthless.
        let config = CompilerConfig {
            backend: Some("simulator".into()),
            passes: PassSelection::all_except(["gate-cancellation"]),
            ..CompilerConfig::default()
        };
        let artifacts = compile(BELL, &config).unwrap();
        let provenance = artifacts.provenance.as_ref().unwrap();

        assert!(
            !provenance
                .pass_pipeline
                .contains(&"gate-cancellation".to_string())
        );
        assert!(
            artifacts
                .pass_records
                .iter()
                .any(|r| r.id == "gate-cancellation" && !r.enabled),
            "the skipped pass is still recorded, so the gap is visible"
        );
    }

    #[test]
    fn parameters_are_bound_before_optimization_so_the_passes_can_fire() {
        // Rotation merging only fires on concrete angles: there is no `Param`
        // variant for "a + b" of two symbols, and inventing one is out of
        // scope. Binding after optimization would therefore silently lose
        // every rewrite that depends on a number.
        //
        // (The frontend's subset does not accept `rz(-theta)` — a compound
        // expression over a parameter — so the pair here is `theta; theta`,
        // which merges rather than cancels. Same point.)
        let source = "input float[64] theta; qubit[1] q; rz(theta) q[0]; rz(theta) q[0];";

        let unbound = compile(source, &CompilerConfig::default()).unwrap();
        assert_eq!(
            unbound.optimized.len(),
            2,
            "two symbolic rotations cannot be merged"
        );

        let bound = compile(
            source,
            &CompilerConfig {
                bindings: HashMap::from([("theta".to_string(), 0.5)]),
                ..CompilerConfig::default()
            },
        )
        .unwrap();
        assert_eq!(
            bound.optimized.len(),
            1,
            "bound first, the two rotations merge into one: {:?}",
            bound.optimized.instructions()
        );
    }

    #[test]
    fn an_unknown_backend_lists_what_is_available() {
        let err = compile(BELL, &config(Some("quantum-supercomputer"))).unwrap_err();
        assert!(
            err.to_string().contains("simulator"),
            "the error should name the alternatives: {err}"
        );
    }

    #[test]
    fn a_malformed_program_fails_in_the_frontend() {
        let err = compile("qubit[2] q; nonsense", &config(None)).unwrap_err();
        assert!(matches!(err, CompileError::Frontend { .. }), "got {err}");
    }

    #[test]
    fn a_circuit_too_wide_for_the_device_fails_in_lowering() {
        let wide = "qubit[8] q; h q[0]; cx q[0], q[7];";
        let err = compile(wide, &config(Some("simulator-nisq"))).unwrap_err();
        assert!(matches!(err, CompileError::Backend { .. }), "got {err}");
    }

    #[test]
    fn the_final_circuit_is_the_furthest_stage_reached() {
        let target_independent = compile(BELL, &config(None)).unwrap();
        assert_eq!(
            target_independent.final_circuit(),
            &target_independent.optimized
        );

        let full = compile(BELL, &config(Some("simulator-nisq"))).unwrap();
        assert_eq!(
            full.final_circuit(),
            &full.lowered.as_ref().unwrap().circuit
        );
    }

    #[test]
    fn compilation_is_deterministic() {
        let config = config(Some("simulator-nisq"));
        let once = compile(BELL, &config).unwrap();
        let twice = compile(BELL, &config).unwrap();
        assert_eq!(once.optimized, twice.optimized);
        assert_eq!(
            once.lowered.as_ref().unwrap().circuit,
            twice.lowered.as_ref().unwrap().circuit
        );
        assert_eq!(once.executable, twice.executable);
    }

    #[test]
    fn every_available_backend_can_compile_a_bell_circuit() {
        for (id, _) in available_backends() {
            let artifacts = compile(BELL, &config(Some(&id)))
                .unwrap_or_else(|e| panic!("backend `{id}` failed: {e}"));
            assert!(
                artifacts.lowered.as_ref().unwrap().legality.is_legal(),
                "backend `{id}` produced an illegal circuit"
            );
        }
    }
}
