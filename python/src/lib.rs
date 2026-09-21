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

/// Compiles OpenQASM 3 source for a backend, returning the artifacts as a
/// plain dict.
///
/// The whole pipeline: frontend, optimization, target lowering and execution
/// preparation. Everything comes from `oqci::compile`, the same entry point
/// the CLI uses, so the Python SDK cannot disagree with the compiler about
/// what happened.
///
/// `backend` names one of `oqci_native.backends()`. Omitting it means
/// target-independent compilation, which stops after optimization.
#[pyfunction]
#[pyo3(signature = (source, backend = None, bindings = None, name = "main", shots = 1024, seed = None, layout = "trivial"))]
#[allow(clippy::too_many_arguments)]
fn compile_qasm3(
    py: Python<'_>,
    source: &str,
    backend: Option<&str>,
    bindings: Option<&Bound<'_, PyDict>>,
    name: &str,
    shots: u32,
    seed: Option<u64>,
    layout: &str,
) -> PyResult<PyObject> {
    use oqci::compile::{CompilerConfig, compile_named};
    use oqci::lowering::{LayoutChoice, LoweringConfig};

    let layout = match layout {
        "trivial" => LayoutChoice::Trivial,
        "dense" => LayoutChoice::Dense,
        other => {
            return Err(OqciError::new_err(format!(
                "unknown layout `{other}`; expected `trivial` or `dense`"
            )));
        }
    };

    let config = CompilerConfig {
        bindings: read_bindings(bindings)?,
        backend: backend.map(str::to_string),
        lowering: LoweringConfig {
            layout,
            ..LoweringConfig::default()
        },
        settings: oqci::backend::ExecutionSettings {
            shots,
            seed,
            memory: false,
        },
        ..CompilerConfig::default()
    };

    let artifacts = compile_named(source, name, &config).map_err(|e| to_py_err(&e))?;
    artifacts_to_dict(py, &artifacts)
}

/// Turns compilation artifacts into a dict.
///
/// This is the SDK's own view, and it is **not** the same shape as the CLI's
/// `--json` report: that one is built for reading a pipeline stage by stage,
/// this one for using the result programmatically. What they do share is the
/// parts that matter to a consumer — `executable` and `cost` are the same
/// serde types on both paths, so an artifact produced here and one printed by
/// `oqci prepare` are byte-identical.
///
/// The rest is assembled here because a Python caller wants different things
/// from a terminal reader; where that assembly duplicates a field the CLI
/// also renders, the duplication is in the presentation, not in the numbers.
fn artifacts_to_dict(
    py: Python<'_>,
    artifacts: &oqci::compile::CompilationArtifacts,
) -> PyResult<PyObject> {
    let mut body = serde_json::Map::new();
    body.insert(
        "backend".to_string(),
        match &artifacts.backend_id {
            Some(id) => serde_json::Value::String(id.clone()),
            None => serde_json::Value::Null,
        },
    );
    body.insert(
        "source_metrics".to_string(),
        serde_json::to_value(&artifacts.source_metrics).map_err(json_err)?,
    );
    body.insert(
        "optimized_metrics".to_string(),
        serde_json::to_value(&artifacts.optimized_metrics).map_err(json_err)?,
    );
    body.insert(
        "passes".to_string(),
        serde_json::Value::Array(
            artifacts
                .pass_records
                .iter()
                .map(|record| {
                    serde_json::json!({
                        "id": record.id,
                        "enabled": record.enabled,
                        "changed": record.changed,
                        "notes": record.notes,
                    })
                })
                .collect(),
        ),
    );
    if let Some(lowered) = &artifacts.lowered {
        body.insert(
            "lowering".to_string(),
            serde_json::json!({
                "profile": lowered.profile_id,
                "initial_layout": lowered.initial_layout.permutation(),
                "final_layout": lowered.final_layout.permutation(),
                "swaps_inserted": lowered.swaps_inserted,
                "orientations_repaired": lowered.orientations_repaired,
                "rules_applied": lowered.rules_applied,
                "legal": lowered.legality.is_legal(),
            }),
        );
    }
    if let Some(cost) = &artifacts.cost {
        body.insert(
            "cost".to_string(),
            serde_json::to_value(cost).map_err(json_err)?,
        );
    }
    if let Some(executable) = &artifacts.executable {
        body.insert(
            "executable".to_string(),
            serde_json::to_value(executable).map_err(json_err)?,
        );
    }
    json_to_py(py, &serde_json::Value::Object(body))
}

/// The backends this build can compile for, as `(id, description)` pairs.
#[pyfunction]
fn backends() -> Vec<(String, String)> {
    oqci::compile::available_backends()
}

/// The decomposition-rule library, as dicts.
///
/// Exported so `python/tests/test_rules.py` can re-check every identity
/// against Qiskit's `quantum_info.Operator` — a second, independent
/// implementation of gate semantics. Enumerating the real registry rather
/// than a hand-copied list is what stops the two drifting apart.
#[pyfunction]
fn decomposition_rules(py: Python<'_>) -> PyResult<PyObject> {
    let rules: Vec<serde_json::Value> = oqci::lowering::rules::all_builtin()
        .iter()
        .map(|rule| {
            serde_json::json!({
                "id": rule.id,
                "source": rule.source,
                "source_arity": rule.source_arity,
                "source_params": rule.source_params,
                "exactness": match rule.exactness {
                    oqci::lowering::Exactness::Exact => "exact",
                    oqci::lowering::Exactness::UpToGlobalPhase => "up_to_global_phase",
                },
                "note": rule.note,
                "steps": rule.steps.iter().map(|step| serde_json::json!({
                    "op": step.op,
                    "operands": step.operands,
                    "params": step.params.iter().map(|transform| match transform {
                        oqci::lowering::ParamTransform::Constant(value) => serde_json::json!({
                            "kind": "constant", "value": value,
                        }),
                        oqci::lowering::ParamTransform::Affine { index, scale, offset } =>
                            serde_json::json!({
                                "kind": "affine",
                                "index": index,
                                "scale": scale,
                                "offset": offset,
                            }),
                    }).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json_to_py(py, &serde_json::Value::Array(rules))
}

fn json_err(error: serde_json::Error) -> PyErr {
    OqciError::new_err(format!("cannot serialize compiler output: {error}"))
}

/// Converts a `serde_json::Value` into the equivalent Python object.
///
/// Written out rather than pulled in as a dependency: the conversion is
/// twenty lines, and a serde-to-Python crate would be a new dependency on the
/// boundary this crate exists to keep thin.
fn json_to_py(py: Python<'_>, value: &serde_json::Value) -> PyResult<PyObject> {
    use pyo3::types::{PyList, PyNone};
    Ok(match value {
        serde_json::Value::Null => PyNone::get(py).to_owned().into_any().unbind(),
        serde_json::Value::Bool(b) => b.into_pyobject(py)?.to_owned().into_any().unbind(),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into_pyobject(py)?.into_any().unbind()
            } else {
                n.as_f64()
                    .unwrap_or(f64::NAN)
                    .into_pyobject(py)?
                    .into_any()
                    .unbind()
            }
        }
        serde_json::Value::String(s) => s.into_pyobject(py)?.into_any().unbind(),
        serde_json::Value::Array(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(json_to_py(py, item)?)?;
            }
            list.into_any().unbind()
        }
        serde_json::Value::Object(fields) => {
            let dict = PyDict::new(py);
            for (key, item) in fields {
                dict.set_item(key, json_to_py(py, item)?)?;
            }
            dict.into_any().unbind()
        }
    })
}

/// Native OQCI bindings.
///
/// Nested inside the `oqci` package rather than sitting at the top level:
/// maturin's mixed layout requires the compiled module to live under the
/// pure-Python package it ships with, and `python/oqci_native.py` keeps the
/// original import path working for anything that already uses it.
#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add("OqciError", module.py().get_type::<OqciError>())?;
    module.add_function(wrap_pyfunction!(qiskit_to_qir, module)?)?;
    module.add_function(wrap_pyfunction!(qiskit_parameters, module)?)?;
    module.add_function(wrap_pyfunction!(qasm3_to_qir, module)?)?;
    module.add_function(wrap_pyfunction!(compile_qasm3, module)?)?;
    module.add_function(wrap_pyfunction!(backends, module)?)?;
    module.add_function(wrap_pyfunction!(decomposition_rules, module)?)?;
    Ok(())
}
