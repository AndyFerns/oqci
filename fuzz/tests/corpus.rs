//! Every seed in `corpus/` through every fuzz target, on any toolchain.
//!
//! This is not fuzzing — it is the regression floor: each target must accept
//! each seed (and some deliberately hostile inputs) without panicking, and
//! `compile_verified` must find no miscompilation in any of them.

use std::fs;

fn inputs() -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    for dir in fs::read_dir("corpus").unwrap() {
        for file in fs::read_dir(dir.unwrap().path()).unwrap() {
            out.push(fs::read(file.unwrap().path()).unwrap());
        }
    }
    // Hostile inputs: empty, binary, truncated, deeply nested, huge widths.
    out.push(Vec::new());
    out.push((0u8..=255).collect());
    out.push(b"OPENQASM 3.0; qubit[4294967295] q;".to_vec());
    out.push(b"qubit[2] q; cx q[0], q[0];".to_vec());
    out.push(b"qubit[1] q; rz(1e400) q[0];".to_vec());
    out.push(b"{\"topology\": {\"qubit_count\": 4294967295, \"edges\": []}}".to_vec());
    out.push(b"@cudaq.kernel\ndef k():\n    q = cudaq.qvector(-1)\n".to_vec());
    out.push(vec![b'('; 10_000]);
    out
}

#[test]
fn every_target_survives_every_seed() {
    let all = inputs();
    assert!(all.len() > 20, "the corpus is missing");
    for data in &all {
        oqci_fuzz::openqasm(data);
        oqci_fuzz::cudaq(data);
        oqci_fuzz::target_json(data);
        oqci_fuzz::qiskit_handoff(data);
        oqci_fuzz::compile_verified(data);
    }
}
