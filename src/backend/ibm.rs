//! The IBM target: lowering, validation and execution preparation.
//!
//! §9 requires IBM to have a distinct backend module rather than being a
//! special case inside generic code, and Stage C §4 gives it a dedicated
//! target-lowering stage. This is that module.
//!
//! # What this does, and where the rest happens
//!
//! Implemented, in pure Rust and fully tested: backend configuration, target
//! profile construction from supplied data, connectivity, target legality
//! checking, target lowering, and execution preparation with structured
//! result metadata.
//!
//! **Execution submission and result retrieval happen outside this crate**,
//! in the Python execution adapter `oqci.backends.ibm`, like every other
//! runtime (`docs/adapters.md`). [`Backend::execute`] therefore returns
//! [`BackendError::ExecutionNotAvailableInProcess`] and names that adapter.
//! The adapter:
//!
//! - builds the profile from the device's live `Target`
//!   (`target_from_backend`), which [`crate::backend::from_target_json`]
//!   loads through [`IbmBackend::from_profile`], so lowering targets the real
//!   coupling map and basis;
//! - submits the prepared executable *verbatim* through `qiskit-ibm-runtime`,
//!   after checking every instruction against the device's instruction set
//!   and refusing on any mismatch — it never transpiles;
//! - retrieves counts, queue time and execution time separately, and the
//!   result converts back into [`ExecutionResult`] via
//!   [`ExecutionResult::from_adapter`].
//!
//! Only CZ-native devices (IBM's Heron family) are supported: their basis is
//! `{rz, sx, x, cz}`, reached through the `cx-to-cz` rule. ECR is not in the
//! gate set.
//!
//! **What is claimed is narrow and checkable:** an [`Executable`] produced
//! here has been lowered to, and validated against, the target description it
//! was given. If that description is wrong, so is the conclusion — which is
//! why the real description is retrieved from the device rather than written
//! down, and why [`IbmBackend::illustrative`] says in its own name that it is
//! not real.
//!
//! # Target data comes from outside
//!
//! §9.2: "The selected IBM backend must be determined by
//! configuration/runtime availability. Do not hard-code one physical IBM
//! device name into the compiler core." So the real constructor is
//! [`IbmBackend::from_target_json`], which takes a serialized
//! [`BasisProfile`] — the shape a retrieved target would be converted into —
//! and the built-in profile exists only so the path can be exercised without
//! a network.

use crate::lowering::Lowered;
use crate::target::{
    BasisProfile, BasisProfileBuilder, CostModel, MeasurementSupport, Topology, WeightedCostModel,
    cost,
};

use super::result::{ExecutionSettings, Provenance};
use super::{Backend, BackendError, Executable, ExecutionResult, validated};

/// A backend that compiles for IBM-style superconducting hardware.
#[derive(Debug, Clone)]
pub struct IbmBackend {
    id: String,
    description: String,
    profile: BasisProfile,
    cost_model: WeightedCostModel,
}

impl IbmBackend {
    /// Builds a backend from a serialized target description.
    ///
    /// This is the constructor that matters. Stage C §8 lists "target
    /// retrieval; target profile construction" among the execution adapter's
    /// responsibilities; retrieval needs the SDK, but *construction* from
    /// retrieved data does not, and is implemented here so the boundary falls
    /// in exactly one place.
    ///
    /// # Errors
    ///
    /// [`BackendError::InvalidTarget`] if the JSON is not a well-formed
    /// profile, or names a cost model nothing can resolve.
    pub fn from_target_json(id: &str, description: &str, json: &str) -> Result<Self, BackendError> {
        let profile: BasisProfile =
            serde_json::from_str(json).map_err(|error| BackendError::InvalidTarget {
                backend: id.to_string(),
                detail: format!("could not read target description: {error}"),
            })?;
        IbmBackend::from_profile(id, description, profile)
    }

    /// Builds a backend from an already-constructed profile.
    ///
    /// # Errors
    ///
    /// [`BackendError::InvalidTarget`] if the profile names an unresolvable
    /// cost model, or if its decomposition rules cannot reach its own basis —
    /// checked here rather than on the first circuit, so a malformed target
    /// is reported against the target.
    pub fn from_profile(
        id: &str,
        description: &str,
        profile: BasisProfile,
    ) -> Result<Self, BackendError> {
        let cost_model =
            cost::resolve(profile.cost_model_id()).ok_or_else(|| BackendError::InvalidTarget {
                backend: id.to_string(),
                detail: format!("unknown cost model `{}`", profile.cost_model_id()),
            })?;
        crate::lowering::RuleSet::new(&profile).map_err(|source| BackendError::InvalidTarget {
            backend: id.to_string(),
            detail: format!("decomposition rules do not reach the basis: {source}"),
        })?;
        Ok(IbmBackend {
            id: id.to_string(),
            description: description.to_string(),
            profile,
            cost_model,
        })
    }

    /// A synthetic IBM-shaped target, for exercising the path without a
    /// network.
    ///
    /// **This does not describe any real device.** The basis is the shape
    /// IBM superconducting hardware tends to have — which is what motivated
    /// adding `sx` to the gate set — and the topology is a five-qubit ring
    /// with two one-way couplings, chosen to exercise directed-edge handling.
    /// The qubit count, the connectivity and the directions are illustrative.
    /// Real values belong in a retrieved target description, not in this
    /// source file (§9.2), and inventing calibration data here would be
    /// exactly the fabricated experimental data §33.15 rules out.
    ///
    /// # Panics
    ///
    /// Never; the profile is well-formed by construction and covered by a
    /// test.
    #[must_use]
    pub fn illustrative() -> Self {
        // A ring, so routing has a choice of direction, with two links
        // declared one-way so orientation repair is genuinely exercised.
        let mut topology = Topology::disconnected(5);
        topology.add_undirected(
            crate::target::PhysicalQubit(0),
            crate::target::PhysicalQubit(1),
        );
        topology.add_undirected(
            crate::target::PhysicalQubit(1),
            crate::target::PhysicalQubit(2),
        );
        topology.add_directed(
            crate::target::PhysicalQubit(2),
            crate::target::PhysicalQubit(3),
        );
        topology.add_undirected(
            crate::target::PhysicalQubit(3),
            crate::target::PhysicalQubit(4),
        );
        topology.add_directed(
            crate::target::PhysicalQubit(4),
            crate::target::PhysicalQubit(0),
        );

        let profile = BasisProfileBuilder::new("ibm-illustrative", "1", "ibm", topology)
            .operations(["rz", "sx", "x", "cx", "measure"])
            .measurement(MeasurementSupport {
                measurement: true,
                mid_circuit_measurement: false,
                reset: false,
            })
            .decomposition_rules(crate::target::builtin::CX_BASIS_RULES.iter().copied())
            .capability("directed-coupling")
            .capability("synthetic-not-a-real-device")
            .cost_model("nisq-weighted")
            .build()
            .expect("the illustrative IBM profile is well-formed by construction");

        IbmBackend::from_profile(
            "ibm-illustrative",
            "synthetic IBM-shaped target; describes no real device and cannot execute",
            profile,
        )
        .expect("the illustrative IBM profile resolves its own cost model")
    }
}

impl Backend for IbmBackend {
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
        // Stage C exit criterion 5: target validity is checked before
        // submission. The artifact does not exist until it has passed.
        validated(self, lowered)?;
        Executable::from_lowered(lowered, &self.id, settings.clone(), provenance)
    }

    fn execute(&self, _executable: &Executable) -> Result<ExecutionResult, BackendError> {
        Err(BackendError::ExecutionNotAvailableInProcess {
            backend: self.id.clone(),
            reason: "IBM submission runs in the Python adapter `oqci.backends.ibm` \
                     (qiskit-ibm-runtime, with the caller's own saved account). The executable \
                     is prepared and validated against the supplied target description; no \
                     claim is made that it will run on any device other than the one that \
                     description came from"
                .to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::CircuitBuilder;
    use crate::lowering::LoweringConfig;
    use crate::target::{CouplingMode, PhysicalQubit};

    fn ghz() -> crate::ir::Circuit {
        let mut b = CircuitBuilder::new("ghz");
        let q = b.alloc_qubits(3);
        let c = b.alloc_clbits(3);
        b.h(q[0])
            .cx(q[0], q[1])
            .cx(q[1], q[2])
            .measure(q[0], c[0])
            .measure(q[1], c[1])
            .measure(q[2], c[2]);
        b.build().unwrap()
    }

    #[test]
    fn the_illustrative_target_is_well_formed_and_lowerable() {
        let backend = IbmBackend::illustrative();
        assert!(crate::lowering::RuleSet::new(backend.profile()).is_ok());
        assert!(
            !backend.profile().topology().is_symmetric(),
            "it has one-way links"
        );
        assert!(
            backend
                .profile()
                .topology()
                .is_connected(CouplingMode::Undirected)
        );
    }

    #[test]
    fn the_illustrative_target_says_in_its_own_metadata_that_it_is_synthetic() {
        // §33 forbids presenting invented device data as real. The profile
        // carries the disclaimer, so it survives serialization into any
        // result that cites it.
        let backend = IbmBackend::illustrative();
        assert!(
            backend
                .profile()
                .capabilities()
                .contains(&"synthetic-not-a-real-device")
        );
        assert!(backend.description().contains("synthetic"));
    }

    #[test]
    fn a_circuit_lowers_and_prepares_for_the_illustrative_target() {
        let backend = IbmBackend::illustrative();
        let circuit = ghz();
        let lowered = backend.lower(&circuit, &LoweringConfig::default()).unwrap();
        assert!(backend.validate(&lowered).is_legal());

        let settings = ExecutionSettings::default();
        let provenance =
            super::super::simulator::provenance_for(&backend, &circuit, &lowered, &[], &settings);
        let executable = backend.prepare(&lowered, &settings, provenance).unwrap();

        for op in &executable.ops {
            assert!(
                backend.profile().supports_operation(&op.op),
                "`{}` is outside the IBM basis",
                op.op
            );
        }
        assert_eq!(executable.backend_id, "ibm-illustrative");
    }

    #[test]
    fn a_one_way_link_forces_orientation_repair() {
        // The reason the illustrative topology has directed edges at all: a
        // symmetric one would leave this path untested.
        let backend = IbmBackend::illustrative();
        let mut b = CircuitBuilder::new("reverse");
        let q = b.alloc_qubits(4);
        // Physical 2->3 is declared one way; this asks for the other.
        b.cx(q[3], q[2]);

        let lowered = backend
            .lower(&b.build().unwrap(), &LoweringConfig::default())
            .unwrap();
        assert!(lowered.orientations_repaired > 0 || lowered.swaps_inserted > 0);
        assert!(lowered.legality.is_legal());
    }

    #[test]
    fn a_target_can_be_built_from_a_serialized_description() {
        // §9.2: the device is supplied, not hard-coded. This is the path a
        // retrieved IBM target would take.
        let json = serde_json::to_string(&IbmBackend::illustrative().profile).unwrap();
        let backend = IbmBackend::from_target_json("ibm-retrieved", "from JSON", &json).unwrap();

        assert_eq!(backend.profile().qualified_id(), "ibm-illustrative@1");
        assert!(
            backend
                .profile()
                .topology()
                .supports(PhysicalQubit(2), PhysicalQubit(3))
        );
        assert!(
            !backend
                .profile()
                .topology()
                .supports(PhysicalQubit(3), PhysicalQubit(2)),
            "the one-way link must survive the round trip"
        );
    }

    #[test]
    fn malformed_target_data_is_refused_rather_than_partially_applied() {
        let err = IbmBackend::from_target_json("ibm", "", "{ not json").unwrap_err();
        assert!(
            matches!(err, BackendError::InvalidTarget { .. }),
            "got {err}"
        );
    }

    #[test]
    fn a_target_whose_rules_cannot_reach_its_basis_is_refused_at_construction() {
        // Better here than on the first circuit: the fault is in the target,
        // so the diagnostic should name the target.
        let profile = BasisProfileBuilder::new("incomplete", "1", "ibm", Topology::linear(3))
            .operations(["rz", "sx", "cx"])
            .decomposition_rules(["ccx-to-cx"])
            .cost_model("nisq-weighted")
            .build()
            .unwrap();

        let err = IbmBackend::from_profile("ibm", "", profile).unwrap_err();
        assert!(
            err.to_string().contains("do not reach the basis"),
            "got {err}"
        );
    }

    #[test]
    fn execution_is_an_explicit_boundary_not_a_silent_success() {
        let backend = IbmBackend::illustrative();
        let circuit = ghz();
        let lowered = backend.lower(&circuit, &LoweringConfig::default()).unwrap();
        let settings = ExecutionSettings::default();
        let provenance =
            super::super::simulator::provenance_for(&backend, &circuit, &lowered, &[], &settings);
        let executable = backend.prepare(&lowered, &settings, provenance).unwrap();

        let err = backend.execute(&executable).unwrap_err();
        assert!(
            matches!(err, BackendError::ExecutionNotAvailableInProcess { .. }),
            "got {err}"
        );
        assert!(
            err.to_string().contains("no claim is made"),
            "the boundary must not overclaim: {err}"
        );
    }
}
