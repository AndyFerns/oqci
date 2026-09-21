//! Simulator backends.
//!
//! Stage C §7 asks that simulator execution use the same backend contract
//! where practical, and names Qiskit Aer, the Cirq simulator and the CUDA-Q
//! simulator as targets. This module implements the contract's compilation
//! half for any of them: lowering, validation and preparation are simulator-
//! agnostic, because the executable representation is a structured operation
//! list rather than any one SDK's format.
//!
//! # Where execution actually happens
//!
//! Not here. The project's non-goals rule out "a custom quantum simulator
//! replacing established simulator frameworks", so writing one in this crate
//! to satisfy [`Backend::execute`] would trade a boundary for a violation.
//! Execution belongs to Aer, through the Python adapter in `python/`, and
//! [`Backend::execute`] says so with a typed error.
//!
//! The test-only state-vector simulator under `tests/support/` is not a
//! counterexample: it exists to verify that rewrites preserve semantics, never
//! leaves `cargo test`, and is a dev-dependency precisely so it cannot become
//! a product feature by accident.

use crate::ir::Circuit;
use crate::lowering::Lowered;
use crate::target::{BasisProfile, CostModel, WeightedCostModel, builtin, cost};

use super::result::{ExecutionSettings, Provenance};
use super::{Backend, BackendError, Executable, ExecutionResult, validated};

/// A backend that compiles for a simulator.
#[derive(Debug, Clone)]
pub struct SimulatorBackend {
    id: String,
    description: String,
    profile: BasisProfile,
    cost_model: WeightedCostModel,
}

impl SimulatorBackend {
    /// A simulator with no connectivity or basis constraints.
    #[must_use]
    pub fn ideal() -> Self {
        SimulatorBackend::from_profile(
            "simulator",
            "unconstrained simulator: all-to-all connectivity, full gate set",
            builtin::ideal_simulator(),
        )
    }

    /// A simulator constrained like a small NISQ device.
    ///
    /// Useful because it exercises routing and decomposition while still
    /// being runnable: the same circuit can be executed on both this and
    /// [`SimulatorBackend::ideal`] and the results compared, which is how the
    /// lowering is checked against something other than its own semantics.
    #[must_use]
    pub fn linear_nisq(qubit_count: u32) -> Self {
        SimulatorBackend::from_profile(
            "simulator-nisq",
            "simulator constrained to a linear NISQ basis and topology",
            builtin::linear_nisq(qubit_count),
        )
    }

    /// A simulator backend over any profile.
    ///
    /// # Panics
    ///
    /// If the profile names a cost model nothing can resolve. Unreachable for
    /// the built-ins, which are covered by a test; a caller supplying its own
    /// profile should use [`SimulatorBackend::try_from_profile`].
    #[must_use]
    pub fn from_profile(id: &str, description: &str, profile: BasisProfile) -> Self {
        SimulatorBackend::try_from_profile(id, description, profile)
            .expect("built-in profiles name resolvable cost models")
    }

    /// A simulator backend over any profile, reporting an unresolvable cost
    /// model rather than panicking.
    ///
    /// # Errors
    ///
    /// [`BackendError::InvalidTarget`] if the profile's cost model is
    /// unknown. `cost::resolve` returns `None` rather than substituting a
    /// default, because a result recording a cost model that did not produce
    /// it is false provenance (Stage E §9).
    pub fn try_from_profile(
        id: &str,
        description: &str,
        profile: BasisProfile,
    ) -> Result<Self, BackendError> {
        let cost_model =
            cost::resolve(profile.cost_model_id()).ok_or_else(|| BackendError::InvalidTarget {
                backend: id.to_string(),
                detail: format!("unknown cost model `{}`", profile.cost_model_id()),
            })?;
        Ok(SimulatorBackend {
            id: id.to_string(),
            description: description.to_string(),
            profile,
            cost_model,
        })
    }
}

impl Backend for SimulatorBackend {
    fn id(&self) -> &str {
        &self.id
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn profile(&self) -> &BasisProfile {
        &self.profile
    }

    fn cost_model(&self) -> &dyn CostModel {
        &self.cost_model
    }

    fn prepare(
        &self,
        lowered: &Lowered,
        settings: &ExecutionSettings,
        provenance: Provenance,
    ) -> Result<Executable, BackendError> {
        validated(self, lowered)?;
        Executable::from_lowered(lowered, &self.id, settings.clone(), provenance)
    }

    fn execute(&self, _executable: &Executable) -> Result<ExecutionResult, BackendError> {
        Err(BackendError::ExecutionNotAvailableInProcess {
            backend: self.id.clone(),
            reason: "simulator execution runs through Qiskit Aer in the Python adapter \
                     (`oqci.backends.aer`); this crate prepares the executable and does not \
                     implement a simulator of its own"
                .to_string(),
        })
    }
}

/// Assembles the provenance record for a compilation.
///
/// Lives here rather than in each backend so that every result carries the
/// same fields whatever produced it — Stage C §9's list is a contract, and a
/// backend that quietly omitted one would make its results incomparable with
/// the others'.
#[must_use]
pub fn provenance_for(
    backend: &dyn Backend,
    circuit: &Circuit,
    lowered: &Lowered,
    pass_pipeline: &[String],
    settings: &ExecutionSettings,
) -> Provenance {
    let (compiler_version, git_commit) = Provenance::compiler_identity();
    let cost_model = backend.cost_model();
    Provenance {
        circuit: circuit.name().to_string(),
        compiler_version,
        git_commit,
        backend_id: backend.id().to_string(),
        profile_id: lowered.profile_id.clone(),
        cost_model_id: cost_model.id().to_string(),
        cost_model_version: cost_model.version().to_string(),
        cost_model_configuration: cost_model.configuration(),
        pass_pipeline: pass_pipeline.to_vec(),
        lowering_steps: lowered.steps.iter().map(|s| s.id.to_string()).collect(),
        decomposition_rules: lowered.rules_applied.clone(),
        initial_layout: lowered.initial_layout.permutation(),
        final_layout: lowered.final_layout.permutation(),
        swaps_inserted: lowered.swaps_inserted,
        shots: settings.shots,
        seed: settings.seed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::CircuitBuilder;
    use crate::lowering::{LoweringConfig, lower};

    fn bell() -> Circuit {
        let mut b = CircuitBuilder::new("bell");
        let q = b.alloc_qubits(2);
        let c = b.alloc_clbits(2);
        b.h(q[0])
            .cx(q[0], q[1])
            .measure(q[0], c[0])
            .measure(q[1], c[1]);
        b.build().unwrap()
    }

    #[test]
    fn the_ideal_simulator_prepares_a_circuit_unchanged() {
        let backend = SimulatorBackend::ideal();
        let circuit = bell();
        let lowered = backend.lower(&circuit, &LoweringConfig::default()).unwrap();
        let settings = ExecutionSettings::default();
        let provenance = provenance_for(&backend, &circuit, &lowered, &[], &settings);
        let executable = backend.prepare(&lowered, &settings, provenance).unwrap();

        assert_eq!(executable.operations(), vec!["cx", "h", "measure"]);
        assert!(executable.has_measurement());
    }

    #[test]
    fn the_nisq_simulator_lowers_into_its_basis() {
        let backend = SimulatorBackend::linear_nisq(3);
        let circuit = bell();
        let lowered = backend.lower(&circuit, &LoweringConfig::default()).unwrap();
        let settings = ExecutionSettings::default();
        let provenance = provenance_for(&backend, &circuit, &lowered, &[], &settings);
        let executable = backend.prepare(&lowered, &settings, provenance).unwrap();

        for op in &executable.ops {
            assert!(
                backend.profile().supports_operation(&op.op),
                "`{}` is outside the basis",
                op.op
            );
        }
    }

    #[test]
    fn provenance_records_every_field_stage_c_requires() {
        let backend = SimulatorBackend::linear_nisq(3);
        let circuit = bell();
        let lowered = backend.lower(&circuit, &LoweringConfig::default()).unwrap();
        let settings = ExecutionSettings {
            shots: 512,
            seed: Some(11),
            memory: false,
        };
        let provenance = provenance_for(
            &backend,
            &circuit,
            &lowered,
            &["canonicalize".to_string()],
            &settings,
        );

        assert_eq!(provenance.circuit, "bell");
        assert_eq!(provenance.compiler_version, env!("CARGO_PKG_VERSION"));
        assert!(!provenance.git_commit.is_empty());
        assert_eq!(provenance.backend_id, "simulator-nisq");
        assert_eq!(provenance.profile_id, "linear-nisq@1");
        assert_eq!(provenance.cost_model_id, "nisq-weighted");
        assert!(!provenance.cost_model_configuration.is_empty());
        assert_eq!(provenance.pass_pipeline, vec!["canonicalize".to_string()]);
        assert!(provenance.lowering_steps.contains(&"routing".to_string()));
        assert!(!provenance.decomposition_rules.is_empty());
        assert_eq!(provenance.shots, 512);
        assert_eq!(provenance.seed, Some(11));
    }

    #[test]
    fn the_cost_model_configuration_travels_with_the_score() {
        // Stage E §6: a scalar with undisclosed weights is not evidence.
        let backend = SimulatorBackend::linear_nisq(3);
        let configuration = backend.cost_model().configuration();
        assert!(configuration.contains_key("two_qubit_weight"));
    }

    #[test]
    fn a_profile_naming_an_unknown_cost_model_is_refused_not_defaulted() {
        let profile = crate::target::BasisProfileBuilder::new(
            "odd",
            "1",
            "b",
            crate::target::Topology::linear(2),
        )
        .operations(["cx"])
        .cost_model("a-model-nobody-implements")
        .build()
        .unwrap();

        let err = SimulatorBackend::try_from_profile("odd", "", profile).unwrap_err();
        assert!(
            matches!(err, BackendError::InvalidTarget { .. }),
            "got {err}"
        );
    }

    #[test]
    fn execution_names_where_it_actually_happens() {
        let backend = SimulatorBackend::ideal();
        let circuit = bell();
        let lowered = backend.lower(&circuit, &LoweringConfig::default()).unwrap();
        let settings = ExecutionSettings::default();
        let provenance = provenance_for(&backend, &circuit, &lowered, &[], &settings);
        let executable = backend.prepare(&lowered, &settings, provenance).unwrap();

        let err = backend.execute(&executable).unwrap_err();
        assert!(
            err.to_string().contains("Aer"),
            "the boundary should say where execution happens: {err}"
        );
    }

    #[test]
    fn preparing_refuses_a_circuit_lowered_for_a_different_device() {
        // Stage C exit criterion 5: target validity is checked before
        // submission, not assumed from the fact that lowering succeeded
        // somewhere.
        let backend = SimulatorBackend::linear_nisq(3);
        let mut b = CircuitBuilder::new("far");
        let q = b.alloc_qubits(3);
        b.cx(q[0], q[2]);
        let circuit = b.build().unwrap();

        let elsewhere = lower(
            &circuit,
            &builtin::ideal_simulator(),
            &LoweringConfig::default(),
        )
        .unwrap();
        let settings = ExecutionSettings::default();
        let provenance = provenance_for(&backend, &circuit, &elsewhere, &[], &settings);

        let err = backend
            .prepare(&elsewhere, &settings, provenance)
            .unwrap_err();
        assert!(
            matches!(err, BackendError::InvalidTarget { .. }),
            "got {err}"
        );
    }
}
