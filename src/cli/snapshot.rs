//! The stage-snapshot schema: what the compiler did, as data.
//!
//! Everything the CLI prints — human-readable or JSON — is rendered from a
//! [`PipelineReport`]. Holding the report as data first, and formatting it
//! second, is what lets `--json` and the default view be the same information
//! rather than two independently-assembled stories about the same run. It is
//! also the contract a future dashboard consumes: point it at
//! `oqci … --json` and it has everything this terminal shows.
//!
//! These types are **views**: serde-friendly mirrors of the IR and pass
//! types, built here rather than derived on the originals. The IR stays
//! serde-free deliberately — its shape is a compiler contract governed by
//! `docs/ir_spec.md`, and it should not acquire a wire format by accident.
//! [`crate::analysis::ResourceReport`] is the exception, serialized directly:
//! it *is* the report type, and mirroring it would create exactly the kind of
//! drift this module exists to avoid.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::analysis::{CircuitDiff, ResourceReport};
use crate::ir::{Circuit, GateKind, Instruction, Param, QcoCircuit, Wire};
use crate::pass::PassRecord;
use crate::target::{BasisProfile, Cost, CostModel, Violation, check};

/// A full run of the compiler over one input.
#[derive(Debug, Clone, Serialize)]
pub struct PipelineReport {
    /// Path the source was read from.
    pub source_path: String,
    /// Which frontend parsed it.
    pub frontend: String,
    /// The circuit's name.
    pub circuit_name: String,
    /// Free symbolic parameters remaining after any `--bind` values were
    /// applied. Non-empty means QIR cannot be emitted yet.
    pub unbound_parameters: Vec<String>,
    /// One entry per pipeline stage, in order.
    pub stages: Vec<StageSnapshot>,
    /// Per-pass records, when an optimization pipeline ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passes: Option<Vec<PassRecordView>>,
    /// Whole-pipeline before/after diff, when requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<Vec<DiffEntryView>>,
    /// Target legality and cost, when `--target` named a profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<TargetReportView>,
}

/// How a circuit fares against a specific backend target.
#[derive(Debug, Clone, Serialize)]
pub struct TargetReportView {
    /// The profile's `id@version` — the form that belongs in result
    /// provenance (Stage D §8).
    pub profile: String,
    /// The backend the profile describes.
    pub backend_id: String,
    /// Physical qubits available.
    pub qubit_count: u32,
    /// Directed couplings declared.
    pub edge_count: usize,
    /// The target's native operation set.
    pub supported_operations: Vec<String>,
    /// Whether the circuit runs on this target as written.
    pub legal: bool,
    /// Every violation found, not just the first.
    pub violations: Vec<Violation>,
    /// The cost model that produced `cost`, as `id@version`.
    pub cost_model: String,
    /// The cost model's configuration — weights included, so a scalar score is
    /// never an unexplained figure (Stage E §6).
    pub cost_model_configuration: BTreeMap<String, String>,
    /// The structured cost breakdown.
    pub cost: Cost,
}

/// One stage's output.
#[derive(Debug, Clone, Serialize)]
pub struct StageSnapshot {
    /// `"qc-ir"`, `"qco-ir"`, `"qir"`, or `"optimized"`.
    pub stage: String,
    /// Metrics for the circuit at this stage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<ResourceReport>,
    /// The instruction list, for circuit-shaped stages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<Vec<InstructionView>>,
    /// The dependency graph, for the QCO-IR stage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<GraphView>,
    /// Emitted QIR text, for the QIR stage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qir: Option<String>,
    /// Why this stage produced nothing, when it could not run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
}

/// One instruction.
#[derive(Debug, Clone, Serialize)]
pub struct InstructionView {
    /// Position in program order.
    pub index: usize,
    /// `"gate"`, `"measure"` or `"reset"`.
    pub op: String,
    /// Gate mnemonic, for gates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    /// Gate parameters, in canonical order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamView>,
    /// Qubit operands, in order.
    pub qubits: Vec<u32>,
    /// Destination classical bit, for measurements.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clbit: Option<u32>,
    /// One-line rendering, e.g. `"rz(0.5) %q0"`.
    pub text: String,
}

/// A gate parameter.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ParamView {
    /// A bound numeric angle.
    Concrete {
        /// The value, in radians.
        radians: f64,
    },
    /// An unbound symbolic parameter.
    Symbol {
        /// The parameter's name.
        name: String,
    },
}

/// The QCO-IR dependency graph.
#[derive(Debug, Clone, Serialize)]
pub struct GraphView {
    /// Operation nodes (boundaries excluded).
    pub op_count: usize,
    /// ASAP scheduling depth.
    pub depth: usize,
    /// Widest scheduling layer.
    pub max_parallel_width: usize,
    /// Program indices grouped into ASAP layers.
    pub layers: Vec<Vec<usize>>,
    /// Dependency edges.
    pub edges: Vec<EdgeView>,
}

/// One dependency edge.
#[derive(Debug, Clone, Serialize)]
pub struct EdgeView {
    /// Source node label, e.g. `"in:%q0"` or `"op:2"`.
    pub from: String,
    /// Destination node label.
    pub to: String,
    /// The wire carrying the dependency, e.g. `"%q0"`.
    pub wire: String,
    /// `"data"` or `"control"`.
    pub kind: String,
}

/// What one pass did.
#[derive(Debug, Clone, Serialize)]
pub struct PassRecordView {
    /// The pass's id.
    pub id: String,
    /// The pass's description.
    pub description: String,
    /// Whether the selection enabled it.
    pub enabled: bool,
    /// Whether it changed the circuit.
    pub changed: bool,
    /// Metrics before it ran.
    pub before: ResourceReport,
    /// Metrics after it ran.
    pub after: ResourceReport,
    /// Time spent, in microseconds.
    pub duration_us: u128,
    /// The pass's own notes.
    pub notes: Vec<String>,
}

/// One aligned position in a circuit diff.
#[derive(Debug, Clone, Serialize)]
pub struct DiffEntryView {
    /// `" "`, `"-"` or `"+"`.
    pub marker: String,
    /// The instruction at this position.
    pub text: String,
}

// --- Construction -----------------------------------------------------------

impl InstructionView {
    /// Builds a view of one instruction.
    pub fn new(index: usize, inst: &Instruction) -> Self {
        let text = format_instruction(inst);
        match inst {
            Instruction::Gate { kind, qubits } => InstructionView {
                index,
                op: "gate".into(),
                gate: Some(kind.mnemonic().to_string()),
                params: kind.params().iter().map(ParamView::new).collect(),
                qubits: qubits.iter().map(|q| q.index()).collect(),
                clbit: None,
                text,
            },
            Instruction::Measure { qubit, target } => InstructionView {
                index,
                op: "measure".into(),
                gate: None,
                params: Vec::new(),
                qubits: vec![qubit.index()],
                clbit: Some(target.index()),
                text,
            },
            Instruction::Reset { qubit } => InstructionView {
                index,
                op: "reset".into(),
                gate: None,
                params: Vec::new(),
                qubits: vec![qubit.index()],
                clbit: None,
                text,
            },
        }
    }
}

impl ParamView {
    fn new(param: &Param) -> Self {
        match param {
            Param::Concrete(angle) => ParamView::Concrete {
                radians: angle.radians(),
            },
            Param::Symbol(name) => ParamView::Symbol { name: name.clone() },
        }
    }
}

/// Renders an instruction as a single line, e.g. `"cx %q0, %q1"`.
pub fn format_instruction(inst: &Instruction) -> String {
    match inst {
        Instruction::Gate { kind, qubits } => {
            let operands: Vec<String> = qubits.iter().map(ToString::to_string).collect();
            format!("{} {}", format_gate(kind), operands.join(", "))
        }
        Instruction::Measure { qubit, target } => format!("measure {qubit} -> {target}"),
        Instruction::Reset { qubit } => format!("reset {qubit}"),
    }
}

/// `GateKind`'s `Display` already renders `mnemonic(params…)`; this keeps the
/// call site readable.
fn format_gate(kind: &GateKind) -> String {
    kind.to_string()
}

/// Builds the instruction list for a circuit.
pub fn instructions_of(circuit: &Circuit) -> Vec<InstructionView> {
    circuit
        .instructions()
        .iter()
        .enumerate()
        .map(|(i, inst)| InstructionView::new(i, inst))
        .collect()
}

/// Builds a graph view from QCO-IR.
///
/// # Errors
///
/// Propagates [`crate::ir::IrError`] from the layering computation.
pub fn graph_of(dag: &QcoCircuit) -> Result<GraphView, crate::ir::IrError> {
    use crate::ir::{DepKind, NodeKind, QcoNode};

    fn label(node: &QcoNode) -> String {
        match &node.kind {
            NodeKind::Input(wire) => format!("in:{}", wire_label(wire)),
            NodeKind::Output(wire) => format!("out:{}", wire_label(wire)),
            NodeKind::Op { index, .. } => format!("op:{index}"),
        }
    }

    let layers = dag.layers()?;
    let edges = dag
        .dependencies()
        .map(|(from, to, dep)| EdgeView {
            from: label(from),
            to: label(to),
            wire: wire_label(&dep.wire),
            kind: match dep.kind {
                DepKind::Data => "data".into(),
                DepKind::Control => "control".into(),
            },
        })
        .collect();

    Ok(GraphView {
        op_count: dag.op_count(),
        max_parallel_width: layers.iter().map(Vec::len).max().unwrap_or(0),
        depth: layers.len(),
        layers,
        edges,
    })
}

fn wire_label(wire: &Wire) -> String {
    match wire {
        Wire::Qubit(q) => q.to_string(),
        Wire::Clbit(c) => c.to_string(),
    }
}

impl PassRecordView {
    /// Builds a view of one pass record.
    pub fn new(record: &PassRecord) -> Self {
        PassRecordView {
            id: record.id.clone(),
            description: record.description.clone(),
            enabled: record.enabled,
            changed: record.changed,
            before: record.before.clone(),
            after: record.after.clone(),
            duration_us: record.duration.as_micros(),
            notes: record.notes.clone(),
        }
    }
}

/// Checks a circuit against a target and evaluates its cost.
///
/// Both halves come from `crate::target` — nothing is judged or scored here.
///
/// # Errors
///
/// Propagates [`crate::ir::IrError`] from the cost model's analysis.
pub fn target_report(
    circuit: &Circuit,
    profile: &BasisProfile,
    cost_model: &dyn CostModel,
) -> Result<TargetReportView, crate::ir::IrError> {
    let report = check(circuit, profile);
    Ok(TargetReportView {
        profile: profile.qualified_id(),
        backend_id: profile.backend_id().to_string(),
        qubit_count: profile.qubit_count(),
        edge_count: profile.topology().edge_count(),
        supported_operations: profile
            .supported_operations()
            .into_iter()
            .map(ToString::to_string)
            .collect(),
        legal: report.is_legal(),
        violations: report.violations,
        cost_model: format!("{}@{}", cost_model.id(), cost_model.version()),
        cost_model_configuration: cost_model.configuration(),
        cost: cost_model.evaluate(circuit, profile)?,
    })
}

/// Builds a view of a circuit diff.
pub fn diff_view(diff: &CircuitDiff) -> Vec<DiffEntryView> {
    diff.entries
        .iter()
        .map(|entry| DiffEntryView {
            marker: entry.marker().to_string(),
            text: format_instruction(entry.instruction()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CircuitBuilder, ClbitId, QubitId};

    #[test]
    fn instructions_render_readably() {
        let mut b = CircuitBuilder::new("c");
        b.alloc_qubits(2);
        b.alloc_clbits(1);
        b.h(QubitId(0))
            .cx(QubitId(0), QubitId(1))
            .rz(0.5, QubitId(0))
            .measure(QubitId(0), ClbitId(0))
            .reset(QubitId(1));
        let circuit = b.build().unwrap();

        let texts: Vec<String> = instructions_of(&circuit)
            .into_iter()
            .map(|v| v.text)
            .collect();
        assert_eq!(
            texts,
            vec![
                "h %q0",
                "cx %q0, %q1",
                "rz(0.5) %q0",
                "measure %q0 -> %c0",
                "reset %q1",
            ]
        );
    }

    #[test]
    fn symbolic_parameters_are_distinguishable_in_the_schema() {
        let mut b = CircuitBuilder::new("c");
        b.alloc_qubits(1);
        b.rz(Param::symbol("theta"), QubitId(0));
        let circuit = b.build().unwrap();

        let view = &instructions_of(&circuit)[0];
        assert!(matches!(
            view.params.as_slice(),
            [ParamView::Symbol { name }] if name == "theta"
        ));
        assert_eq!(view.text, "rz(%theta) %q0");
    }

    #[test]
    fn graph_view_labels_boundaries_and_ops() {
        let mut b = CircuitBuilder::new("c");
        b.alloc_qubits(1);
        b.h(QubitId(0));
        let dag = crate::ir::qc_to_qco(&b.build().unwrap()).unwrap();

        let view = graph_of(&dag).unwrap();
        assert_eq!(view.op_count, 1);
        assert_eq!(view.depth, 1);
        assert!(
            view.edges
                .iter()
                .any(|e| e.from == "in:%q0" && e.to == "op:0")
        );
        assert!(
            view.edges
                .iter()
                .any(|e| e.from == "op:0" && e.to == "out:%q0")
        );
        assert!(view.edges.iter().all(|e| e.kind == "data"));
    }

    #[test]
    fn control_edges_are_labelled() {
        let mut b = CircuitBuilder::new("c");
        b.alloc_qubits(1);
        b.alloc_clbits(1);
        b.x(QubitId(0))
            .measure(QubitId(0), ClbitId(0))
            .x(QubitId(0));
        let dag = crate::ir::qc_to_qco(&b.build().unwrap()).unwrap();

        let view = graph_of(&dag).unwrap();
        assert!(view.edges.iter().any(|e| e.kind == "control"));
    }

    #[test]
    fn a_report_serializes_to_json() {
        let report = PipelineReport {
            source_path: "x.qasm".into(),
            frontend: "openqasm3".into(),
            circuit_name: "x".into(),
            unbound_parameters: vec![],
            stages: vec![],
            passes: None,
            diff: None,
            target: None,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"frontend\":\"openqasm3\""));
        assert!(
            !json.contains("passes"),
            "absent sections are omitted, not null"
        );
    }
}
