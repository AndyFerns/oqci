//! The PyO3 boundary: a live Qiskit `QuantumCircuit` → OQCI's QC-IR.
//!
//! This crate is deliberately **thin**. Every translation decision — gate
//! mapping, operand order, measurement destinations, parameter handling —
//! lives in `oqci::frontend::qiskit`, which is pure Rust and tested without a
//! Python interpreter. All this layer does is read a `QuantumCircuit` into the
//! vendor-neutral [`QiskitCircuitIr`] handoff struct and call `translate`.
//!
//! That split is what keeps the Qiskit dependency honest: the code that could
//! be *wrong* is testable in `cargo test`, and the code that needs Qiskit is
//! short enough to audit by eye.
//!
//! # Reading a `QuantumCircuit` without importing Qiskit
//!
//! The extraction below duck-types rather than importing `qiskit.circuit`
//! classes for `isinstance` checks, so it does not break when Qiskit moves a
//! class between modules (as it did when `Parameter` moved into the Rust
//! accelerator in 2.x). Verified against **Qiskit 2.5.2**; the checks used are:
//!
//! - a parameter that converts to `float` is concrete;
//! - otherwise, one exposing a `.name` attribute is a bare `Parameter`;
//! - otherwise it is a compound `ParameterExpression`, which QC-IR cannot
//!   represent, so it is reported via its `str()` form rather than folded.

use pyo3::create_exception;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};

use oqci::frontend::{FrontendError, QiskitCircuitIr, QiskitInstruction, QiskitParam};
use oqci::ir::{Circuit, bind_parameters, emit_qir, qc_to_qco};

create_exception!(
    oqci_native,
    OqciError,
    PyValueError,
    "A circuit OQCI cannot compile: malformed source, an unsupported construct, or an IR invariant violation."
);

fn to_py_err(error: &impl std::fmt::Display) -> PyErr {
    OqciError::new_err(error.to_string())
}

/// Reads a live `QuantumCircuit` into the vendor-neutral handoff struct.
fn read_circuit(circuit: &Bound<'_, PyAny>) -> PyResult<QiskitCircuitIr> {
    let name: String = circuit
        .getattr("name")
        .and_then(|n| n.extract())
        .unwrap_or_else(|_| "main".to_string());
    let num_qubits: u32 = circuit.getattr("num_qubits")?.extract()?;
    let num_clbits: u32 = circuit.getattr("num_clbits")?.extract()?;

    let mut instructions = Vec::new();
    for item in circuit.getattr("data")?.try_iter()? {
        let entry = item?;
        let operation = entry.getattr("operation")?;

        let mut params = Vec::new();
        for param in operation.getattr("params")?.try_iter()? {
            params.push(read_param(&param?)?);
        }

        instructions.push(QiskitInstruction {
            name: operation.getattr("name")?.extract()?,
            params,
            qubits: read_bits(circuit, &entry.getattr("qubits")?)?,
            clbits: read_bits(circuit, &entry.getattr("clbits")?)?,
        });
    }

    Ok(QiskitCircuitIr {
        name,
        num_qubits,
        num_clbits,
        instructions,
    })
}

/// Resolves a sequence of Qiskit `Bit`s to flat, circuit-wide indices via
/// `QuantumCircuit.find_bit`, which is the same dense numbering QC-IR uses.
fn read_bits(circuit: &Bound<'_, PyAny>, bits: &Bound<'_, PyAny>) -> PyResult<Vec<u32>> {
    let mut indices = Vec::new();
    for bit in bits.try_iter()? {
        let location = circuit.call_method1("find_bit", (bit?,))?;
        indices.push(location.getattr("index")?.extract()?);
    }
    Ok(indices)
}

fn read_param(param: &Bound<'_, PyAny>) -> PyResult<QiskitParam> {
    // A bound value converts straight to a float.
    if let Ok(value) = param.extract::<f64>() {
        return Ok(QiskitParam::Concrete(value));
    }
    // A bare `Parameter` carries its own name; a compound expression does not.
    if let Ok(name) = param.getattr("name").and_then(|n| n.extract::<String>()) {
        return Ok(QiskitParam::Symbol(name));
    }
    Ok(QiskitParam::Expression(param.str()?.to_string()))
}

fn read_bindings(
    bindings: Option<&Bound<'_, PyDict>>,
) -> PyResult<std::collections::HashMap<String, f64>> {
    let mut map = std::collections::HashMap::new();
    if let Some(dict) = bindings {
        for (key, value) in dict.iter() {
            let name: String = if let Ok(text) = key.downcast::<PyString>() {
                text.extract()?
            } else {
                // Accept a Qiskit `Parameter` object as a key, which is how
                // `assign_parameters` is normally called.
                key.getattr("name")
                    .map_err(|_| {
                        OqciError::new_err(
                            "binding keys must be parameter names or Parameter objects",
                        )
                    })?
                    .extract()?
            };
            map.insert(name, value.extract::<f64>()?);
        }
    }
    Ok(map)
}

/// Applies bindings (when given) and lowers to QIR text.
fn finish(circuit: Circuit, bindings: Option<&Bound<'_, PyDict>>) -> PyResult<String> {
    let bindings = read_bindings(bindings)?;
    let circuit = if bindings.is_empty() && circuit.is_concrete() {
        circuit
    } else {
        bind_parameters(&circuit, &bindings).map_err(|e| to_py_err(&e))?
    };
    let dag = qc_to_qco(&circuit).map_err(|e| to_py_err(&e))?;
    emit_qir(&dag).map_err(|e| to_py_err(&e))
}

/// Compiles a Qiskit `QuantumCircuit` to QIR text.
///
/// `bindings` supplies values for any unbound `Parameter`s; keys may be
/// parameter names or `Parameter` objects. A circuit with unbound parameters
/// and no bindings is an error — OQCI never invents a value.
#[pyfunction]
#[pyo3(signature = (circuit, bindings = None))]
fn qiskit_to_qir(
    circuit: &Bound<'_, PyAny>,
    bindings: Option<&Bound<'_, PyDict>>,
) -> PyResult<String> {
    let ir = read_circuit(circuit)?;
    let translated = oqci::frontend::qiskit::translate(&ir).map_err(|e| to_py_err(&e))?;
    finish(translated, bindings)
}

/// Returns the names of a `QuantumCircuit`'s free parameters, as OQCI sees
/// them, sorted.
///
/// This is a useful cross-check: a name missing here that Qiskit reports means
/// the parameter reached OQCI inside a compound expression, which is not
/// representable.
#[pyfunction]
fn qiskit_parameters(circuit: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    let ir = read_circuit(circuit)?;
    let translated = oqci::frontend::qiskit::translate(&ir).map_err(|e| to_py_err(&e))?;
    Ok(translated.parameters())
}

/// Compiles OpenQASM 3 source text to QIR, through the same IR and the same
/// gate table as the Qiskit path.
///
/// The supported subset is documented in `docs/openqasm_frontend.md`.
#[pyfunction]
#[pyo3(signature = (source, bindings = None))]
fn qasm3_to_qir(source: &str, bindings: Option<&Bound<'_, PyDict>>) -> PyResult<String> {
    let circuit: Circuit =
        oqci::frontend::parse_openqasm3(source).map_err(|e: FrontendError| to_py_err(&e))?;
    finish(circuit, bindings)
}

/// Native OQCI bindings.
#[pymodule]
fn oqci_native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add("OqciError", module.py().get_type::<OqciError>())?;
    module.add_function(wrap_pyfunction!(qiskit_to_qir, module)?)?;
    module.add_function(wrap_pyfunction!(qiskit_parameters, module)?)?;
    module.add_function(wrap_pyfunction!(qasm3_to_qir, module)?)?;
    Ok(())
}
