//! Basis profiles: a formal description of what a backend accepts.
//!
//! Stage D §2 is explicit that a profile is "more than a list of gate names",
//! and `final-deliverables-spec.md` §11 enumerates what it must be able to
//! express. [`BasisProfile`] covers that list:
//!
//! | §11 field | Here |
//! |---|---|
//! | `profile_id`, `profile_version` | [`BasisProfile::id`], [`BasisProfile::version`] |
//! | `backend_id` | [`BasisProfile::backend_id`] |
//! | `physical_qubit_count` | [`BasisProfile::qubit_count`], from the topology |
//! | `topology` | [`BasisProfile::topology`] |
//! | `supported_operations` | [`BasisProfile::supports_operation`] |
//! | `parameter_constraints` | [`BasisProfile::parameter_constraint`] |
//! | `decomposition_rules` | [`BasisProfile::decomposition_rules`] |
//! | `measurement_constraints`, `reset_constraints` | [`MeasurementSupport`] |
//! | `capabilities` | [`BasisProfile::capabilities`] |
//! | `cost_model_reference` | [`BasisProfile::cost_model_id`] |
//!
//! # Operations are named by mnemonic, not by `GateKind`
//!
//! A profile says it supports `"rz"`, not `GateKind::Rz(Param)`. The reason is
//! that "supported" is a statement about a gate *kind*, independent of the
//! angle any particular instance carries — and [`crate::ir::GateKind::mnemonic`]
//! already gives exactly that, so the target layer needs no parallel gate
//! vocabulary of its own. A backend naming an operation OQCI has no registered
//! variant for is expressible too: the mnemonic is just a string, and an
//! `Opaque` gate reports its own name.
//!
//! # Profiles are snapshots
//!
//! Stage D §8 requires that every profile used in an experiment carry a stable
//! identifier and version, and that results record which profile produced them.
//! [`BasisProfile`] is therefore `Serialize` and ordered-collection-backed, so
//! two profiles built by different code paths serialize byte-identically.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::target::topology::{PhysicalQubit, Topology};

/// Why a profile could not be built.
///
/// Malformed profiles are rejected at construction rather than surfacing later
/// as confusing legality failures against a circuit that was actually fine.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum TargetError {
    /// The profile is missing an identifier, version, or backend id.
    #[error("profile is missing a non-empty `{field}`")]
    MissingField {
        /// Which field.
        field: &'static str,
    },
    /// A coupling references a qubit the device does not have.
    #[error("coupling {control} -> {target} references a qubit outside the device's {qubit_count}")]
    EdgeOutOfRange {
        /// Offending control.
        control: PhysicalQubit,
        /// Offending target.
        target: PhysicalQubit,
        /// Declared device size.
        qubit_count: u32,
    },
    /// A parameter constraint was given for an operation the profile does not
    /// support, which is almost always a typo in the mnemonic.
    #[error("parameter constraint names `{mnemonic}`, which is not a supported operation")]
    ConstraintForUnsupportedOperation {
        /// The unsupported mnemonic.
        mnemonic: String,
    },
    /// A constraint's bounds are reversed or non-finite.
    #[error("parameter constraint for `{mnemonic}` has an invalid range [{min}, {max}]")]
    InvalidParameterRange {
        /// The operation.
        mnemonic: String,
        /// Lower bound as given.
        min: f64,
        /// Upper bound as given.
        max: f64,
    },
}

/// An inclusive bound on a gate's parameter values, in radians.
///
/// Stage D §2 lists "parameter domains/constraints" among what a profile must
/// describe: hardware that implements `rz` by frame change may accept any
/// angle, while a pulse-calibrated rotation may not.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ParameterConstraint {
    /// Smallest accepted value, inclusive.
    pub min: f64,
    /// Largest accepted value, inclusive.
    pub max: f64,
}

impl ParameterConstraint {
    /// Builds a constraint over an inclusive range.
    #[must_use]
    pub const fn new(min: f64, max: f64) -> Self {
        ParameterConstraint { min, max }
    }

    /// Whether `value` lies within the bound.
    #[must_use]
    pub fn admits(&self, value: f64) -> bool {
        value >= self.min && value <= self.max
    }

    fn is_well_formed(&self) -> bool {
        self.min.is_finite() && self.max.is_finite() && self.min <= self.max
    }
}

/// What a backend permits in the way of measurement and reset.
///
/// Stage D §2 lists measurement and reset constraints separately from the gate
/// set because they constrain differently: a device may measure only at the end
/// of a circuit, or may not implement reset at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MeasurementSupport {
    /// Whether the device can measure at all.
    pub measurement: bool,
    /// Whether a measurement may be followed by further operations on the
    /// measured qubit. `false` means measurements must be terminal.
    pub mid_circuit_measurement: bool,
    /// Whether the device implements reset.
    pub reset: bool,
}

impl MeasurementSupport {
    /// Everything permitted — the simulator case.
    #[must_use]
    pub const fn unrestricted() -> Self {
        MeasurementSupport {
            measurement: true,
            mid_circuit_measurement: true,
            reset: true,
        }
    }
}

impl Default for MeasurementSupport {
    fn default() -> Self {
        MeasurementSupport::unrestricted()
    }
}

/// A formal description of one backend target.
///
/// Build one with [`BasisProfileBuilder`]; the fields are private so that a
/// profile in hand is always one that passed validation, mirroring how
/// [`crate::ir::Circuit`] relates to [`crate::ir::CircuitBuilder`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BasisProfile {
    id: String,
    version: String,
    backend_id: String,
    topology: Topology,
    supported_operations: BTreeSet<String>,
    parameter_constraints: BTreeMap<String, ParameterConstraint>,
    decomposition_rules: BTreeSet<String>,
    measurement: MeasurementSupport,
    capabilities: BTreeSet<String>,
    cost_model_id: String,
}

impl BasisProfile {
    /// Stable profile identifier, e.g. `"ideal-simulator"`.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Profile version. Together with [`BasisProfile::id`] this is what an
    /// experiment records to make a result reproducible (Stage D §8).
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The backend this profile describes.
    #[must_use]
    pub fn backend_id(&self) -> &str {
        &self.backend_id
    }

    /// The device's connectivity.
    #[must_use]
    pub fn topology(&self) -> &Topology {
        &self.topology
    }

    /// Number of physical qubits the device has.
    #[must_use]
    pub fn qubit_count(&self) -> u32 {
        self.topology.qubit_count()
    }

    /// Whether the backend natively supports an operation, by mnemonic.
    ///
    /// `"measure"` and `"reset"` answer from [`MeasurementSupport`] rather
    /// than the gate set. Those two have their own constraint fields, so
    /// letting the basis list answer for them as well would give the profile
    /// two ways to say the same thing — and therefore a way to contradict
    /// itself, with legality believing one and cost the other.
    #[must_use]
    pub fn supports_operation(&self, mnemonic: &str) -> bool {
        match mnemonic {
            "measure" => self.measurement.measurement,
            "reset" => self.measurement.reset,
            other => self.supported_operations.contains(other),
        }
    }

    /// The native operation set, ascending.
    #[must_use]
    pub fn supported_operations(&self) -> Vec<&str> {
        self.supported_operations
            .iter()
            .map(String::as_str)
            .collect()
    }

    /// The parameter bound for an operation, if the profile declared one.
    #[must_use]
    pub fn parameter_constraint(&self, mnemonic: &str) -> Option<ParameterConstraint> {
        self.parameter_constraints.get(mnemonic).copied()
    }

    /// Identifiers of the decomposition rules this profile expects to be
    /// available when target lowering runs.
    ///
    /// These are **references, not executable rewrites**. Stage D §5 specifies
    /// what a rule must define (source operation, target sequence, parameter
    /// transformation, operand mapping, exact-vs-approximate); that data model
    /// lands with basis decomposition, which is the first thing that will
    /// consume it. Recording the ids now is what lets a profile state its
    /// expectations without inventing a rewrite format nothing yet reads.
    #[must_use]
    pub fn decomposition_rules(&self) -> Vec<&str> {
        self.decomposition_rules
            .iter()
            .map(String::as_str)
            .collect()
    }

    /// Measurement and reset constraints.
    #[must_use]
    pub fn measurement(&self) -> MeasurementSupport {
        self.measurement
    }

    /// Free-form capability flags.
    #[must_use]
    pub fn capabilities(&self) -> Vec<&str> {
        self.capabilities.iter().map(String::as_str).collect()
    }

    /// Identifier of the cost model this target defines optimization cost by
    /// (Stage E: the backend owns cost).
    #[must_use]
    pub fn cost_model_id(&self) -> &str {
        &self.cost_model_id
    }

    /// `"<id>@<version>"` — the form that belongs in result provenance.
    #[must_use]
    pub fn qualified_id(&self) -> String {
        format!("{}@{}", self.id, self.version)
    }
}

/// Builder for [`BasisProfile`].
#[derive(Debug, Clone)]
pub struct BasisProfileBuilder {
    id: String,
    version: String,
    backend_id: String,
    topology: Topology,
    supported_operations: BTreeSet<String>,
    parameter_constraints: BTreeMap<String, ParameterConstraint>,
    decomposition_rules: BTreeSet<String>,
    measurement: MeasurementSupport,
    capabilities: BTreeSet<String>,
    cost_model_id: String,
}

impl BasisProfileBuilder {
    /// Starts a profile for `backend_id` over the given topology.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        backend_id: impl Into<String>,
        topology: Topology,
    ) -> Self {
        BasisProfileBuilder {
            id: id.into(),
            version: version.into(),
            backend_id: backend_id.into(),
            topology,
            supported_operations: BTreeSet::new(),
            parameter_constraints: BTreeMap::new(),
            decomposition_rules: BTreeSet::new(),
            measurement: MeasurementSupport::unrestricted(),
            capabilities: BTreeSet::new(),
            cost_model_id: String::new(),
        }
    }

    /// Declares one natively-supported operation, by mnemonic.
    #[must_use]
    pub fn operation(mut self, mnemonic: impl Into<String>) -> Self {
        self.supported_operations.insert(mnemonic.into());
        self
    }

    /// Declares several natively-supported operations.
    #[must_use]
    pub fn operations<I, S>(mut self, mnemonics: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.supported_operations
            .extend(mnemonics.into_iter().map(Into::into));
        self
    }

    /// Bounds an operation's parameter values.
    #[must_use]
    pub fn parameter_constraint(
        mut self,
        mnemonic: impl Into<String>,
        constraint: ParameterConstraint,
    ) -> Self {
        self.parameter_constraints
            .insert(mnemonic.into(), constraint);
        self
    }

    /// Records a decomposition-rule identifier this target expects.
    #[must_use]
    pub fn decomposition_rule(mut self, rule_id: impl Into<String>) -> Self {
        self.decomposition_rules.insert(rule_id.into());
        self
    }

    /// Sets measurement and reset constraints.
    #[must_use]
    pub fn measurement(mut self, measurement: MeasurementSupport) -> Self {
        self.measurement = measurement;
        self
    }

    /// Adds a capability flag.
    #[must_use]
    pub fn capability(mut self, capability: impl Into<String>) -> Self {
        self.capabilities.insert(capability.into());
        self
    }

    /// Names the cost model this target defines cost by.
    #[must_use]
    pub fn cost_model(mut self, cost_model_id: impl Into<String>) -> Self {
        self.cost_model_id = cost_model_id.into();
        self
    }

    /// Validates and produces the profile.
    ///
    /// # Errors
    ///
    /// [`TargetError`] if an identifier is empty, a coupling references a
    /// non-existent qubit, or a parameter constraint is malformed or names an
    /// unsupported operation.
    pub fn build(self) -> Result<BasisProfile, TargetError> {
        for (field, value) in [
            ("id", &self.id),
            ("version", &self.version),
            ("backend_id", &self.backend_id),
            ("cost_model_id", &self.cost_model_id),
        ] {
            if value.trim().is_empty() {
                return Err(TargetError::MissingField { field });
            }
        }

        if let Some((control, target)) = self.topology.out_of_range_edges().first() {
            return Err(TargetError::EdgeOutOfRange {
                control: *control,
                target: *target,
                qubit_count: self.topology.qubit_count(),
            });
        }

        for (mnemonic, constraint) in &self.parameter_constraints {
            if !self.supported_operations.contains(mnemonic) {
                return Err(TargetError::ConstraintForUnsupportedOperation {
                    mnemonic: mnemonic.clone(),
                });
            }
            if !constraint.is_well_formed() {
                return Err(TargetError::InvalidParameterRange {
                    mnemonic: mnemonic.clone(),
                    min: constraint.min,
                    max: constraint.max,
                });
            }
        }

        Ok(BasisProfile {
            id: self.id,
            version: self.version,
            backend_id: self.backend_id,
            topology: self.topology,
            supported_operations: self.supported_operations,
            parameter_constraints: self.parameter_constraints,
            decomposition_rules: self.decomposition_rules,
            measurement: self.measurement,
            capabilities: self.capabilities,
            cost_model_id: self.cost_model_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builder() -> BasisProfileBuilder {
        BasisProfileBuilder::new("test", "1", "test-backend", Topology::linear(3))
            .operations(["h", "cx", "rz"])
            .cost_model("test-cost")
    }

    #[test]
    fn a_complete_profile_builds() {
        let profile = builder().build().unwrap();
        assert_eq!(profile.id(), "test");
        assert_eq!(profile.qualified_id(), "test@1");
        assert_eq!(profile.qubit_count(), 3);
        assert!(profile.supports_operation("cx"));
        assert!(!profile.supports_operation("ccx"));
    }

    #[test]
    fn supported_operations_are_sorted_for_reproducible_snapshots() {
        let profile = builder().build().unwrap();
        assert_eq!(profile.supported_operations(), vec!["cx", "h", "rz"]);
    }

    #[test]
    fn empty_identifiers_are_rejected() {
        for (field, builder) in [
            (
                "id",
                BasisProfileBuilder::new("", "1", "b", Topology::linear(2)).cost_model("c"),
            ),
            (
                "version",
                BasisProfileBuilder::new("i", "", "b", Topology::linear(2)).cost_model("c"),
            ),
            (
                "backend_id",
                BasisProfileBuilder::new("i", "1", "", Topology::linear(2)).cost_model("c"),
            ),
            (
                "cost_model_id",
                BasisProfileBuilder::new("i", "1", "b", Topology::linear(2)),
            ),
        ] {
            assert_eq!(
                builder.build(),
                Err(TargetError::MissingField { field }),
                "{field} should be required"
            );
        }
    }

    #[test]
    fn a_coupling_outside_the_device_is_rejected() {
        let mut topology = Topology::disconnected(2);
        topology.add_directed(PhysicalQubit(0), PhysicalQubit(9));
        let error = BasisProfileBuilder::new("i", "1", "b", topology)
            .cost_model("c")
            .build()
            .unwrap_err();
        assert!(matches!(error, TargetError::EdgeOutOfRange { .. }));
    }

    #[test]
    fn a_constraint_on_an_unsupported_operation_is_rejected() {
        // Almost always a typo, and silently ignoring it would make the
        // profile quietly weaker than its author believed.
        let error = builder()
            .parameter_constraint("rx", ParameterConstraint::new(0.0, 1.0))
            .build()
            .unwrap_err();
        assert!(matches!(
            error,
            TargetError::ConstraintForUnsupportedOperation { mnemonic } if mnemonic == "rx"
        ));
    }

    #[test]
    fn a_reversed_or_non_finite_range_is_rejected() {
        for constraint in [
            ParameterConstraint::new(1.0, 0.0),
            ParameterConstraint::new(f64::NAN, 1.0),
            ParameterConstraint::new(0.0, f64::INFINITY),
        ] {
            let error = builder()
                .parameter_constraint("rz", constraint)
                .build()
                .unwrap_err();
            assert!(matches!(error, TargetError::InvalidParameterRange { .. }));
        }
    }

    #[test]
    fn parameter_constraints_admit_their_bounds_inclusively() {
        let constraint = ParameterConstraint::new(-1.0, 1.0);
        assert!(constraint.admits(-1.0) && constraint.admits(1.0) && constraint.admits(0.0));
        assert!(!constraint.admits(1.5));
    }

    #[test]
    fn measure_and_reset_support_comes_from_the_measurement_fields() {
        // One question, one answer: legality and cost must not be able to
        // disagree about whether this device can measure.
        let profile = builder()
            .measurement(MeasurementSupport {
                measurement: true,
                mid_circuit_measurement: false,
                reset: false,
            })
            .build()
            .unwrap();
        assert!(profile.supports_operation("measure"));
        assert!(!profile.supports_operation("reset"));
    }

    #[test]
    fn listing_measure_in_the_basis_set_does_not_override_measurement_support() {
        let profile = builder()
            .operation("reset")
            .measurement(MeasurementSupport {
                measurement: true,
                mid_circuit_measurement: true,
                reset: false,
            })
            .build()
            .unwrap();
        assert!(
            !profile.supports_operation("reset"),
            "the constraint field is authoritative"
        );
    }

    #[test]
    fn measurement_support_defaults_to_unrestricted() {
        let profile = builder().build().unwrap();
        let support = profile.measurement();
        assert!(support.measurement && support.mid_circuit_measurement && support.reset);
    }

    #[test]
    fn measurement_constraints_are_recorded() {
        let profile = builder()
            .measurement(MeasurementSupport {
                measurement: true,
                mid_circuit_measurement: false,
                reset: false,
            })
            .build()
            .unwrap();
        assert!(!profile.measurement().mid_circuit_measurement);
        assert!(!profile.measurement().reset);
    }

    #[test]
    fn decomposition_rules_are_recorded_as_identifiers() {
        let profile = builder()
            .decomposition_rule("h-to-rz-sx")
            .decomposition_rule("cz-to-cx")
            .build()
            .unwrap();
        assert_eq!(
            profile.decomposition_rules(),
            vec!["cz-to-cx", "h-to-rz-sx"]
        );
    }

    #[test]
    fn a_profile_serializes_for_snapshotting() {
        let profile = builder().capability("pulse-control").build().unwrap();
        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("\"id\":\"test\""));
        assert!(json.contains("pulse-control"));

        // Stage D §8: the same profile must snapshot identically every time.
        assert_eq!(
            json,
            serde_json::to_string(&builder().capability("pulse-control").build().unwrap()).unwrap()
        );
    }
}
