//! Execution results, and the provenance that makes them citable.
//!
//! Stage C §9 lists what every execution result must be attributable to:
//! input circuit, compiler version, git commit, optimization pipeline,
//! backend identity, target profile, compilation metadata, execution
//! settings, shot count, raw backend result, derived metrics. [`Provenance`]
//! carries all of it, and carries it *with* the result rather than in a
//! separate log that can drift.
//!
//! # Why the durations are separate fields
//!
//! Stage C §8 is blunt — "queue/wait behavior must not be mistaken for
//! compiler execution time" — and §10 repeats it. So
//! [`ExecutionResult`] keeps compilation, submission and execution time as
//! three independent `Option`s, and **none is derived from another**. A
//! backend that cannot measure one leaves it `None` rather than reporting a
//! plausible number, because an invented duration would be exactly the kind
//! of fabricated measurement the benchmark work downstream must not inherit.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Everything needed to reproduce and cite a result.
///
/// Stage E §9 adds cost-model identity and configuration to Stage C §9's
/// list, so both are here: a compiled result must be attributable to the cost
/// model that guided it, or a comparison between two optimization experiments
/// means nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// The source circuit's name.
    pub circuit: String,
    /// OQCI's version, from `CARGO_PKG_VERSION`.
    pub compiler_version: String,
    /// The git commit OQCI was built from, or `"unknown"` outside a checkout.
    ///
    /// Reported as `"unknown"` rather than omitted: a missing field invites a
    /// reader to assume it was never recorded, while an explicit `"unknown"`
    /// says the build had no git metadata.
    pub git_commit: String,
    /// The backend this was prepared for.
    pub backend_id: String,
    /// The target profile, as `id@version` (Stage D §8).
    pub profile_id: String,
    /// The cost model the profile names.
    pub cost_model_id: String,
    /// That cost model's version.
    pub cost_model_version: String,
    /// Its configuration — weights included, so a scalar score is never an
    /// unexplained figure (Stage E §6).
    pub cost_model_configuration: BTreeMap<String, String>,
    /// The optimization passes that ran, in order.
    pub pass_pipeline: Vec<String>,
    /// The lowering steps that ran, in order.
    pub lowering_steps: Vec<String>,
    /// Decomposition rules that fired.
    pub decomposition_rules: Vec<String>,
    /// Where each logical qubit started, as `logical index -> physical index`.
    pub initial_layout: Vec<u32>,
    /// Where each logical qubit ended.
    pub final_layout: Vec<u32>,
    /// SWAPs routing inserted.
    pub swaps_inserted: usize,
    /// Shots requested.
    pub shots: u32,
    /// The simulator seed, when one was fixed.
    ///
    /// Recorded, never *chosen* by this crate: §33.15 puts random seeds and
    /// repetition counts in Stage G, and inventing one here would be
    /// fabricating an experimental parameter.
    pub seed: Option<u64>,
}

impl Provenance {
    /// The compiler identity fields, which are the same for every result this
    /// binary produces.
    #[must_use]
    pub fn compiler_identity() -> (String, String) {
        (
            env!("CARGO_PKG_VERSION").to_string(),
            env!("OQCI_GIT_COMMIT").to_string(),
        )
    }
}

/// How an execution should be performed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionSettings {
    /// How many times to run the circuit.
    pub shots: u32,
    /// A simulator seed, for reproducibility.
    ///
    /// `None` means "let the backend choose", which is the honest default:
    /// picking a seed here would be choosing an experimental parameter that
    /// Stage G owns.
    pub seed: Option<u64>,
    /// Whether to request per-shot results rather than aggregated counts.
    pub memory: bool,
}

impl Default for ExecutionSettings {
    fn default() -> Self {
        ExecutionSettings {
            shots: 1024,
            seed: None,
            memory: false,
        }
    }
}

/// What came back from a backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// Measured bitstrings and how often each occurred.
    ///
    /// A `BTreeMap` so the same result serializes identically every time —
    /// results get snapshotted and compared (Stage D §8).
    pub counts: BTreeMap<String, u64>,
    /// Per-shot outcomes, when requested and supported.
    pub memory: Option<Vec<String>>,
    /// Backend-reported metadata, verbatim.
    ///
    /// Kept raw and unparsed on purpose: Stage C §9 requires the raw backend
    /// result to be preserved, and normalizing it here would lose whatever
    /// this particular backend reported that OQCI does not yet model.
    pub backend_metadata: BTreeMap<String, String>,
    /// How long compilation took.
    pub compilation_duration_ms: Option<f64>,
    /// How long the job waited before running — queue time.
    pub submission_duration_ms: Option<f64>,
    /// How long the circuit actually ran.
    pub execution_duration_ms: Option<f64>,
    /// Everything needed to cite this result.
    pub provenance: Provenance,
}

impl ExecutionResult {
    /// Total shots actually observed, summed from the counts.
    ///
    /// Derived from the data rather than echoed from the request, so a
    /// backend that returned fewer shots than were asked for is visible
    /// instead of assumed away.
    #[must_use]
    pub fn observed_shots(&self) -> u64 {
        self.counts.values().sum()
    }

    /// The most frequent outcome, with its count.
    #[must_use]
    pub fn most_frequent(&self) -> Option<(&str, u64)> {
        self.counts
            .iter()
            // Ties break on the lexicographically smaller bitstring, so this
            // is a function of the data rather than of map iteration order.
            .max_by_key(|(bits, count)| (**count, std::cmp::Reverse(bits.to_string())))
            .map(|(bits, count)| (bits.as_str(), *count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance() -> Provenance {
        let (version, commit) = Provenance::compiler_identity();
        Provenance {
            circuit: "bell".into(),
            compiler_version: version,
            git_commit: commit,
            backend_id: "simulator".into(),
            profile_id: "ideal-simulator@1".into(),
            cost_model_id: "uniform".into(),
            cost_model_version: "1".into(),
            cost_model_configuration: BTreeMap::new(),
            pass_pipeline: vec!["canonicalize".into()],
            lowering_steps: vec!["layout".into()],
            decomposition_rules: vec![],
            initial_layout: vec![0, 1],
            final_layout: vec![0, 1],
            swaps_inserted: 0,
            shots: 100,
            seed: Some(7),
        }
    }

    fn result(counts: &[(&str, u64)]) -> ExecutionResult {
        ExecutionResult {
            counts: counts
                .iter()
                .map(|(bits, n)| ((*bits).to_string(), *n))
                .collect(),
            memory: None,
            backend_metadata: BTreeMap::new(),
            compilation_duration_ms: Some(1.0),
            submission_duration_ms: None,
            execution_duration_ms: Some(3.0),
            provenance: provenance(),
        }
    }

    #[test]
    fn the_compiler_identity_is_baked_in_at_build_time() {
        let (version, commit) = Provenance::compiler_identity();
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
        assert!(
            !commit.is_empty(),
            "a commit of some kind is always recorded"
        );
    }

    #[test]
    fn observed_shots_come_from_the_data_not_the_request() {
        // A backend that returned fewer shots than were asked for must be
        // visible, not papered over with the requested number.
        let result = result(&[("00", 40), ("11", 50)]);
        assert_eq!(result.provenance.shots, 100);
        assert_eq!(result.observed_shots(), 90);
    }

    #[test]
    fn the_most_frequent_outcome_breaks_ties_deterministically() {
        let result = result(&[("00", 50), ("11", 50)]);
        assert_eq!(result.most_frequent(), Some(("00", 50)));
    }

    #[test]
    fn an_empty_result_has_no_most_frequent_outcome() {
        assert_eq!(result(&[]).most_frequent(), None);
    }

    #[test]
    fn durations_are_independent_and_absent_when_unmeasured() {
        // Stage C §8: queue time must not be conflated with compiler time.
        // A backend with no queue reports `None`, not zero — zero would claim
        // a measurement that was never made.
        let result = result(&[("00", 1)]);
        assert_eq!(result.submission_duration_ms, None);
        assert_eq!(result.compilation_duration_ms, Some(1.0));
        assert_eq!(result.execution_duration_ms, Some(3.0));
    }

    #[test]
    fn a_result_round_trips_through_json() {
        // The wire format has to survive a round trip: an execution result is
        // an archival record, and something will eventually read one back.
        //
        // Note that the Python adapter's `AerResult` is deliberately *not*
        // this type — it carries only what an execution knows, with no
        // compilation timings to invent — so nothing converts between the two
        // today. If something ever does, the conversion is the thing to test,
        // not this.
        let original = result(&[("00", 12), ("11", 9)]);
        let json = serde_json::to_string(&original).unwrap();
        let parsed: ExecutionResult = serde_json::from_str(&json).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn counts_serialize_in_a_stable_order() {
        let a = result(&[("11", 1), ("00", 2)]);
        let b = result(&[("00", 2), ("11", 1)]);
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    #[test]
    fn the_default_settings_choose_no_seed() {
        // Choosing one would be picking an experimental parameter that
        // Stage G owns (spec §33.15).
        assert_eq!(ExecutionSettings::default().seed, None);
    }
}
