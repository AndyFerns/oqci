//! The shared pipeline runner.
//!
//! `final-deliverables-spec.md` §19: the CLI "must not duplicate compiler
//! logic that belongs in the Rust library — it should invoke the same public
//! compiler APIs." These two functions are the *only* place in the CLI that
//! touches the compiler, and they do nothing but call library functions and
//! record what came back:
//!
//! - [`crate::compile::Frontend`] — source to QC-IR, the frontend chosen by
//!   file extension (`.py` is a CUDA-Q kernel, anything else OpenQASM 3)
//! - [`crate::ir::bind_parameters`] — symbolic to concrete
//! - [`crate::ir::qc_to_qco`] — QC-IR to QCO-IR
//! - [`crate::pass::PassManager`] — the optimization pipeline
//! - [`crate::analysis::analyze`] / [`crate::analysis::diff_circuits`] — every
//!   metric and every diff
//!
//! No gate is interpreted, no metric recomputed, no circuit rewritten here.
//! `compile`, `optimize`, `analyze` and every tick of `watch` all funnel
//! through these functions, so the four commands cannot disagree with each
//! other or with the compiler.

use std::collections::HashMap;
use std::path::Path;

use crate::analysis::{analyze, diff_circuits};
use crate::backend::ExecutionSettings;
use crate::cli::CliError;
use crate::cli::snapshot::{
    PassRecordView, PipelineReport, QirInfoView, StageSnapshot, TimingsView, diff_view,
    executable_view, graph_of, instructions_of, lowering_view, target_report,
};
use crate::compile::{CompilationArtifacts, CompilerConfig, QirArtifact, QirRequest, Stop};
use crate::extension::Registry;
use crate::ir::{Circuit, qc_to_qco};
use crate::lowering::LoweringConfig;
use crate::pass::PassSelection;
use crate::target::{BasisProfile, WeightedCostModel, cost};

/// Which stages to include in a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Stage {
    /// The imperative circuit IR.
    QcIr,
    /// The dependency graph.
    QcoIr,
    /// Emitted QIR text.
    Qir,
}

impl Stage {
    /// Every stage, in pipeline order — the default when `--emit` is omitted.
    pub fn all() -> Vec<Stage> {
        vec![Stage::QcIr, Stage::QcoIr, Stage::Qir]
    }

    fn label(self) -> &'static str {
        match self {
            Stage::QcIr => "qc-ir",
            Stage::QcoIr => "qco-ir",
            Stage::Qir => "qir",
        }
    }
}

/// What every command shares beyond its own arguments: the extensions it may
/// use and the optional report sections it asked for.
#[derive(Debug, Clone, Default)]
pub struct Session {
    /// Extension frontends, passes, backends and lint rules, plus any target
    /// file loaded for this run.
    pub registry: Registry,
    /// The frontend named with `--frontend`, overriding the file extension.
    pub frontend: Option<String>,
    /// Whether to include lint findings (`--lint`).
    pub lint: bool,
    /// Whether to include per-stage timings (`--timings`).
    pub timings: bool,
    /// Semantic verification (`--verify`).
    pub verify: crate::verify::VerifyConfig,
}

/// Runs frontend → QC-IR → QCO-IR → QIR, recording each requested stage.
///
/// # Errors
///
/// [`CliError`] from any stage. A circuit with unbound parameters is **not**
/// an error: the QIR stage records why it could not run, so `watch` stays
/// useful while an ansatz is being written.
#[cfg(test)]
pub fn run_compile(
    path: &Path,
    source: &str,
    bindings: &HashMap<String, f64>,
    emit: &[Stage],
    target: Option<&BasisProfile>,
) -> Result<PipelineReport, CliError> {
    run_compile_with(
        path,
        source,
        bindings,
        emit,
        target,
        &QirRequest::default(),
        &Session::default(),
    )
}

/// As [`run_compile`], with an explicit request for the QIR output.
///
/// No passes run, so the canonical QIR is emitted from the circuit exactly as
/// the frontend read it (recorded as stage `optimized`, the pass pipeline's
/// output, which here is the identity).
///
/// # Errors
///
/// As [`run_compile`], plus [`crate::compile::CompileError::QirProfile`] when
/// `qir` demands the Base Profile and the circuit does not qualify.
pub fn run_compile_with(
    path: &Path,
    source: &str,
    bindings: &HashMap<String, f64>,
    emit: &[Stage],
    target: Option<&BasisProfile>,
    qir: &QirRequest,
    session: &Session,
) -> Result<PipelineReport, CliError> {
    // Everything before rendering comes from the orchestrator, so the CLI
    // cannot drift from the library (spec 19: the CLI "must not duplicate
    // compiler logic"). Only presentation lives here.
    let artifacts = orchestrate(
        path,
        source,
        bindings,
        &PassSelection::only::<[&str; 0], &str>([]),
        qir,
        session,
    )?;
    let mut report = new_report(path, &artifacts, &artifacts.source_circuit, session);
    report.stages = stages_for(&artifacts.source_circuit, emit, artifacts.qir.as_ref())?;
    report.target = target_for(&artifacts.source_circuit, target)?;
    if session.lint {
        report.lint = Some(
            session
                .registry
                .linter()
                .lint(&artifacts.source_circuit, target),
        );
    }
    Ok(report)
}

/// Lints a program: the source circuit, or the optimized one, against a
/// target profile when one is given.
///
/// # Errors
///
/// [`CliError`] from the frontend or the pass pipeline.
pub fn run_lint(
    path: &Path,
    source: &str,
    bindings: &HashMap<String, f64>,
    optimized: bool,
    target: Option<&BasisProfile>,
    session: &Session,
) -> Result<PipelineReport, CliError> {
    let passes = if optimized {
        PassSelection::All
    } else {
        PassSelection::only::<[&str; 0], &str>([])
    };
    let artifacts = orchestrate(
        path,
        source,
        bindings,
        &passes,
        &QirRequest::default(),
        session,
    )?;
    let circuit = if optimized {
        &artifacts.optimized
    } else {
        &artifacts.source_circuit
    };
    let mut report = new_report(path, &artifacts, circuit, session);
    report.lint = Some(session.registry.linter().lint(circuit, target));
    report.target = target_for(circuit, target)?;
    Ok(report)
}

/// Runs the frontend and, optionally, the pass pipeline through the shared
/// orchestrator.
///
/// `compile` is the single path from source text to artifacts. Routing every
/// command through it is what keeps the numbers a user sees identical to the
/// numbers the compiler computed.
fn orchestrate(
    path: &Path,
    source: &str,
    bindings: &HashMap<String, f64>,
    passes: &PassSelection,
    qir: &QirRequest,
    session: &Session,
) -> Result<CompilationArtifacts, CliError> {
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("main");
    let config = CompilerConfig {
        bindings: bindings.clone(),
        passes: passes.clone(),
        stop: Stop::Optimized,
        qir: *qir,
        extensions: session.registry.clone(),
        verify: session.verify,
        ..CompilerConfig::default()
    };
    let frontend = frontend_id(path, session);
    Ok(crate::compile::compile_with_frontend(
        source, name, &frontend, &config,
    )?)
}

/// The frontend to read `path` with: `--frontend` when given, otherwise the
/// one the file extension implies (an extension's claim first, then the
/// built-in rule: `.py` is CUDA-Q, anything else OpenQASM 3).
fn frontend_id(path: &Path, session: &Session) -> String {
    session
        .frontend
        .clone()
        .unwrap_or_else(|| session.registry.frontend_for_path(path).id().to_string())
}

/// Evaluates a circuit against a target profile, when one was selected.
///
/// The cost model is the one the profile names — Stage E's rule that the
/// *target* defines what is expensive, not the tool doing the reporting.
fn target_for(
    circuit: &Circuit,
    target: Option<&BasisProfile>,
) -> Result<Option<crate::cli::snapshot::TargetReportView>, CliError> {
    let Some(profile) = target else {
        return Ok(None);
    };
    let cost_model = cost_model_for(profile)?;
    Ok(Some(target_report(circuit, profile, &cost_model)?))
}

/// The cost model a profile names, or a usage error naming both.
///
/// Resolution can fail: `cost::resolve` returns `None` for an unknown id
/// rather than substituting a default, because a result recording a cost
/// model that did not produce it is false provenance (Stage E §9).
fn cost_model_for(profile: &BasisProfile) -> Result<WeightedCostModel, CliError> {
    cost::resolve(profile.cost_model_id()).ok_or_else(|| {
        CliError::Usage(format!(
            "target `{}` references unknown cost model `{}`",
            profile.id(),
            profile.cost_model_id()
        ))
    })
}

/// Runs frontend → QC-IR → pass pipeline → QCO-IR → QIR.
///
/// # Errors
///
/// [`CliError`] from the frontend, a pass, or lowering.
#[cfg(test)]
pub fn run_optimize(
    path: &Path,
    source: &str,
    bindings: &HashMap<String, f64>,
    selection: &PassSelection,
    emit: &[Stage],
    want_diff: bool,
    target: Option<&BasisProfile>,
) -> Result<PipelineReport, CliError> {
    run_optimize_with(
        path,
        source,
        bindings,
        selection,
        emit,
        want_diff,
        target,
        &QirRequest::default(),
        &Session::default(),
    )
}

/// As [`run_optimize`], with an explicit request for the QIR output.
///
/// # Errors
///
/// As [`run_optimize`], plus [`crate::compile::CompileError::QirProfile`]
/// when `qir` demands the Base Profile and the circuit does not qualify.
#[allow(clippy::too_many_arguments)]
pub fn run_optimize_with(
    path: &Path,
    source: &str,
    bindings: &HashMap<String, f64>,
    selection: &PassSelection,
    emit: &[Stage],
    want_diff: bool,
    target: Option<&BasisProfile>,
    qir: &QirRequest,
    session: &Session,
) -> Result<PipelineReport, CliError> {
    let artifacts = orchestrate(path, source, bindings, selection, qir, session)?;
    let original = artifacts.source_circuit.clone();
    let optimized = artifacts.optimized.clone();

    let mut report = new_report(path, &artifacts, &optimized, session);
    if session.lint {
        report.lint = Some(session.registry.linter().lint(&original, target));
    }
    report.passes = Some(
        artifacts
            .pass_records
            .iter()
            .map(PassRecordView::new)
            .collect(),
    );
    // The optimized circuit is what would actually be submitted, so that is
    // what gets checked against the target.
    report.target = target_for(&optimized, target)?;

    // The pre-optimization circuit, so a reader can see what it started from.
    if emit.contains(&Stage::QcIr) {
        report.stages.push(StageSnapshot {
            stage: "qc-ir".into(),
            metrics: Some(analyze(&original)?),
            instructions: Some(instructions_of(&original)),
            graph: None,
            qir: None,
            qir_info: None,
            unavailable: None,
        });
    }
    report.stages.extend(
        stages_for(&optimized, emit, artifacts.qir.as_ref())?
            .into_iter()
            .map(|mut snapshot| {
                // Distinguish the post-pass circuit from the input above.
                if snapshot.stage == "qc-ir" {
                    snapshot.stage = "optimized".into();
                }
                snapshot
            }),
    );

    if want_diff {
        report.diff = Some(diff_view(&diff_circuits(&original, &optimized)));
    }
    Ok(report)
}

/// Everything [`run_lower`] needs.
///
/// Bundled rather than passed as nine positional arguments, where `want_diff`
/// and `prepare` would sit next to each other as two bare `bool`s — a call
/// site that is easy to get backwards and impossible to read.
pub struct LowerRequest<'a> {
    /// Values for symbolic parameters.
    pub bindings: &'a HashMap<String, f64>,
    /// Which backend to compile for.
    pub backend: &'a str,
    /// How to lower.
    pub lowering: LoweringConfig,
    /// Shots and seed, carried into the provenance record.
    pub settings: ExecutionSettings,
    /// Which stages to snapshot.
    pub emit: &'a [Stage],
    /// Whether to include a before/after diff.
    pub want_diff: bool,
    /// Whether to go on and build the executable.
    pub prepare: bool,
    /// How to emit the canonical QIR output.
    pub qir: QirRequest,
}

/// Runs the whole pipeline through to target lowering, and optionally to a
/// prepared executable.
///
/// The one command that exercises every stage the compiler has. Everything it
/// reports comes from [`crate::compile::compile_named`], so `oqci lower` and a
/// library caller cannot disagree about what happened.
///
/// # Errors
///
/// [`CliError::Compile`] when any stage refuses, or [`CliError::Usage`] for an
/// unknown backend.
pub fn run_lower(
    path: &Path,
    source: &str,
    request: &LowerRequest<'_>,
    session: &Session,
) -> Result<PipelineReport, CliError> {
    let backend = request.backend;
    let emit = request.emit;

    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("main");
    let config = CompilerConfig {
        bindings: request.bindings.clone(),
        backend: Some(backend.to_string()),
        lowering: request.lowering.clone(),
        settings: request.settings.clone(),
        stop: if request.prepare {
            Stop::Prepared
        } else {
            Stop::Lowered
        },
        qir: request.qir,
        extensions: session.registry.clone(),
        lint: session.lint,
        verify: session.verify,
        ..CompilerConfig::default()
    };
    let frontend = frontend_id(path, session);
    let artifacts = crate::compile::compile_with_frontend(source, name, &frontend, &config)?;

    let lowered = artifacts
        .lowered
        .as_ref()
        .expect("lowering ran, so there is a lowered circuit");

    let mut report = new_report(path, &artifacts, &lowered.circuit, session);
    report.lint.clone_from(&artifacts.lint);
    report.passes = Some(
        artifacts
            .pass_records
            .iter()
            .map(PassRecordView::new)
            .collect(),
    );
    report.lowering = Some(lowering_view(backend, lowered));
    report.executable = artifacts.executable.as_ref().map(executable_view);

    // The pre-lowering circuit, so a reader can see what it started from.
    if emit.contains(&Stage::QcIr) {
        report.stages.push(StageSnapshot {
            stage: "qc-ir".into(),
            metrics: Some(artifacts.source_metrics.clone()),
            instructions: Some(instructions_of(&artifacts.source_circuit)),
            graph: None,
            qir: None,
            qir_info: None,
            unavailable: None,
        });
    }
    report.stages.extend(
        stages_for(&lowered.circuit, emit, artifacts.qir.as_ref())?
            .into_iter()
            .map(|mut snapshot| {
                if snapshot.stage == "qc-ir" {
                    snapshot.stage = "lowered".into();
                }
                snapshot
            }),
    );

    // The cost the *target* assigns, not one this tool invented (Stage E).
    //
    // Asked of the backend rather than looked up in `builtin`, which only
    // knows the profiles it enumerates: resolving by id meant a backend with
    // its own profile — `ibm-illustrative` — silently printed no target or
    // cost section at all, with nothing to say why. Asked of the session's
    // registry, so an extension or a loaded target file reports too.
    if let Some(backend) = session.registry.backend(backend) {
        report.target = Some(target_report(
            &lowered.circuit,
            backend.profile(),
            backend.cost_model(),
        )?);
    }

    if request.want_diff {
        report.diff = Some(diff_view(&diff_circuits(
            &artifacts.source_circuit,
            &lowered.circuit,
        )));
    }
    Ok(report)
}

fn new_report(
    path: &Path,
    artifacts: &CompilationArtifacts,
    circuit: &Circuit,
    session: &Session,
) -> PipelineReport {
    PipelineReport {
        source_path: path.display().to_string(),
        frontend: artifacts
            .frontend_id
            .clone()
            .unwrap_or_else(|| frontend_id(path, session)),
        circuit_name: circuit.name().to_string(),
        unbound_parameters: circuit.parameters(),
        extensions: artifacts.extensions.clone(),
        timings: session
            .timings
            .then(|| TimingsView::new(&artifacts.timings)),
        verification: artifacts.verification.clone(),
        ..PipelineReport::default()
    }
}

/// Builds the requested stage snapshots for one circuit.
///
/// The QIR stage is the compilation's canonical artifact, never emitted here:
/// the CLI shows the one module the compiler produced.
fn stages_for(
    circuit: &Circuit,
    emit: &[Stage],
    qir: Option<&QirArtifact>,
) -> Result<Vec<StageSnapshot>, CliError> {
    let mut stages = Vec::new();

    for stage in emit {
        let snapshot = match stage {
            Stage::QcIr => StageSnapshot {
                stage: stage.label().into(),
                metrics: Some(analyze(circuit)?),
                instructions: Some(instructions_of(circuit)),
                graph: None,
                qir: None,
                qir_info: None,
                unavailable: None,
            },
            Stage::QcoIr => StageSnapshot {
                stage: stage.label().into(),
                metrics: None,
                instructions: None,
                graph: Some(graph_of(&qc_to_qco(circuit)?)?),
                qir: None,
                qir_info: None,
                unavailable: None,
            },
            Stage::Qir => {
                // Unbound parameters are a normal state for a parameterized
                // circuit, not a failure — say so rather than aborting.
                if let Some(artifact) = qir {
                    StageSnapshot {
                        stage: stage.label().into(),
                        metrics: None,
                        instructions: None,
                        graph: None,
                        qir: Some(artifact.text.clone()),
                        qir_info: Some(QirInfoView::new(artifact)),
                        unavailable: None,
                    }
                } else {
                    StageSnapshot {
                        stage: stage.label().into(),
                        metrics: None,
                        instructions: None,
                        graph: None,
                        qir: None,
                        qir_info: None,
                        unavailable: Some(format!(
                            "circuit has unbound parameter(s): {}; supply values with --bind NAME=VALUE",
                            circuit.parameters().join(", ")
                        )),
                    }
                }
            }
        };
        stages.push(snapshot);
    }

    Ok(stages)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BELL: &str = r#"
        OPENQASM 3.0;
        include "stdgates.inc";
        qubit[2] q;
        bit[2] c;
        h q[0];
        cx q[0], q[1];
        c = measure q;
    "#;

    fn path() -> &'static Path {
        Path::new("bell.qasm")
    }

    #[test]
    fn compile_reports_every_stage() {
        let report = run_compile(path(), BELL, &HashMap::new(), &Stage::all(), None).unwrap();

        let labels: Vec<&str> = report.stages.iter().map(|s| s.stage.as_str()).collect();
        assert_eq!(labels, vec!["qc-ir", "qco-ir", "qir"]);
        assert_eq!(report.circuit_name, "bell", "named after the file stem");
        assert!(report.passes.is_none());
    }

    #[test]
    fn emit_selection_is_respected() {
        let report = run_compile(path(), BELL, &HashMap::new(), &[Stage::Qir], None).unwrap();
        assert_eq!(report.stages.len(), 1);
        assert!(report.stages[0].qir.is_some());
    }

    #[test]
    fn optimize_records_passes_and_both_circuits() {
        let source = "qubit[1] q; h q[0]; h q[0];";
        let report = run_optimize(
            Path::new("t.qasm"),
            source,
            &HashMap::new(),
            &PassSelection::All,
            &Stage::all(),
            true,
            None,
        )
        .unwrap();

        let labels: Vec<&str> = report.stages.iter().map(|s| s.stage.as_str()).collect();
        assert_eq!(labels, vec!["qc-ir", "optimized", "qco-ir", "qir"]);

        let passes = report.passes.expect("pass records are present");
        assert_eq!(passes.len(), 5);
        assert!(
            passes
                .iter()
                .any(|p| p.id == "gate-cancellation" && p.changed)
        );

        let diff = report.diff.expect("a diff was requested");
        assert_eq!(diff.iter().filter(|e| e.marker == "-").count(), 2);
    }

    #[test]
    fn unbound_parameters_report_rather_than_fail() {
        let source = "input float[64] theta; qubit[1] q; rz(theta) q[0];";
        let report = run_compile(
            Path::new("a.qasm"),
            source,
            &HashMap::new(),
            &Stage::all(),
            None,
        )
        .unwrap();

        assert_eq!(report.unbound_parameters, vec!["theta".to_string()]);
        let qir = report.stages.iter().find(|s| s.stage == "qir").unwrap();
        assert!(qir.qir.is_none());
        assert!(qir.unavailable.as_ref().unwrap().contains("theta"));
    }

    #[test]
    fn bindings_make_a_parameterized_circuit_emittable() {
        let source = "input float[64] theta; qubit[1] q; rz(theta) q[0];";
        let bindings = HashMap::from([("theta".to_string(), 0.5)]);
        let report =
            run_compile(Path::new("a.qasm"), source, &bindings, &Stage::all(), None).unwrap();

        assert!(report.unbound_parameters.is_empty());
        let qir = report.stages.iter().find(|s| s.stage == "qir").unwrap();
        assert!(
            qir.qir
                .as_ref()
                .unwrap()
                .contains("__quantum__qis__rz__body")
        );
    }

    #[test]
    fn a_py_file_is_read_as_a_cudaq_kernel() {
        let source = "import cudaq\n\n@cudaq.kernel\ndef bell():\n    q = cudaq.qvector(2)\n    h(q[0])\n    x.ctrl(q[0], q[1])\n    mz(q)\n";
        let report = run_compile(
            Path::new("bell.py"),
            source,
            &HashMap::new(),
            &Stage::all(),
            None,
        )
        .unwrap();
        assert_eq!(report.frontend, "cudaq");
        assert_eq!(report.circuit_name, "bell");
        let qc = report.stages.iter().find(|s| s.stage == "qc-ir").unwrap();
        assert_eq!(qc.instructions.as_ref().unwrap().len(), 4);
    }

    #[test]
    fn a_frontend_error_surfaces() {
        let error = run_compile(
            path(),
            "qubit[1] q; h q[0]",
            &HashMap::new(),
            &Stage::all(),
            None,
        )
        .unwrap_err();
        // Arrives through the orchestrator now, and `#[error(transparent)]`
        // means the message a user sees is the frontend's own either way.
        assert!(
            matches!(
                error,
                CliError::Compile(crate::compile::CompileError::Frontend { .. })
            ),
            "got {error}"
        );
        assert!(error.to_string().contains("expected"), "{error}");
    }

    #[test]
    fn disabled_passes_change_the_result() {
        let source = "qubit[1] q; h q[0]; h q[0];";
        let report = run_optimize(
            Path::new("t.qasm"),
            source,
            &HashMap::new(),
            &PassSelection::all_except(["gate-cancellation"]),
            &[Stage::QcIr],
            false,
            None,
        )
        .unwrap();

        let optimized = report
            .stages
            .iter()
            .find(|s| s.stage == "optimized")
            .unwrap();
        assert_eq!(optimized.metrics.as_ref().unwrap().op_count, 2);
    }
}
