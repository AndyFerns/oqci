//! The ground-truth compile path.
//!
//! Calls the real, unmodified compiler exactly once per request, through
//! `oqci::compile::compile_named` — the same single entry point the CLI and
//! the Python SDK already go through. This module assembles a
//! `PipelineReport` from the result using the same view-building functions
//! `src/cli/pipeline.rs` uses (`oqci::cli::snapshot::*`), mirroring that
//! module's structure rather than reimplementing it. It must stay short and
//! purely delegating.

use std::collections::HashMap;

use oqci::analysis::diff_circuits;
use oqci::cli::snapshot::{self, PassRecordView, PipelineReport, StageSnapshot};
use oqci::compile::{CompilationArtifacts, CompilerConfig, Stop};
use oqci::ir::{emit_qir, qc_to_qco};

use crate::error::ServerError;

pub struct CompileRequest {
    pub source: String,
    pub name: String,
    pub backend: Option<String>,
    pub bindings: HashMap<String, f64>,
}

pub struct CompileResult {
    pub report: PipelineReport,
    pub artifacts: CompilationArtifacts,
}

/// Runs the real compiler once and builds the JSON report from its result.
pub fn compile(request: &CompileRequest) -> Result<CompileResult, ServerError> {
    let config = CompilerConfig {
        bindings: request.bindings.clone(),
        backend: request.backend.clone(),
        stop: Stop::Prepared,
        ..CompilerConfig::default()
    };
    let artifacts = oqci::compile::compile_named(&request.source, &request.name, &config)?;
    let report = build_report(&request.name, &artifacts)?;
    Ok(CompileResult { report, artifacts })
}

fn build_report(name: &str, artifacts: &CompilationArtifacts) -> Result<PipelineReport, ServerError> {
    let circuit = artifacts.final_circuit();
    let mut report = PipelineReport {
        source_path: name.to_string(),
        frontend: "openqasm3".into(),
        circuit_name: circuit.name().to_string(),
        unbound_parameters: circuit.parameters(),
        stages: Vec::new(),
        passes: Some(
            artifacts
                .pass_records
                .iter()
                .map(PassRecordView::new)
                .collect(),
        ),
        diff: Some(snapshot::diff_view(&diff_circuits(
            &artifacts.source_circuit,
            &artifacts.optimized,
        ))),
        target: None,
        lowering: None,
        executable: None,
    };

    report.stages.push(StageSnapshot {
        stage: "qc-ir".into(),
        metrics: Some(artifacts.source_metrics.clone()),
        instructions: Some(snapshot::instructions_of(&artifacts.source_circuit)),
        graph: None,
        qir: None,
        unavailable: None,
    });

    report.stages.push(StageSnapshot {
        stage: "optimized".into(),
        metrics: Some(artifacts.optimized_metrics.clone()),
        instructions: Some(snapshot::instructions_of(&artifacts.optimized)),
        graph: None,
        qir: None,
        unavailable: None,
    });

    let dag = qc_to_qco(&artifacts.optimized)?;
    report.stages.push(StageSnapshot {
        stage: "qco-ir".into(),
        metrics: None,
        instructions: None,
        graph: Some(snapshot::graph_of(&dag)?),
        qir: None,
        unavailable: None,
    });

    if artifacts.optimized.is_concrete() {
        report.stages.push(StageSnapshot {
            stage: "qir".into(),
            metrics: None,
            instructions: None,
            graph: None,
            qir: Some(emit_qir(&dag)?),
            unavailable: None,
        });
    } else {
        report.stages.push(StageSnapshot {
            stage: "qir".into(),
            metrics: None,
            instructions: None,
            graph: None,
            qir: None,
            unavailable: Some(format!(
                "circuit has unbound parameter(s): {}; supply bindings to enable QIR emission",
                artifacts.optimized.parameters().join(", ")
            )),
        });
    }

    if let Some(lowered) = &artifacts.lowered {
        report.stages.push(StageSnapshot {
            stage: "lowered".into(),
            metrics: None,
            instructions: Some(snapshot::instructions_of(&lowered.circuit)),
            graph: None,
            qir: None,
            unavailable: None,
        });

        let backend_id = artifacts.backend_id.clone().unwrap_or_default();
        report.lowering = Some(snapshot::lowering_view(&backend_id, lowered));

        if let Some(backend) = oqci::backend::by_id(&backend_id) {
            report.target = Some(snapshot::target_report(
                &lowered.circuit,
                backend.profile(),
                backend.cost_model(),
            )?);
        }
    }

    report.executable = artifacts.executable.as_ref().map(snapshot::executable_view);

    Ok(report)
}
