//! Rendering a [`PipelineReport`] for a human or for a machine.
//!
//! Both renderers read the same report, so `--json` and the default view can
//! never tell different stories about a run. Nothing here computes anything:
//! if a number appears on screen, [`crate::cli::pipeline`] already got it from
//! [`crate::analysis`].
//!
//! The human renderer is plain text — no ANSI colour. Output from this tool
//! gets pasted into issues, diffed, and piped into files, and escape codes
//! survive none of that well.

use std::fmt::Write as _;

use crate::cli::CliError;
use crate::cli::snapshot::{PipelineReport, StageSnapshot};

/// Serializes a report as pretty-printed JSON.
///
/// # Errors
///
/// [`CliError::Json`] if serialization fails.
pub fn json(report: &PipelineReport) -> Result<String, CliError> {
    Ok(serde_json::to_string_pretty(report)?)
}

/// Renders a report as readable text.
#[must_use]
pub fn human(report: &PipelineReport) -> String {
    let mut out = String::new();

    let _ = writeln!(
        out,
        "== {} ({} via {})",
        report.source_path, report.circuit_name, report.frontend
    );
    if !report.unbound_parameters.is_empty() {
        let _ = writeln!(
            out,
            "   unbound parameter(s): {}",
            report.unbound_parameters.join(", ")
        );
    }

    if let Some(passes) = &report.passes {
        out.push_str("\n-- passes --\n");
        for pass in passes {
            let status = match (pass.enabled, pass.changed) {
                (false, _) => "skip",
                (true, true) => "  ok",
                (true, false) => "  --",
            };
            let _ = writeln!(
                out,
                "  [{status}] {:<18} {} -> {} ops, depth {} -> {}",
                pass.id,
                pass.before.op_count,
                pass.after.op_count,
                pass.before.depth,
                pass.after.depth
            );
            for note in &pass.notes {
                let _ = writeln!(out, "           {note}");
            }
        }
    }

    for stage in &report.stages {
        render_stage(&mut out, stage);
    }

    if let Some(diff) = &report.diff {
        out.push_str("\n-- diff --\n");
        if diff.iter().all(|e| e.marker == " ") {
            out.push_str("  (no change)\n");
        } else {
            for entry in diff {
                let _ = writeln!(out, "  {} {}", entry.marker, entry.text);
            }
        }
    }

    out
}

fn render_stage(out: &mut String, stage: &StageSnapshot) {
    let _ = write!(out, "\n-- {} --", stage.stage);

    if let Some(metrics) = &stage.metrics {
        let _ = write!(
            out,
            " {} qubit(s), {} clbit(s), {} op(s), depth {}",
            metrics.num_qubits, metrics.num_clbits, metrics.op_count, metrics.depth
        );
    }
    out.push('\n');

    if let Some(reason) = &stage.unavailable {
        let _ = writeln!(out, "  unavailable: {reason}");
    }

    if let Some(instructions) = &stage.instructions {
        if instructions.is_empty() {
            out.push_str("  (empty circuit)\n");
        }
        for inst in instructions {
            let _ = writeln!(out, "  {:>3}  {}", inst.index, inst.text);
        }
    }

    if let Some(graph) = &stage.graph {
        let _ = writeln!(
            out,
            "  {} op node(s), depth {}, max parallel width {}",
            graph.op_count, graph.depth, graph.max_parallel_width
        );
        for (i, layer) in graph.layers.iter().enumerate() {
            let indices: Vec<String> = layer.iter().map(ToString::to_string).collect();
            let _ = writeln!(out, "  layer {i}: {}", indices.join(", "));
        }
        // Control edges between two operations are the ones that actually
        // constrain optimization, so surface those rather than burying them in
        // the full edge list. An edge into an output boundary is also marked
        // control when the wire ends on a collapse, but it constrains nothing
        // — listing it under "barriers" would be noise.
        let control: Vec<&crate::cli::snapshot::EdgeView> = graph
            .edges
            .iter()
            .filter(|e| e.kind == "control" && e.to.starts_with("op:"))
            .collect();
        if !control.is_empty() {
            let _ = writeln!(out, "  control barriers:");
            for edge in control {
                let _ = writeln!(out, "    {} -> {} on {}", edge.from, edge.to, edge.wire);
            }
        }
    }

    if let Some(qir) = &stage.qir {
        for line in qir.lines() {
            let _ = writeln!(out, "  {line}");
        }
    }
}

/// Renders a resource report on its own, for `oqci analyze`.
#[must_use]
pub fn resource_report(report: &PipelineReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "== {} ({})", report.source_path, report.circuit_name);

    // When an optimization pipeline ran, the report carries both the parsed
    // and the optimized circuit. Report the optimized one — it is what the
    // caller asked to measure.
    let metrics = report
        .stages
        .iter()
        .find(|s| s.stage == "optimized")
        .or_else(|| report.stages.iter().find(|s| s.metrics.is_some()))
        .and_then(|s| s.metrics.as_ref());
    let Some(metrics) = metrics else {
        return out;
    };

    let _ = writeln!(out, "  qubits              {}", metrics.num_qubits);
    let _ = writeln!(out, "  clbits              {}", metrics.num_clbits);
    let _ = writeln!(out, "  operations          {}", metrics.op_count);
    let _ = writeln!(out, "  one-qubit gates     {}", metrics.one_qubit_count);
    let _ = writeln!(out, "  multi-qubit gates   {}", metrics.two_qubit_count);
    let _ = writeln!(out, "  measurements        {}", metrics.measure_count);
    let _ = writeln!(out, "  resets              {}", metrics.reset_count);
    let _ = writeln!(out, "  depth               {}", metrics.depth);
    let _ = writeln!(out, "  max parallel width  {}", metrics.max_parallel_width);

    if !metrics.gate_counts.is_empty() {
        out.push_str("  by operation:\n");
        for (name, count) in &metrics.gate_counts {
            let _ = writeln!(out, "    {name:<10} {count}");
        }
    }
    if !report.unbound_parameters.is_empty() {
        let _ = writeln!(
            out,
            "  unbound parameters  {}",
            report.unbound_parameters.join(", ")
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use super::*;
    use crate::cli::pipeline::{Stage, run_compile, run_optimize};
    use crate::pass::PassSelection;

    const BELL: &str = r#"
        qubit[2] q;
        bit[2] c;
        h q[0];
        cx q[0], q[1];
        c = measure q;
    "#;

    fn compiled() -> PipelineReport {
        run_compile(Path::new("bell.qasm"), BELL, &HashMap::new(), &Stage::all()).unwrap()
    }

    #[test]
    fn human_output_shows_every_stage_and_instruction() {
        let text = human(&compiled());
        assert!(text.contains("-- qc-ir --"));
        assert!(text.contains("-- qco-ir --"));
        assert!(text.contains("-- qir --"));
        assert!(text.contains("h %q0"));
        assert!(text.contains("cx %q0, %q1"));
        assert!(text.contains("@__quantum__qis__h__body"));
    }

    #[test]
    fn human_output_carries_no_ansi_escapes() {
        assert!(!human(&compiled()).contains('\u{1b}'));
    }

    #[test]
    fn json_output_parses_and_keeps_the_schema() {
        let text = json(&compiled()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["frontend"], "openqasm3");
        assert_eq!(value["circuit_name"], "bell");
        assert_eq!(value["stages"][0]["stage"], "qc-ir");
        assert_eq!(value["stages"][0]["instructions"][0]["gate"], "h");
        assert_eq!(value["stages"][0]["metrics"]["depth"], 3);
    }

    #[test]
    fn pass_table_reports_skipped_and_changed_passes() {
        let report = run_optimize(
            Path::new("t.qasm"),
            "qubit[1] q; h q[0]; h q[0];",
            &HashMap::new(),
            &PassSelection::all_except(["rotation-merge"]),
            &[Stage::QcIr],
            true,
        )
        .unwrap();

        let text = human(&report);
        assert!(text.contains("-- passes --"));
        assert!(text.contains("[skip] rotation-merge"));
        assert!(text.contains("cancelled 1 inverse pair(s)"));
        assert!(text.contains("-- diff --"));
        assert!(text.contains("- h %q0"));
    }

    #[test]
    fn a_circuit_with_no_changes_says_so() {
        let report = run_optimize(
            Path::new("t.qasm"),
            "qubit[2] q; h q[0]; cx q[0], q[1];",
            &HashMap::new(),
            &PassSelection::All,
            &[],
            true,
        )
        .unwrap();
        assert!(human(&report).contains("(no change)"));
    }

    #[test]
    fn unbound_parameters_are_announced() {
        let report = run_compile(
            Path::new("a.qasm"),
            "input float[64] theta; qubit[1] q; rz(theta) q[0];",
            &HashMap::new(),
            &Stage::all(),
        )
        .unwrap();

        let text = human(&report);
        assert!(text.contains("unbound parameter(s): theta"));
        assert!(text.contains("unavailable:"));
    }

    #[test]
    fn analyze_output_lists_metrics() {
        let text = resource_report(&compiled());
        assert!(text.contains("qubits              2"));
        assert!(text.contains("depth               3"));
        assert!(text.contains("measure"));
    }

    #[test]
    fn analyze_of_an_optimized_run_reports_the_optimized_circuit() {
        // The report holds both circuits; measuring the input rather than the
        // optimized result would silently answer a different question.
        let report = run_optimize(
            Path::new("t.qasm"),
            "qubit[1] q; h q[0]; x q[0]; x q[0];",
            &HashMap::new(),
            &PassSelection::All,
            &[Stage::QcIr],
            false,
        )
        .unwrap();

        let text = resource_report(&report);
        assert!(text.contains("operations          1"), "got:\n{text}");
    }

    #[test]
    fn control_barriers_are_surfaced_in_the_graph_view() {
        let report = run_compile(
            Path::new("m.qasm"),
            "qubit[1] q; bit[1] c; x q[0]; measure q[0] -> c[0]; x q[0];",
            &HashMap::new(),
            &[Stage::QcoIr],
        )
        .unwrap();
        assert!(human(&report).contains("control barriers:"));
    }

    #[test]
    fn an_empty_circuit_renders_without_panicking() {
        let report = run_compile(
            Path::new("e.qasm"),
            "OPENQASM 3.0;",
            &HashMap::new(),
            &Stage::all(),
        )
        .unwrap();
        assert!(human(&report).contains("(empty circuit)"));
    }
}
