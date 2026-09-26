//! Fine-grained visualization events and the server's WebSocket message
//! envelope.
//!
//! Every type here is server-owned — none of it lives in, or is derived by
//! modifying, the `oqci` compiler crate. Every *value* placed into these
//! types comes from calling the compiler's own public functions (see
//! `replay.rs`) and, for [`PipelineReport`], from `oqci::cli::snapshot` —
//! the exact view-building code the CLI's `--json` flag already uses.

use oqci::analysis::ResourceReport;
pub use oqci::cli::snapshot::{InstructionView, PipelineReport};
use serde::Serialize;

/// One optimization pass's circuit, before and after — reconstructed by
/// `replay::replay_passes` from repeated calls to the real, unmodified
/// `PassManager`, never by modifying `PassRecord` itself.
#[derive(Debug, Clone, Serialize)]
pub struct PassStepEvent {
    /// Position in the pipeline (0-based).
    pub pass_index: usize,
    /// The pass's id, e.g. `"gate-cancellation"`.
    pub id: String,
    pub enabled: bool,
    pub changed: bool,
    pub before: ResourceReport,
    pub after: ResourceReport,
    pub instructions_before: Vec<InstructionView>,
    pub instructions_after: Vec<InstructionView>,
    pub duration_us: u128,
    pub notes: Vec<String>,
}

/// One replayed run of the pass pipeline.
///
/// `degraded: true` means the server's own repeated `PassManager` calls
/// produced a result that did not match the real, single, ground-truth
/// `PassManager::run` call — in that case `steps` is empty and the frontend
/// should fall back to `PipelineReport.passes` (the always-correct, coarse
/// per-pass metrics the CLI itself would show).
#[derive(Debug, Clone, Serialize, Default)]
pub struct PassReplay {
    pub steps: Vec<PassStepEvent>,
    pub degraded: bool,
    pub degraded_reason: Option<String>,
}

/// One SWAP inserted during routing, reconstructed by diffing the real
/// router's output against its input — the router's decision, just read
/// back, never re-decided.
#[derive(Debug, Clone, Serialize)]
pub struct SwapEvent {
    /// Order among all swaps in this routing run.
    pub sequence: usize,
    pub physical_a: u32,
    pub physical_b: u32,
    /// Logical -> physical assignment, ascending by logical qubit, after
    /// this swap (computed by replaying `Layout::swap_physical`, a public,
    /// pure method — see `replay.rs`).
    pub layout_after: Vec<u32>,
}

/// One decomposition rule's firing on one original instruction.
///
/// Reconstructed by calling the real, unmodified `decompose()` once per
/// source instruction rather than once for a whole list — provably
/// equivalent to the real compiler's own call because a decomposition rule
/// may only permute the operands it was given (the project's own documented
/// and tested I3 invariant), so no cross-instruction interaction exists for
/// this replay to get wrong.
#[derive(Debug, Clone, Serialize)]
pub struct RuleFiringEvent {
    pub sequence: usize,
    /// Which of `lower()`'s three decompose scopes this belongs to:
    /// `"arity-reduction"`, `"basis-decomposition"`, or
    /// `"single-qubit-cleanup"`.
    pub scope: String,
    /// Index of the source instruction in the circuit this scope started
    /// from.
    pub source_index: usize,
    pub source_mnemonic: String,
    pub source_qubits: Vec<u32>,
    pub expanded_into: Vec<InstructionView>,
    pub rules_applied: Vec<String>,
}

/// One of `lower()`'s seven schedule steps, replayed.
#[derive(Debug, Clone, Serialize)]
pub struct LoweringStepReplay {
    pub id: String,
    pub op_count: usize,
    pub detail: String,
    pub instructions: Vec<InstructionView>,
    /// Populated only for the `"routing"` step.
    pub swap_events: Vec<SwapEvent>,
    /// Populated for the three decompose-scoped steps (`"arity-reduction"`,
    /// `"basis-decomposition"`, `"single-qubit-cleanup"`).
    pub rule_firings: Vec<RuleFiringEvent>,
}

/// One replayed run of `lower()`.
///
/// `degraded: true` means the server's step-by-step reconstruction did not
/// match the real, single, ground-truth `lower()` call's final circuit or
/// aggregate counts — in that case `steps` is empty and the frontend should
/// fall back to `PipelineReport.lowering` (the coarse, always-correct
/// 7-step summary the CLI itself would show).
#[derive(Debug, Clone, Serialize, Default)]
pub struct LoweringReplay {
    pub steps: Vec<LoweringStepReplay>,
    pub degraded: bool,
    pub degraded_reason: Option<String>,
}

/// A subscribe request from the frontend. `CompileSource` is defined now,
/// for wire-contract stability, but returns `Unimplemented` until the
/// hosted-playground phase.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClientMessage {
    WatchFile { path: String },
    CompileSource { source: String, name: String },
}

/// Everything the server ever pushes over the WebSocket.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerMessage {
    PipelineReport {
        sequence: u64,
        report: PipelineReport,
    },
    PassReplay {
        sequence: u64,
        replay: PassReplay,
    },
    LoweringReplay {
        sequence: u64,
        replay: LoweringReplay,
    },
    CompileError {
        sequence: u64,
        message: String,
    },
}
