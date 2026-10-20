//! The fuzz targets' bodies, as plain functions (final-deliverables-spec.md §23.3).
//!
//! `fuzz_targets/*.rs` wrap each in libFuzzer for `cargo +nightly fuzz run <name>`;
//! `tests/corpus.rs` runs every seed in `corpus/` through all of them on any
//! toolchain, so the targets are exercised even where libFuzzer cannot build.

use oqci::compile::{CompileError, CompilerConfig, compile};
use oqci::frontend::{QiskitCircuitIr, QiskitInstruction, QiskitParam};
use oqci::verify::VerifyConfig;

const NAMES: &[&str] = &[
    "h", "x", "y", "z", "s", "sdg", "t", "tdg", "sx", "rx", "ry", "rz", "p", "u", "cx", "cy", "cz",
    "swap", "ccx", "measure", "reset", "barrier", "id", "if_else", "iswap", "",
];

/// The OpenQASM 3 frontend never panics on any input: it returns a circuit
/// (which, by construction, has passed I1-I8) or a typed error.
pub fn openqasm(data: &[u8]) {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = oqci::frontend::parse_openqasm3(text);
    }
}

/// The CUDA-Q frontend never panics on any input.
pub fn cudaq(data: &[u8]) {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = oqci::frontend::parse_cudaq(text);
    }
}

/// A target description from outside the process (Â§29: safe parsing of
/// untrusted input) is either loaded â€” and then compiles a small circuit
/// without panicking â€” or refused with a typed error.
pub fn target_json(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let mut registry = oqci::extension::Registry::default();
    let Ok(id) = registry.register_target_json(
        oqci::extension::ExtensionOrigin::new("fuzz", "0"),
        text,
        None,
        Some("fuzzed"),
    ) else {
        return;
    };
    let config = oqci::compile::CompilerConfig {
        backend: Some(id),
        extensions: registry,
        ..Default::default()
    };
    let _ = oqci::compile::compile("qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;", &config);
}

/// The Qiskit translation core accepts any handoff struct the PyO3 layer
/// could build â€” arbitrary names, operands and parameters â€” without panicking.
pub fn qiskit_handoff(data: &[u8]) {
    let mut bytes = data.iter().copied();
    let mut next = || bytes.next();
    let (Some(nq), Some(nc)) = (next(), next()) else {
        return;
    };
    let mut circuit = QiskitCircuitIr {
        name: "fuzz".into(),
        num_qubits: u32::from(nq % 8),
        num_clbits: u32::from(nc % 8),
        instructions: Vec::new(),
    };
    while let Some(op) = next() {
        let name = NAMES[usize::from(op) % NAMES.len()].to_string();
        let count = usize::from(next().unwrap_or(0) % 4);
        let qubits = (0..count).map(|_| u32::from(next().unwrap_or(0) % 10)).collect();
        let clbits = (0..usize::from(next().unwrap_or(0) % 2))
            .map(|_| u32::from(next().unwrap_or(0) % 10))
            .collect();
        let params = (0..usize::from(next().unwrap_or(0) % 4))
            .map(|_| match next().unwrap_or(0) % 4 {
                0 => QiskitParam::Concrete(f64::from(next().unwrap_or(0)) - 128.0),
                1 => QiskitParam::Concrete(f64::NAN),
                2 => QiskitParam::Symbol(format!("t{}", next().unwrap_or(0) % 3)),
                _ => QiskitParam::Expression("2*t".into()),
            })
            .collect();
        circuit.instructions.push(QiskitInstruction { name, params, qubits, clbits });
    }
    let _ = oqci::frontend::qiskit::translate(&circuit);
}

/// A semantic fuzz target: any program the OpenQASM frontend accepts, compiled
/// for a constrained target with in-compiler verification on, must never be
/// found NotEquivalent. A `SemanticMismatch` here is a miscompilation, and it
/// panics so libFuzzer records the input.
pub fn compile_verified(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    for backend in ["simulator-nisq", "ibm-illustrative", "qir-base"] {
        let config = CompilerConfig {
            backend: Some(backend.into()),
            verify: VerifyConfig { enabled: true, max_qubits: 5, ..VerifyConfig::default() },
            ..CompilerConfig::default()
        };
        if let Err(CompileError::SemanticMismatch { transformation, detail }) = compile(text, &config) {
            panic!("miscompilation on {backend}: {transformation}: {detail}");
        }
    }
}
