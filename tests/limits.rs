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
