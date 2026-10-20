//! Compiling for a target supplied at runtime.
//!
//! `final-deliverables-spec.md` §9.2: "the selected IBM backend must be
//! determined by configuration/runtime availability" rather than hard-coded.
//! A device's description is retrieved outside the crate, serialized as a
//! basis profile, and loaded here. The fixture is a synthetic CZ-native ring
//! — the basis of CZ-native superconducting hardware — that says in its own
//! capabilities that it describes no real device.

use std::sync::Arc;

use oqci::backend::{self, BackendError};
use oqci::compile::{CompileError, CompilerConfig, Stop, compile};
use oqci::extension::{ExtensionError, ExtensionOrigin, Registry};

const HERON_STYLE: &str = include_str!("fixtures/heron_style_synthetic.json");

const BELL: &str = "qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;";
const FAR_CX: &str = "qubit[4] q; bit[4] c; h q[0]; cx q[0], q[3]; c = measure q;";

fn origin() -> ExtensionOrigin {
    ExtensionOrigin::target_file("tests/fixtures/heron_style_synthetic.json")
}

#[test]
fn a_target_file_loads_as_the_kind_its_profile_names() {
    let backend = backend::from_target_json(HERON_STYLE, None, None).unwrap();
    assert_eq!(backend.id(), "heron-style-synthetic");
    assert_eq!(backend.profile().qualified_id(), "heron-style-synthetic@1");
    assert!(
        backend
            .profile()
            .capabilities()
            .contains(&"synthetic-not-a-real-device")
    );
    // Loaded through the IBM loader (`backend_id: "ibm"`), whose boundary
    // message is preserved.
    assert!(backend.description().contains("ibm target"));
}

#[test]
fn kind_and_id_can_be_overridden() {
    let backend =
        backend::from_target_json(HERON_STYLE, Some("simulator"), Some("my-device")).unwrap();
    assert_eq!(backend.id(), "my-device");
    assert!(backend.description().starts_with("simulator target"));
}

#[test]
fn an_unknown_kind_is_refused_with_the_alternatives() {
    let err = backend::from_target_json(HERON_STYLE, Some("quantum-toaster"), None)
        .err()
        .unwrap();
    assert!(matches!(err, BackendError::InvalidTarget { .. }), "{err}");
    assert!(err.to_string().contains("ibm, simulator"), "{err}");
}

#[test]
fn malformed_json_is_refused() {
    let err = backend::from_target_json("{ not json", None, None)
        .err()
        .unwrap();
    assert!(
        err.to_string()
            .contains("could not read target description")
    );
}

#[test]
fn a_target_whose_rules_do_not_close_is_refused_on_load() {
    // Remove `cx-to-cz`: `cy`, `swap` and `ccx` reduce to `cx`, which then has
    // nowhere to go. Refused against the target, before any circuit.
    let broken = HERON_STYLE.replace("\"cx-to-cz\",", "");
    for kind in ["ibm", "simulator"] {
        let err = backend::from_target_json(&broken, Some(kind), None)
            .err()
            .unwrap();
        assert!(err.to_string().contains("do not reach the basis"), "{err}");
    }
}

#[test]
fn a_loaded_target_compiles_through_the_whole_pipeline() {
    let mut registry = Registry::default();
    let id = registry
        .register_target_json(origin(), HERON_STYLE, None, None)
        .unwrap();
    let config = CompilerConfig {
        backend: Some(id),
        extensions: registry,
        verify: oqci::verify::VerifyConfig {
            enabled: true,
            ..oqci::verify::VerifyConfig::default()
        },
        ..CompilerConfig::default()
    };

    for source in [BELL, FAR_CX] {
        let artifacts = compile(source, &config).unwrap();
        let lowered = artifacts.lowered.as_ref().unwrap();
        assert!(lowered.legality.is_legal());
        let executable = artifacts.executable.as_ref().unwrap();
        for op in &executable.ops {
            assert!(
                ["cz", "rz", "sx", "x", "measure", "reset"].contains(&op.op.as_str()),
                "`{}` is outside the CZ basis",
                op.op
            );
        }
        // Verified, not just legal.
        let report = artifacts.verification.as_ref().unwrap();
        assert_eq!(report.not_equivalent, 0);
        assert_eq!(report.inconclusive, 0, "{report:?}");

        // The provenance says where the target came from.
        let provenance = artifacts.provenance.as_ref().unwrap();
        assert_eq!(provenance.profile_id, "heron-style-synthetic@1");
        assert_eq!(
            provenance.extensions,
            vec![
                "target-file:tests/fixtures/heron_style_synthetic.json backend:heron-style-synthetic"
                    .to_string()
            ]
        );
    }
}

#[test]
fn a_loaded_target_cannot_shadow_a_built_in_backend() {
    let mut registry = Registry::default();
    let err = registry
        .register_target_json(origin(), HERON_STYLE, None, Some("simulator"))
        .unwrap_err();
    assert!(
        matches!(
            err,
            ExtensionError::DuplicateId {
                kind: "backend",
                ..
            }
        ),
        "{err}"
    );
}

#[test]
fn an_unregistered_backend_id_is_still_unknown() {
    let config = CompilerConfig {
        backend: Some("heron-style-synthetic".into()),
        stop: Stop::Lowered,
        ..CompilerConfig::default()
    };
    let err = compile(BELL, &config).unwrap_err();
    assert!(matches!(err, CompileError::UnknownBackend { .. }), "{err}");
}

#[test]
fn a_backend_object_registers_like_a_target_file() {
    let mut registry = Registry::default();
    let backend: Arc<dyn oqci::backend::Backend> =
        Arc::from(backend::from_target_json(HERON_STYLE, None, Some("as-object")).unwrap());
    registry
        .register_backend(ExtensionOrigin::new("test", "0"), backend)
        .unwrap();
    assert!(registry.backend("as-object").is_some());
    assert!(registry.backend_ids().contains(&"as-object".to_string()));
}
