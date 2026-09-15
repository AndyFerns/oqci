//! The shared pipeline runner.
//!
//! `final-deliverables-spec.md` §19: the CLI "must not duplicate compiler
//! logic that belongs in the Rust library — it should invoke the same public
//! compiler APIs." These two functions are the *only* place in the CLI that
//! touches the compiler, and they do nothing but call library functions and
//! record what came back:
//!
//! - [`crate::frontend::parse_openqasm3_named`] — source to QC-IR
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
use crate::cli::CliError;
use crate::cli::snapshot::{
    PassRecordView, PipelineReport, StageSnapshot, diff_view, graph_of, instructions_of,
};
use crate::frontend::parse_openqasm3_named;
use crate::ir::{Circuit, bind_parameters, emit_qir, qc_to_qco};
use crate::pass::{PassManager, PassSelection};

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

/// Parses a source file into QC-IR, applying any `--bind` values.
///
/// The circuit's name comes from the file stem, so the emitted QIR entry
/// point matches the file a reader is looking at.
///
/// # Errors
///
/// [`CliError::Frontend`] for a malformed or unsupported program, or
/// [`CliError::Ir`] if a binding is rejected.
pub fn parse(
    path: &Path,
    source: &str,
    bindings: &HashMap<String, f64>,
) -> Result<Circuit, CliError> {
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("main");
    let circuit = parse_openqasm3_named(source, name)?;

    if bindings.is_empty() {
        return Ok(circuit);
    }
    Ok(bind_parameters(&circuit, bindings)?)
}

/// Runs frontend → QC-IR → QCO-IR → QIR, recording each requested stage.
///
/// # Errors
///
/// [`CliError`] from any stage. A circuit with unbound parameters is **not**
/// an error: the QIR stage records why it could not run, so `watch` stays
/// useful while an ansatz is being written.
pub fn run_compile(
    path: &Path,
    source: &str,
    bindings: &HashMap<String, f64>,
    emit: &[Stage],
) -> Result<PipelineReport, CliError> {
    let circuit = parse(path, source, bindings)?;
    let mut report = new_report(path, &circuit);
    report.stages = stages_for(&circuit, emit)?;
    Ok(report)
}

/// Runs frontend → QC-IR → pass pipeline → QCO-IR → QIR.
///
/// # Errors
///
/// [`CliError`] from the frontend, a pass, or lowering.
pub fn run_optimize(
    path: &Path,
    source: &str,
    bindings: &HashMap<String, f64>,
    selection: &PassSelection,
    emit: &[Stage],
    want_diff: bool,
) -> Result<PipelineReport, CliError> {
    let original = parse(path, source, bindings)?;
    let result = PassManager::default_pipeline().run(&original, selection)?;

    let mut report = new_report(path, &result.circuit);
    report.passes = Some(result.records.iter().map(PassRecordView::new).collect());

    // The pre-optimization circuit, so a reader can see what it started from.
    if emit.contains(&Stage::QcIr) {
        report.stages.push(StageSnapshot {
            stage: "qc-ir".into(),
            metrics: Some(analyze(&original)?),
            instructions: Some(instructions_of(&original)),
            graph: None,
            qir: None,
            unavailable: None,
        });
    }
    report.stages.extend(
        stages_for(&result.circuit, emit)?
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
        report.diff = Some(diff_view(&diff_circuits(&original, &result.circuit)));
    }
    Ok(report)
}

fn new_report(path: &Path, circuit: &Circuit) -> PipelineReport {
    PipelineReport {
        source_path: path.display().to_string(),
        frontend: "openqasm3".into(),
        circuit_name: circuit.name().to_string(),
        unbound_parameters: circuit.parameters(),
        stages: Vec::new(),
        passes: None,
        diff: None,
    }
}

/// Builds the requested stage snapshots for one circuit.
fn stages_for(circuit: &Circuit, emit: &[Stage]) -> Result<Vec<StageSnapshot>, CliError> {
    let mut stages = Vec::new();

    for stage in emit {
        let snapshot = match stage {
            Stage::QcIr => StageSnapshot {
                stage: stage.label().into(),
                metrics: Some(analyze(circuit)?),
                instructions: Some(instructions_of(circuit)),
                graph: None,
                qir: None,
                unavailable: None,
            },
            Stage::QcoIr => StageSnapshot {
                stage: stage.label().into(),
                metrics: None,
                instructions: None,
                graph: Some(graph_of(&qc_to_qco(circuit)?)?),
                qir: None,
                unavailable: None,
            },
            Stage::Qir => {
                // Unbound parameters are a normal state for a parameterized
                // circuit, not a failure — say so rather than aborting.
                if circuit.is_concrete() {
                    StageSnapshot {
                        stage: stage.label().into(),
                        metrics: None,
                        instructions: None,
                        graph: None,
                        qir: Some(emit_qir(&qc_to_qco(circuit)?)?),
                        unavailable: None,
                    }
                } else {
                    StageSnapshot {
                        stage: stage.label().into(),
                        metrics: None,
                        instructions: None,
                        graph: None,
                        qir: None,
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
        let report = run_compile(path(), BELL, &HashMap::new(), &Stage::all()).unwrap();

        let labels: Vec<&str> = report.stages.iter().map(|s| s.stage.as_str()).collect();
        assert_eq!(labels, vec!["qc-ir", "qco-ir", "qir"]);
        assert_eq!(report.circuit_name, "bell", "named after the file stem");
        assert!(report.passes.is_none());
    }

    #[test]
    fn emit_selection_is_respected() {
        let report = run_compile(path(), BELL, &HashMap::new(), &[Stage::Qir]).unwrap();
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
        let report =
            run_compile(Path::new("a.qasm"), source, &HashMap::new(), &Stage::all()).unwrap();

        assert_eq!(report.unbound_parameters, vec!["theta".to_string()]);
        let qir = report.stages.iter().find(|s| s.stage == "qir").unwrap();
        assert!(qir.qir.is_none());
        assert!(qir.unavailable.as_ref().unwrap().contains("theta"));
    }

    #[test]
    fn bindings_make_a_parameterized_circuit_emittable() {
        let source = "input float[64] theta; qubit[1] q; rz(theta) q[0];";
        let bindings = HashMap::from([("theta".to_string(), 0.5)]);
        let report = run_compile(Path::new("a.qasm"), source, &bindings, &Stage::all()).unwrap();

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
    fn a_frontend_error_surfaces() {
        let error =
            run_compile(path(), "qubit[1] q; h q[0]", &HashMap::new(), &Stage::all()).unwrap_err();
        assert!(matches!(error, CliError::Frontend(_)));
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
