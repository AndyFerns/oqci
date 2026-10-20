//! Untrusted widths are refused before allocation (§29), at every boundary.
//!
//! Found by the fuzz corpus: `qubit[4294967295]` made the OpenQASM frontend
//! try to allocate four billion qubits, and a target profile declaring as
//! many aborted the process on allocation failure.

use oqci::frontend::{FrontendError, QiskitCircuitIr, parse_cudaq, parse_openqasm3, qiskit};
use oqci::ir::MAX_REGISTER_WIDTH;
use oqci::target::{BasisProfileBuilder, Topology};

#[test]
fn a_huge_openqasm_register_is_refused() {
    let err = parse_openqasm3("OPENQASM 3.0; qubit[4294967295] q;").unwrap_err();
    assert!(matches!(err, FrontendError::Unsupported(_)), "{err}");
    let err = parse_openqasm3("bit[4294967295] c;").unwrap_err();
    assert!(matches!(err, FrontendError::Unsupported(_)), "{err}");
}

#[test]
fn the_limit_is_cumulative_across_registers() {
    let half = MAX_REGISTER_WIDTH / 2 + 1;
    let err = parse_openqasm3(&format!("qubit[{half}] a; qubit[{half}] b;")).unwrap_err();
    assert!(err.to_string().contains("wider than"), "{err}");
}

#[test]
fn a_register_at_the_limit_is_accepted() {
    let circuit = parse_openqasm3(&format!("qubit[{MAX_REGISTER_WIDTH}] q;")).unwrap();
    assert_eq!(circuit.num_qubits(), MAX_REGISTER_WIDTH);
}

#[test]
fn a_huge_cudaq_allocation_is_refused() {
    let source = format!(
        "import cudaq\n\n@cudaq.kernel\ndef k():\n    q = cudaq.qvector({})\n    h(q[0])\n",
        u64::from(MAX_REGISTER_WIDTH) + 1
    );
    let err = parse_cudaq(&source).unwrap_err();
    assert!(err.to_string().contains("more than"), "{err}");
}

#[test]
fn a_huge_qiskit_handoff_is_refused() {
    let ir = QiskitCircuitIr {
        name: "big".into(),
        num_qubits: u32::MAX,
        num_clbits: 0,
        instructions: vec![],
    };
    assert!(matches!(
        qiskit::translate(&ir),
        Err(FrontendError::Unsupported(_))
    ));
}

#[test]
fn a_huge_target_topology_is_refused() {
    let json = r#"{"id":"x","version":"1","backend_id":"simulator","cost_model_id":"uniform",
                   "topology":{"qubit_count":4294967295,"edges":[]}}"#;
    let err = oqci::backend::from_target_json(json, None, None)
        .err()
        .unwrap();
    assert!(err.to_string().contains("more than the limit"), "{err}");
    let err = BasisProfileBuilder::new(
        "x",
        "1",
        "sim",
        Topology::disconnected(MAX_REGISTER_WIDTH + 1),
    )
    .cost_model("uniform")
    .build()
    .unwrap_err();
    assert!(err.to_string().contains("more than the limit"), "{err}");
}

// --- Expression depth and size (found by fuzzing: stack overflow) -----------

#[test]
fn deeply_nested_expressions_are_refused_not_a_stack_overflow() {
    let nested = format!(
        "qubit[1] q; rz({}1{}) q[0];",
        "(".repeat(100_000),
        ")".repeat(100_000)
    );
    assert!(matches!(
        parse_openqasm3(&nested),
        Err(FrontendError::Unsupported(_))
    ));
    let signs = format!("qubit[1] q; rz({}1) q[0];", "-".repeat(200_000));
    assert!(matches!(
        parse_openqasm3(&signs),
        Err(FrontendError::Unsupported(_))
    ));
    let chain = format!("qubit[1] q; rz({}) q[0];", vec!["1"; 100_000].join("+"));
    assert!(matches!(
        parse_openqasm3(&chain),
        Err(FrontendError::Unsupported(_))
    ));
    let kernel = format!(
        "import cudaq\n\n@cudaq.kernel\ndef k():\n    q = cudaq.qvector(1)\n    rx({}1.0{}, q[0])\n",
        "(".repeat(100_000),
        ")".repeat(100_000)
    );
    assert!(matches!(
        parse_cudaq(&kernel),
        Err(FrontendError::Unsupported(_))
    ));
}

#[test]
fn ordinary_nested_expressions_are_unaffected() {
    let circuit =
        parse_openqasm3("qubit[1] q; rz(-(2*3.14159265/3) + -(-0.5) * (1 + (2 - 1))) q[0];")
            .unwrap();
    assert_eq!(circuit.len(), 1);
}

// --- Angle arithmetic (found by fuzzing: a miscompilation) -------------------

#[test]
fn a_huge_angle_is_never_miscompiled() {
    use oqci::compile::{CompileError, CompilerConfig, compile};
    use oqci::verify::VerifyConfig;
    // Merging these used to compute Rz(7.01e18) — the 0.4 rounded away — and
    // the in-compiler verifier caught it. Now the merge is skipped, and the
    // whole compilation verifies.
    let merge = "qubit[1] q; bit[1] c; h q[0]; rz(7009997822827970146.3) q[0]; rz(0.4) q[0]; \
                 sx q[0]; c[0] = measure q[0];";
    let config = CompilerConfig {
        backend: Some("simulator-nisq".into()),
        verify: VerifyConfig {
            enabled: true,
            ..VerifyConfig::default()
        },
        ..CompilerConfig::default()
    };
    let report = compile(merge, &config).unwrap().verification.unwrap();
    assert_eq!(report.not_equivalent, 0);

    // A rule that must transform such an angle refuses instead of rounding.
    let rx = "qubit[1] q; bit[1] c; rx(7009997822827970146.3) q[0]; c[0] = measure q[0];";
    let err = compile(rx, &config).unwrap_err();
    assert!(
        matches!(err, CompileError::Backend { .. })
            && err.to_string().contains("exactly in floating point"),
        "{err}"
    );
}

#[test]
fn huge_angles_are_never_turned_into_matrices() {
    use oqci::compile::{CompilerConfig, compile};
    use oqci::verify::{InconclusiveReason, Verdict, VerifyConfig};
    // Found by fuzzing: an exact merge of two huge rotations, followed by
    // fusion, built a U from cos(5.5e19) — which some platforms' libm get
    // wrong in the third decimal place. Such gates are now left unfused, and
    // the checker says it cannot decide rather than guessing.
    let source = "qubit[1] q; bit[1] c; h q[0]; rz(7009997822827970146.3) q[0]; \
                  rz(110337850872972520310.4) q[0]; sx q[0]; c[0] = measure q[0];";
    let config = CompilerConfig {
        backend: Some("simulator-nisq".into()),
        verify: VerifyConfig {
            enabled: true,
            ..VerifyConfig::default()
        },
        ..CompilerConfig::default()
    };
    let artifacts = compile(source, &config).unwrap();
    assert!(artifacts.optimized.instructions().iter().all(|i| !matches!(
        i,
        oqci::ir::Instruction::Gate {
            kind: oqci::ir::GateKind::U { .. },
            ..
        }
    )));
    let report = artifacts.verification.unwrap();
    assert_eq!(report.not_equivalent, 0);
    assert!(report.checks.iter().any(|c| matches!(
        c.verdict,
        Verdict::Inconclusive {
            reason: InconclusiveReason::AngleTooLarge { .. }
        }
    )));
}

#[test]
fn a_huge_cudaq_register_is_refused_before_a_broadcast_lists_it() {
    // Found by fuzzing: `h(q)` over `qvector(3000000000)` listed every qubit
    // — a 24 GiB allocation — before the width was ever checked.
    let source =
        "import cudaq\n\n@cudaq.kernel\ndef k():\n    q = cudaq.qvector(3000000000)\n    h(q)\n";
    let err = parse_cudaq(source).unwrap_err();
    assert!(err.to_string().contains("more than"), "{err}");
    // Two registers whose sum would overflow u32 are refused too.
    let wrap = "import cudaq\n\n@cudaq.kernel\ndef k():\n    a = cudaq.qvector(4294967295)\n    b = cudaq.qvector(2)\n    h(a)\n";
    assert!(parse_cudaq(wrap).is_err());
}
