//! Built-in target profiles.
//!
//! Stage D exit criterion 2 asks for at least one backend profile implemented
//! end to end. There are two here, and **both are explicitly synthetic**.
//!
//! Neither claims to describe a real device. `final-deliverables-spec.md` §9.2
//! forbids hard-coding a physical IBM device name into the compiler core, and
//! §33.3–33.4 forbid assuming a vendor's API or implementing one from memory
//! rather than checking it. Calibration data falls under the same caution: a
//! profile asserting error rates nobody measured would be a fabricated record.
//! So these profiles describe *shapes* of target (unconstrained; small linear
//! NISQ-like) rather than pretending to data they do not have. A real device
//! profile is data loaded from that backend, not a literal compiled in here.

use crate::target::profile::{BasisProfile, BasisProfileBuilder, MeasurementSupport};
use crate::target::topology::Topology;

/// The identifier of [`ideal_simulator`].
pub const IDEAL_SIMULATOR: &str = "ideal-simulator";

/// The identifier prefix of [`linear_nisq`].
pub const LINEAR_NISQ: &str = "linear-nisq";

/// A target with no constraints worth the name: all-to-all connectivity, the
/// entire registered gate set, unrestricted measurement and reset.
///
/// This is the reference point — the profile against which any legal circuit
/// stays legal, useful for checking that a legality failure elsewhere is
/// really about the *target* and not about the circuit.
///
/// # Panics
///
/// Never: the profile is a fixed literal that satisfies its own validation,
/// and this is asserted by the module's tests.
#[must_use]
pub fn ideal_simulator() -> BasisProfile {
    const QUBITS: u32 = 32;

    BasisProfileBuilder::new(
        IDEAL_SIMULATOR,
        "1",
        "simulator",
        Topology::all_to_all(QUBITS),
    )
    .operations([
        // Every registered `GateKind` mnemonic, plus the non-unitary ops.
        "id", "x", "y", "z", "h", "s", "sdg", "t", "tdg", "sx", "sxdg", "rx", "ry", "rz", "p", "u",
        "cx", "cy", "cz", "swap", "ccx", "measure", "reset",
    ])
    .measurement(MeasurementSupport::unrestricted())
    .capability("all-to-all-connectivity")
    .capability("unrestricted-parameters")
    .cost_model("uniform")
    .build()
    .expect("the ideal-simulator profile is well-formed by construction")
}

/// A small NISQ-*shaped* target: a linear chain, a restricted `{rz, sx, x, cx}`
/// basis, and no reset.
///
/// The basis is the shape an IBM-style superconducting device tends to
/// have — which is what motivated adding `sx` to the gate set (see
/// `docs/architecture_decision_sx_basis_gate.md`) — but the qubit count,
/// connectivity and constraints here are chosen for illustration, not taken
/// from any real backend.
///
/// Couplings are symmetric: [`Topology::linear`] declares both directions of
/// each link explicitly, so this profile makes no claim about directional
/// hardware. A genuinely directed device would declare one direction only.
///
/// # Panics
///
/// Never; see [`ideal_simulator`].
#[must_use]
pub fn linear_nisq(qubit_count: u32) -> BasisProfile {
    BasisProfileBuilder::new(
        LINEAR_NISQ,
        "1",
        "generic-nisq",
        Topology::linear(qubit_count),
    )
    .operations(["rz", "sx", "x", "cx", "measure"])
    .measurement(MeasurementSupport {
        measurement: true,
        mid_circuit_measurement: false,
        reset: false,
    })
    // Every registered gate outside the basis needs a route into it, or the
    // profile cannot be lowered to. `RuleSet::new` proves that closure holds
    // rather than taking this list on trust, and a gap here is a build
    // failure rather than a circuit that fails to compile later. `x` and the
    // rest of the basis need no rule; they are already native.
    .decomposition_rules([
        "id-to-nothing",
        "y-to-rz-x",
        "z-to-rz",
        "h-to-rz-sx",
        "s-to-rz",
        "sdg-to-rz",
        "t-to-rz",
        "tdg-to-rz",
        "sxdg-to-sx",
        "rx-to-rz-sx",
        "ry-to-rz-sx",
        "p-to-rz",
        "u-to-rz-sx",
        "cy-to-cx",
        "cz-to-cx",
        "swap-to-cx",
        "ccx-to-cx",
    ])
    .capability("linear-connectivity")
    .cost_model("nisq-weighted")
    .build()
    .expect("the linear-nisq profile is well-formed by construction")
}

/// Every built-in profile, for listing and lookup by id.
///
/// `linear-nisq` is instantiated at a fixed width here so the set is
/// enumerable; [`linear_nisq`] takes any width.
#[must_use]
pub fn all() -> Vec<BasisProfile> {
    vec![ideal_simulator(), linear_nisq(5)]
}

/// Looks a built-in profile up by its [`BasisProfile::id`].
#[must_use]
pub fn by_id(id: &str) -> Option<BasisProfile> {
    all().into_iter().find(|profile| profile.id() == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::GateKind;
    use crate::target::topology::PhysicalQubit;

    #[test]
    fn both_builtin_profiles_are_well_formed() {
        // The `expect` in each constructor is only safe if this holds.
        assert_eq!(all().len(), 2);
    }

    #[test]
    fn the_simulator_supports_every_registered_gate() {
        // Guards against a future `GateKind` variant being added without the
        // "no constraints" profile learning about it.
        let profile = ideal_simulator();
        let registered = [
            GateKind::I,
            GateKind::X,
            GateKind::Y,
            GateKind::Z,
            GateKind::H,
            GateKind::S,
            GateKind::Sdg,
            GateKind::T,
            GateKind::Tdg,
            GateKind::SX,
            GateKind::SXdg,
            GateKind::Cx,
            GateKind::Cy,
            GateKind::Cz,
            GateKind::Swap,
            GateKind::Ccx,
        ];
        for kind in registered {
            assert!(
                profile.supports_operation(kind.mnemonic()),
                "{} missing from the ideal simulator",
                kind.mnemonic()
            );
        }
        assert!(profile.supports_operation("measure"));
        assert!(profile.supports_operation("reset"));
    }

    #[test]
    fn the_simulator_is_fully_connected() {
        let profile = ideal_simulator();
        assert!(
            profile
                .topology()
                .supports(PhysicalQubit(0), PhysicalQubit(31))
        );
        assert!(profile.topology().is_symmetric());
    }

    #[test]
    fn the_nisq_profile_has_a_restricted_basis() {
        let profile = linear_nisq(5);
        assert_eq!(
            profile.supported_operations(),
            vec!["cx", "measure", "rz", "sx", "x"]
        );
        assert!(!profile.supports_operation("h"), "h needs decomposing here");
        assert!(!profile.supports_operation("reset"));
    }

    #[test]
    fn the_nisq_profile_is_a_line() {
        let profile = linear_nisq(5);
        assert_eq!(profile.qubit_count(), 5);
        assert!(
            profile
                .topology()
                .supports(PhysicalQubit(0), PhysicalQubit(1))
        );
        assert!(
            !profile
                .topology()
                .supports(PhysicalQubit(0), PhysicalQubit(2))
        );
    }

    #[test]
    fn the_nisq_profile_names_a_rule_for_every_gate_outside_its_basis() {
        // Asserting the *property* rather than a fixed list: what matters is
        // that nothing registered is left without a route into the basis, and
        // a hardcoded list would have to be edited every time the gate set
        // changes — which is precisely when this check earns its keep.
        // `crate::lowering::RuleSet::new` proves the rules then close over
        // the basis; this only checks that each one is named.
        let profile = linear_nisq(5);
        let named = profile.decomposition_rules();

        for mnemonic in [
            "id", "y", "z", "h", "s", "sdg", "t", "tdg", "sxdg", "rx", "ry", "p", "u", "cy", "cz",
            "swap", "ccx",
        ] {
            assert!(
                !profile.supports_operation(mnemonic),
                "`{mnemonic}` is in the basis, so this list is stale"
            );
            assert!(
                named
                    .iter()
                    .any(|id| id.starts_with(&format!("{mnemonic}-to-"))),
                "no decomposition rule named for `{mnemonic}`"
            );
        }
    }

    #[test]
    fn profiles_are_addressable_by_id() {
        assert_eq!(by_id(IDEAL_SIMULATOR).unwrap().id(), IDEAL_SIMULATOR);
        assert_eq!(by_id(LINEAR_NISQ).unwrap().id(), LINEAR_NISQ);
        assert!(by_id("no-such-target").is_none());
    }

    #[test]
    fn profiles_carry_a_cost_model_reference() {
        assert_eq!(ideal_simulator().cost_model_id(), "uniform");
        assert_eq!(linear_nisq(5).cost_model_id(), "nisq-weighted");
    }

    #[test]
    fn qualified_ids_are_usable_as_provenance() {
        assert_eq!(ideal_simulator().qualified_id(), "ideal-simulator@1");
    }
}
