//! Integration tests for the CUDA-Q frontend, end to end.
//!
//! The assertion style is *equivalence*, as in `tests/openqasm_pipeline.rs`:
//! a CUDA-Q kernel must emit byte-identical QIR to the same circuit written
//! in OpenQASM 3. That is what makes the CUDA-Q frontend a mapping onto the
//! existing IR rather than a second definition of it — the defect of the
//! standalone adapter it replaces, which carried its own private IR.

use std::collections::HashMap;

use oqci::frontend::{FrontendError, parse_cudaq_named, parse_openqasm3_named};
use oqci::ir::{bind_parameters, emit_qir, qc_to_qco};

fn qir(circuit: &oqci::ir::Circuit) -> String {
    emit_qir(&qc_to_qco(circuit).expect("conversion")).expect("lowering")
}

fn qir_of_cudaq(source: &str) -> String {
    qir(&parse_cudaq_named(source, "itest").expect("valid kernel"))
}

fn qir_of_qasm(source: &str) -> String {
    qir(&parse_openqasm3_named(source, "itest").expect("valid program"))
}

fn kernel(body: &str) -> String {
    let indented: Vec<String> = body.lines().map(|l| format!("    {l}")).collect();
    format!(
        "import cudaq\nimport math\n\n@cudaq.kernel\ndef k():\n{}\n\nif __name__ == \"__main__\":\n    print(cudaq.sample(k))\n",
        indented.join("\n")
    )
}

#[test]
fn bell_matches_openqasm() {
    assert_eq!(
        qir_of_cudaq(&kernel(
            "q = cudaq.qvector(2)\nh(q[0])\nx.ctrl(q[0], q[1])\nmz(q)"
        )),
        qir_of_qasm("qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;"),
    );
}

#[test]
fn per_qubit_measurement_matches_register_measurement() {
    // CUDA-Q's own documentation example measures qubit by qubit. That must
    // mean exactly what measuring the whole register means — not "the last
    // qubit measured", which is what the standalone adapter produced.
    let piecewise = qir_of_cudaq(&kernel(
        "q = cudaq.qvector(3)\nh(q[0])\nx.ctrl(q[0], q[1])\nx.ctrl(q[1], q[2])\nmz(q[0])\nmz(q[1])\nmz(q[2])",
    ));
    let whole = qir_of_cudaq(&kernel(
        "q = cudaq.qvector(3)\nh(q[0])\nx.ctrl(q[0], q[1])\nx.ctrl(q[1], q[2])\nmz(q)",
    ));
    let qasm =
        qir_of_qasm("qubit[3] q; bit[3] c; h q[0]; cx q[0], q[1]; cx q[1], q[2]; c = measure q;");
    assert_eq!(piecewise, qasm);
    assert_eq!(whole, qasm);
}

#[test]
fn rotations_and_adjoints_match_openqasm() {
    assert_eq!(
        qir_of_cudaq(&kernel(
            "q = cudaq.qvector(2)\nrx(math.pi / 2, q[0])\nry(-0.25, q[1])\nr1(0.5, q[0])\ns.adj(q[1])\nt.adj(q[0])\nswap(q[0], q[1])",
        )),
        qir_of_qasm(
            "qubit[2] q; rx(pi/2) q[0]; ry(-0.25) q[1]; p(0.5) q[0]; sdg q[1]; tdg q[0]; swap q[0], q[1];"
        ),
    );
}

#[test]
fn multiple_controls_match_a_toffoli() {
    assert_eq!(
        qir_of_cudaq(&kernel("q = cudaq.qvector(3)\nx.ctrl([q[0], q[1]], q[2])")),
        qir_of_qasm("qubit[3] q; ccx q[0], q[1], q[2];"),
    );
}

#[test]
fn a_float_parameter_binds_like_an_openqasm_input() {
    let cudaq = parse_cudaq_named(
        "@cudaq.kernel\ndef ansatz(theta: float):\n    q = cudaq.qubit()\n    ry(theta, q)\n",
        "itest",
    )
    .unwrap();
    let qasm = parse_openqasm3_named(
        "input float[64] theta; qubit[1] q; ry(theta) q[0];",
        "itest",
    )
    .unwrap();
    assert_eq!(cudaq.parameters(), qasm.parameters());

    let bindings = HashMap::from([("theta".to_string(), 0.75)]);
    assert_eq!(
        qir(&bind_parameters(&cudaq, &bindings).unwrap()),
        qir(&bind_parameters(&qasm, &bindings).unwrap()),
    );
}

#[test]
fn host_code_around_the_kernel_is_never_parsed() {
    // Arbitrary Python outside the kernel — including syntax the kernel
    // lexer would refuse — must not affect compilation.
    let source = "import cudaq, argparse\nparser = argparse.ArgumentParser()\nx = {'a': 2 ** 10}\nprint(f\"{x}\")\n\n@cudaq.kernel\ndef k():\n    q = cudaq.qubit()\n    h(q)\n\nresult = cudaq.sample(k)\n";
    let circuit = parse_cudaq_named(source, "itest").unwrap();
    assert_eq!(circuit.len(), 1);
}

#[test]
fn refusals_name_the_construct_and_its_line() {
    let error = parse_cudaq_named(
        &kernel("q = cudaq.qvector(2)\nfor i in range(2):\n    h(q[i])"),
        "itest",
    )
    .unwrap_err();
    let FrontendError::Unsupported(message) = error else {
        panic!("expected an unsupported-construct error");
    };
    assert!(message.contains("for"), "{message}");
    assert!(message.contains("line 7"), "{message}");
}
