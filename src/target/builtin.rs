//! Built-in target profiles.
//!
//! Stage D exit criterion 2 asks for at least one backend profile implemented
//! end to end. There are three here, and **all are explicitly synthetic**:
//! two describe shapes of device, and [`qir_base`] describes an output format
//! (the QIR Base Profile), not a device at all.
//!
//! None claims to describe a real device. `final-deliverables-spec.md` §9.2
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

/// The rules that lower every registered gate into `{rz, sx, x, cx}`.
///
/// The rule set of [`linear_nisq`] and of the illustrative IBM target, named
/// so that a profile built from retrieved device data can reuse it rather
/// than restate it. Every registered gate outside the basis needs a route
/// into it, or the profile cannot be lowered to; `RuleSet::new` proves the
/// closure rather than taking this list on trust.
pub const CX_BASIS_RULES: &[&str] = &[
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
];

/// The rules that lower every registered gate into `{rz, sx, x, cz}`.
///
/// [`CX_BASIS_RULES`] with `cz-to-cx` replaced by `cx-to-cz`: the two-qubit
/// gates that reduce to `cx` (`cy`, `swap`, `ccx`) reach `cz` through it. The
/// two rules are never listed together, since together they form a cycle
/// that `RuleSet::new` rejects. This is the basis of CZ-native superconducting
/// devices; which device a profile describes is data, not this list.
pub const CZ_BASIS_RULES: &[&str] = &[
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
    "cx-to-cz",
    "cy-to-cx",
    "swap-to-cx",
    "ccx-to-cx",
];

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
    .decomposition_rules(CX_BASIS_RULES.iter().copied())
    .capability("linear-connectivity")
    .cost_model("nisq-weighted")
    .build()
    .expect("the linear-nisq profile is well-formed by construction")
}

/// The identifier of [`qir_base`].
pub const QIR_BASE: &str = "qir-base";

/// The width at which [`all`] instantiates [`qir_base`], and at which the
/// `qir-base` backend compiles.
pub const QIR_BASE_WIDTH: u32 = 32;

/// A target that accepts exactly what a QIR 1.0 Base Profile module may
/// contain, as OQCI understands it.
///
/// It is not a device. It is the target a circuit is lowered onto so that its
/// QIR can honestly be labeled `"qir_profiles"="base_profile"`, and the
/// profile a circuit is *checked* against to decide that label
/// (`docs/qir_lowering.md`).
///
/// **Where the basis comes from.** QIR 1.0 defines no normative quantum
/// instruction set: `specification/Instruction_Set.md` in `qir-alliance/qir-spec`
/// at tag `1.0` (commit `e872897`) says each backend declares its own. The
/// basis is therefore pinned to the instructions PyQIR 0.12.5 exposes
/// (`pyqir/qis.py`: `ccx, cx, cz, h, mz, reset, rx, ry, rz, s, s_adj, swap, t,
/// t_adj, x, y, z`, plus `barrier` and `if_result`), minus three:
///
/// - `reset`, because the Base Profile requires unitaries before measurements
///   and no use of a qubit after one, which leaves no sound place for a reset;
/// - `barrier`, which has no QC-IR counterpart;
/// - `if_result`, which is classical control flow (Stage F §5).
///
/// Measurement is terminal only, for the same Base Profile rule. Everything
/// else registered in QC-IR has a rule into the basis, so any supported
/// circuit can be lowered here.
///
/// # Panics
///
/// Never; see [`ideal_simulator`].
#[must_use]
pub fn qir_base(qubit_count: u32) -> BasisProfile {
    BasisProfileBuilder::new(
        QIR_BASE,
        "1",
        "qir-base-profile",
        Topology::all_to_all(qubit_count),
    )
    .operations([
        "x", "y", "z", "h", "s", "sdg", "t", "tdg", "rx", "ry", "rz", "cx", "cz", "swap", "ccx",
        "measure",
    ])
    .measurement(MeasurementSupport {
        measurement: true,
        mid_circuit_measurement: false,
        reset: false,
    })
    .decomposition_rules([
        "id-to-nothing",
        "sx-to-rx",
        "sxdg-to-rx",
        "p-to-rz",
        "u-to-rz-ry",
        "cy-to-cx",
    ])
    .capability("all-to-all-connectivity")
    .capability("unrestricted-parameters")
    .cost_model("uniform")
    .build()
    .expect("the qir-base profile is well-formed by construction")
}

/// Every built-in profile, for listing and lookup by id.
///
/// `linear-nisq` and `qir-base` are instantiated at fixed widths here so the
/// set is enumerable; [`linear_nisq`] and [`qir_base`] take any width.
#[must_use]
pub fn all() -> Vec<BasisProfile> {
    vec![ideal_simulator(), linear_nisq(5), qir_base(QIR_BASE_WIDTH)]
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
    fn every_builtin_profile_is_well_formed() {
        // The `expect` in each constructor is only safe if this holds.
        assert_eq!(all().len(), 3);
    }

    #[test]
    fn the_qir_base_profile_accepts_exactly_its_pinned_basis() {
        let profile = qir_base(4);
        assert_eq!(
            profile.supported_operations(),
            vec![
                "ccx", "cx", "cz", "h", "measure", "rx", "ry", "rz", "s", "sdg", "swap", "t",
                "tdg", "x", "y", "z"
            ]
        );
        for excluded in ["sx", "sxdg", "id", "p", "u", "cy", "reset"] {
            assert!(
                !profile.supports_operation(excluded),
                "`{excluded}` must not be in qir-base"
            );
        }
        assert!(profile.topology().is_symmetric());
        assert!(
            profile
                .topology()
                .supports(PhysicalQubit(0), PhysicalQubit(3))
        );
    }

    #[test]
    fn the_qir_base_profile_names_a_rule_for_every_gate_outside_its_basis() {
        // Same property as for linear-nisq: every registered gate can be
        // lowered onto qir-base. `RuleSet::new` proves closure; this checks
        // each gap is named.
        let profile = qir_base(4);
        let named = profile.decomposition_rules();
        for mnemonic in ["id", "sx", "sxdg", "p", "u", "cy"] {
            assert!(
                named
                    .iter()
                    .any(|id| id.starts_with(&format!("{mnemonic}-to-"))),
                "no decomposition rule named for `{mnemonic}`"
            );
        }
        assert!(crate::lowering::RuleSet::new(&profile).is_ok());
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
        assert_eq!(by_id(QIR_BASE).unwrap().id(), QIR_BASE);
        assert!(by_id("no-such-target").is_none());
    }

    #[test]
    fn profiles_carry_a_cost_model_reference() {
        assert_eq!(ideal_simulator().cost_model_id(), "uniform");
        assert_eq!(linear_nisq(5).cost_model_id(), "nisq-weighted");
        assert_eq!(qir_base(4).cost_model_id(), "uniform");
    }

    #[test]
    fn qualified_ids_are_usable_as_provenance() {
        assert_eq!(ideal_simulator().qualified_id(), "ideal-simulator@1");
    }
}
