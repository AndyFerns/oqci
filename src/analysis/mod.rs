//! Circuit analysis: resource metrics and structural diffing.
//!
//! This module is the project's **single source of truth for measurement**.
//! Every number reported anywhere — the pass manager's per-pass before/after
//! bookkeeping, the CLI's `analyze` output, the diff rendered by
//! `optimize --diff` — is produced here. Nothing recomputes a gate count or a
//! depth on its own, which is what keeps the compiler's own view and the
//! user's view of the same circuit from ever disagreeing.
//!
//! - [`analyze`] → [`ResourceReport`]: gate counts, depth, register widths
//!   (`final-deliverables-spec.md` §13.1–13.3).
//! - [`diff_circuits`] → [`CircuitDiff`]: what actually changed between two
//!   circuits, computed by comparing them rather than by trusting a pass's
//!   own account of itself.
//!
//! Target-native gate counts and routing overhead are also named in §13 but
//! require a target profile that does not exist yet; they are absent rather
//! than reported as zero.

pub mod diff;
pub mod report;

pub use diff::{CircuitDiff, DiffEntry, diff_circuits};
pub use report::{ResourceReport, analyze};
