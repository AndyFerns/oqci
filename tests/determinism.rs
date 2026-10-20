//! The determinism contract (`docs/determinism.md`), checked across processes.
//!
//! A compilation's *semantic* outputs — circuits, layouts, cost, executable,
//! QIR — must be byte-identical across repeated runs of the same build on the
//! same input. Its *measurements* (durations) are never expected to be, and
//! `strip_measurements` removes them before comparing.
//!
//! Each run is a separate `oqci` process. That matters: Rust seeds every
//! `HashMap` randomly per process, so an output that leaked hash iteration
//! order would pass an in-process comparison and fail this one.

use std::path::{Path, PathBuf};
use std::process::Command;

use oqci::cli::snapshot::strip_measurements;

/// Runs from the CI matrix use the Stage G repetition count; locally a few
/// runs are enough to catch a hash-order leak with overwhelming probability.
const REPETITIONS: usize = 3;

fn oqci(args: &[&str]) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_oqci"))
        .args(args)
        .output()
        .expect("the oqci binary runs");
    assert!(
        output.status.success(),
        "oqci {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("--json output is JSON");
    strip_measurements(&mut value);
    value
}

fn examples() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir("examples")
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("qasm" | "py")))
        .collect();
    paths.sort();
    paths
}

fn assert_repeatable(args: &[&str]) -> bool {
    let first = oqci(args);
    for _ in 1..REPETITIONS {
        let again = oqci(args);
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&again).unwrap(),
            "oqci {args:?} is not deterministic"
        );
    }
    true
}

#[test]
fn optimization_is_deterministic_across_processes() {
    for path in examples() {
        let path = path.to_str().unwrap();
        assert_repeatable(&[
            "optimize",
            path,
            "--json",
            "--timings",
            "--verify",
            "--lint",
        ]);
    }
}

#[test]
fn lowering_and_preparation_are_deterministic_across_processes() {
    let mut compiled = 0;
    for path in examples() {
        let path = path.to_str().unwrap();
        for backend in ["simulator-nisq", "ibm-illustrative", "qir-base"] {
            let args = [
                "lower",
                path,
                "--backend",
                backend,
                "--json",
                "--timings",
                "--emit",
                "qc-ir,qir",
                "--layout",
                "dense",
            ];
            // Some examples are parameterized or wider than a backend; those
            // refuse deterministically and are not this test's subject.
            let probe = Command::new(env!("CARGO_BIN_EXE_oqci"))
                .args(args)
                .output()
                .unwrap();
            if !probe.status.success() {
                continue;
            }
            assert_repeatable(&args);
            compiled += 1;
        }
    }
    assert!(
        compiled >= 6,
        "only {compiled} example/backend pairs lowered"
    );
}

#[test]
fn a_loaded_target_is_deterministic_across_processes() {
    let fixture = Path::new("tests/fixtures/heron_style_synthetic.json");
    assert_repeatable(&[
        "lower",
        "examples/ghz3.qasm",
        "--target-file",
        fixture.to_str().unwrap(),
        "--json",
        "--verify",
    ]);
}

#[test]
fn stripping_removes_measurements_and_nothing_else() {
    let mut value = serde_json::json!({
        "circuit": [1, 2],
        "timings": {"total_us": 9},
        "passes": [{"id": "x", "duration_us": 4, "changed": true}],
        "nested": {"execution_duration_ms": 1.0, "kept": "yes"}
    });
    strip_measurements(&mut value);
    assert_eq!(
        value,
        serde_json::json!({
            "circuit": [1, 2],
            "passes": [{"id": "x", "changed": true}],
            "nested": {"kept": "yes"}
        })
    );
}
