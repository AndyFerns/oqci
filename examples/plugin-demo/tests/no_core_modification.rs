//! The demonstration DoD item 9 asks for: extensions "that demonstrate the
//! claimed modular architecture", added without modifying the compiler core.
//!
//! The structural half of the proof is that this crate depends on `oqci` as
//! any third party would and can reach only its public API. The behavioural
//! half is below: a program in an extension language, optimized by an
//! extension pass, lowered onto an extension backend — and semantically
//! verified end to end by the compiler's own checker.

use std::process::Command;
use std::sync::Arc;

use oqci::compile::{CompilerConfig, Stop, compile_with_frontend};
use oqci::extension::{ExtensionError, PassPlacement, Registry};
use oqci::verify::{Verdict, VerifyConfig};
use oqci_plugin_demo::{HzhToX, Qtoy, demo_ring, register};

const PROGRAM: &str = "\
# Bell pair, then an H;Z;H the demo pass turns into X, on a qubit the ring
# cannot reach directly from qubit 0.
qubits 4
bits 4
h 0
cx 0 3
h 2
z 2
h 2
measure 0 -> 0
measure 2 -> 2
measure 3 -> 3
";

fn registry() -> Registry {
    let mut registry = Registry::default();
    register(&mut registry).expect("the demo extensions register cleanly");
    registry
}

#[test]
fn an_extension_program_compiles_end_to_end_and_verifies() {
    let config = CompilerConfig {
        backend: Some("demo-ring".into()),
        extensions: registry(),
        stop: Stop::Prepared,
        verify: VerifyConfig {
            enabled: true,
            ..VerifyConfig::default()
        },
        ..CompilerConfig::default()
    };
    let artifacts = compile_with_frontend(PROGRAM, "demo", "qtoy", &config).unwrap();

    // The extension pass ran and did its job.
    let record = artifacts
        .pass_records
        .iter()
        .find(|r| r.id == "hzh-to-x")
        .expect("the extension pass is in the pipeline");
    assert!(record.changed, "{:?}", record.notes);

    // Lowered onto the extension backend, legally, in its CZ basis.
    let lowered = artifacts.lowered.as_ref().unwrap();
    assert!(lowered.legality.is_legal());
    let executable = artifacts.executable.as_ref().unwrap();
    assert_eq!(executable.backend_id, "demo-ring");
    assert!(
        executable
            .ops
            .iter()
            .all(|op| { ["rz", "sx", "x", "cz", "measure"].contains(&op.op.as_str()) })
    );

    // Every transformation — the extension pass included — verified.
    let report = artifacts.verification.as_ref().unwrap();
    assert_eq!(report.not_equivalent, 0, "{report:?}");
    assert!(
        report
            .checks
            .iter()
            .any(|c| c.transformation.starts_with("pass:hzh-to-x#")
                && matches!(c.verdict, Verdict::Equivalent { .. }))
    );

    // Provenance names exactly the extensions that took part.
    let provenance = artifacts.provenance.as_ref().unwrap();
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(
        provenance.extensions,
        vec![
            format!("oqci-plugin-demo@{version} backend:demo-ring"),
            format!("oqci-plugin-demo@{version} frontend:qtoy"),
            format!("oqci-plugin-demo@{version} pass:hzh-to-x"),
        ]
    );
    assert!(provenance.pass_pipeline.contains(&"hzh-to-x".to_string()));
}

#[test]
fn compilation_with_extensions_is_deterministic() {
    let config = CompilerConfig {
        backend: Some("demo-ring".into()),
        extensions: registry(),
        ..CompilerConfig::default()
    };
    let a = compile_with_frontend(PROGRAM, "demo", "qtoy", &config).unwrap();
    let b = compile_with_frontend(PROGRAM, "demo", "qtoy", &config).unwrap();
    assert_eq!(a.executable, b.executable);
    assert_eq!(a.qir, b.qir);
}

#[test]
fn the_registry_refuses_what_would_be_ambiguous() {
    let mut registry = registry();
    let origin = oqci_plugin_demo::origin();
    assert!(matches!(
        registry.register_backend(origin.clone(), Arc::new(demo_ring())),
        Err(ExtensionError::DuplicateId {
            kind: "backend",
            ..
        })
    ));
    assert!(matches!(
        registry.register_frontend(origin.clone(), Arc::new(Qtoy)),
        Err(ExtensionError::DuplicateId {
            kind: "frontend",
            ..
        })
    ));
    assert!(matches!(
        registry.register_pass(origin, PassPlacement::End, Arc::new(HzhToX)),
        Err(ExtensionError::DuplicateId { kind: "pass", .. })
    ));
}

#[test]
fn the_frontend_reports_errors_with_positions() {
    let config = CompilerConfig {
        extensions: registry(),
        ..CompilerConfig::default()
    };
    let err = compile_with_frontend("qubits 1\nh 5\n", "bad", "qtoy", &config).unwrap_err();
    assert!(err.to_string().contains("out of range"), "{err}");
    let err = compile_with_frontend("qubits 1\niswap 0\n", "bad", "qtoy", &config).unwrap_err();
    assert!(
        err.to_string().contains("not in the registered gate set"),
        "{err}"
    );
}

#[test]
fn the_demo_binary_is_the_oqci_cli_plus_the_extensions() {
    let dir = std::env::temp_dir().join(format!("oqci-demo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("bell.qtoy");
    std::fs::write(&file, PROGRAM).unwrap();

    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_oqci-demo"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    assert!(run(&["passes"]).contains("hzh-to-x"));
    assert!(run(&["backends"]).contains("demo-ring"));
    // The `.qtoy` extension selects the extension frontend by itself.
    let text = run(&[
        "lower",
        file.to_str().unwrap(),
        "--backend",
        "demo-ring",
        "--verify",
    ]);
    assert!(text.contains("via qtoy"), "{text}");
    assert!(text.contains("0 not equivalent"), "{text}");
    assert!(text.contains("extension: oqci-plugin-demo"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}
