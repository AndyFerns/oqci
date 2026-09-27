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

/// Compiles a CUDA-Q kernel's source text to QIR. Reads source text, not a
/// live kernel object, so CUDA-Q itself need not be installed.
///
/// The supported subset is documented in `docs/cudaq_frontend.md`.
#[pyfunction]
#[pyo3(signature = (source, bindings = None))]
fn cudaq_to_qir(source: &str, bindings: Option<&Bound<'_, PyDict>>) -> PyResult<String> {
    let circuit: Circuit =
        oqci::frontend::parse_cudaq(source).map_err(|e: FrontendError| to_py_err(&e))?;
    finish(circuit, bindings)
}

/// Options every `compile_*` entry point shares, read from Python once.
struct CompileOptions<'py> {
    backend: Option<&'py str>,
    bindings: Option<&'py Bound<'py, PyDict>>,
    shots: u32,
    seed: Option<u64>,
    layout: &'py str,
    passes: Option<Vec<String>>,
    disable: Option<Vec<String>>,
}

impl CompileOptions<'_> {
    fn config(&self) -> PyResult<oqci::compile::CompilerConfig> {
        use oqci::compile::CompilerConfig;
        use oqci::lowering::{LayoutChoice, LoweringConfig};

        let layout = match self.layout {
            "trivial" => LayoutChoice::Trivial,
            "dense" => LayoutChoice::Dense,
            other => {
                return Err(OqciError::new_err(format!(
                    "unknown layout `{other}`; expected `trivial` or `dense`"
                )));
            }
        };

        Ok(CompilerConfig {
            bindings: read_bindings(self.bindings)?,
            passes: pass_selection(self.passes.as_deref(), self.disable.as_deref())?,
            backend: self.backend.map(str::to_string),
            lowering: LoweringConfig {
                layout,
                ..LoweringConfig::default()
            },
            settings: oqci::backend::ExecutionSettings {
                shots: self.shots,
                seed: self.seed,
                memory: false,
            },
            ..CompilerConfig::default()
        })
    }
}

/// Builds a pass selection from `passes=` / `disable=`, refusing unknown ids
/// and the two together — the same rules as the CLI's `--passes`/`--disable`,
/// so an ablation cannot silently run a different pipeline than asked for.
fn pass_selection(
    only: Option<&[String]>,
    except: Option<&[String]>,
) -> PyResult<oqci::pass::PassSelection> {
    use oqci::pass::{PassManager, PassSelection};

    let known = PassManager::default_pipeline().pass_ids();
    let check = |ids: &[String]| -> PyResult<()> {
        for id in ids {
            if !known.contains(&id.as_str()) {
                return Err(OqciError::new_err(format!(
                    "unknown pass `{id}`; available: {}",
                    known.join(", ")
                )));
            }
        }
        Ok(())
    };
    match (only, except) {
        (None, None) => Ok(PassSelection::All),
        (Some(ids), None) => {
            check(ids)?;
            Ok(PassSelection::only(ids.iter().cloned()))
        }
        (None, Some(ids)) => {
            check(ids)?;
            Ok(PassSelection::all_except(ids.iter().cloned()))
        }
        (Some(_), Some(_)) => Err(OqciError::new_err(
            "`passes` and `disable` cannot be combined; use one or the other",
        )),
    }
}

/// Compiles source text through the whole pipeline, returning the artifacts
/// as a plain dict.
///
/// `frontend` is `"openqasm3"` or `"cudaq"` (the `@cudaq.kernel` function in
/// Python source). Everything comes from `oqci::compile`, the same entry
/// point the CLI uses, so the SDK cannot disagree with the compiler about
/// what happened. `backend` names one of `backends()`; omitting it means
/// target-independent compilation, which stops after optimization.
#[pyfunction]
#[pyo3(signature = (source, frontend = "openqasm3", backend = None, bindings = None, name = "main", shots = 1024, seed = None, layout = "trivial", passes = None, disable = None))]
#[allow(clippy::too_many_arguments)]
fn compile_source<'py>(
    py: Python<'py>,
    source: &str,
    frontend: &str,
    backend: Option<&'py str>,
    bindings: Option<&'py Bound<'py, PyDict>>,
    name: &str,
    shots: u32,
    seed: Option<u64>,
    layout: &'py str,
    passes: Option<Vec<String>>,
    disable: Option<Vec<String>>,
) -> PyResult<PyObject> {
    use oqci::compile::{Frontend, compile_named};

    let selected = Frontend::from_id(frontend).ok_or_else(|| {
        let known: Vec<&str> = Frontend::ALL.iter().map(|f| f.id()).collect();
        OqciError::new_err(format!(
            "unknown frontend `{frontend}`; available: {}",
            known.join(", ")
        ))
    })?;
    let options = CompileOptions {
        backend,
        bindings,
        shots,
        seed,
        layout,
        passes,
        disable,
    };
    let config = oqci::compile::CompilerConfig {
        frontend: selected,
        ..options.config()?
    };
    let artifacts = compile_named(source, name, &config).map_err(|e| to_py_err(&e))?;
    artifacts_to_dict(py, selected.id(), &artifacts)
}

/// Compiles OpenQASM 3 source. Kept for callers of the original API; it is
/// [`compile_source`] with `frontend="openqasm3"`.
#[pyfunction]
#[pyo3(signature = (source, backend = None, bindings = None, name = "main", shots = 1024, seed = None, layout = "trivial", passes = None, disable = None))]
#[allow(clippy::too_many_arguments)]
fn compile_qasm3<'py>(
    py: Python<'py>,
    source: &str,
    backend: Option<&'py str>,
    bindings: Option<&'py Bound<'py, PyDict>>,
    name: &str,
    shots: u32,
    seed: Option<u64>,
    layout: &'py str,
    passes: Option<Vec<String>>,
    disable: Option<Vec<String>>,
) -> PyResult<PyObject> {
    compile_source(
        py,
        source,
        "openqasm3",
        backend,
        bindings,
        name,
        shots,
        seed,
        layout,
        passes,
        disable,
    )
}

/// Compiles a Qiskit `QuantumCircuit` through the whole pipeline.
///
/// Until this existed a Qiskit circuit could reach QIR but not a backend:
/// `compile` only accepted text. The adapter now enters the orchestrator at
/// `oqci::compile::compile_circuit`, one step after where a text frontend
/// would, so a Qiskit circuit gets optimization, lowering, an executable and
/// a provenance record exactly like an OpenQASM program. `name` defaults to
/// the circuit's own `name`.
#[pyfunction]
#[pyo3(signature = (circuit, backend = None, bindings = None, name = None, shots = 1024, seed = None, layout = "trivial", passes = None, disable = None))]
#[allow(clippy::too_many_arguments)]
fn compile_qiskit<'py>(
    py: Python<'py>,
    circuit: &Bound<'py, PyAny>,
    backend: Option<&'py str>,
    bindings: Option<&'py Bound<'py, PyDict>>,
    name: Option<String>,
    shots: u32,
    seed: Option<u64>,
    layout: &'py str,
    passes: Option<Vec<String>>,
    disable: Option<Vec<String>>,
) -> PyResult<PyObject> {
    let mut ir = read_circuit(circuit)?;
    if let Some(name) = name {
        ir.name = name;
    }
    let translated = oqci::frontend::qiskit::translate(&ir).map_err(|e| to_py_err(&e))?;
    let options = CompileOptions {
        backend,
        bindings,
        shots,
        seed,
        layout,
        passes,
        disable,
    };
    let artifacts = oqci::compile::compile_circuit(translated, &options.config()?)
        .map_err(|e| to_py_err(&e))?;
    artifacts_to_dict(py, "qiskit", &artifacts)
}

/// Turns compilation artifacts into a dict.
///
/// Every section is built from the same serde types the CLI's `--json`
/// report uses — `ResourceReport`, `PassRecordView`, `LoweringView`,
/// `InstructionView`, `Cost`, `Executable` — through the same
/// `oqci::cli::snapshot` builders, so a field present in both means the same
/// thing in both. (It used to assemble its own `passes` and `lowering`
/// objects, which had already drifted from the CLI's: no `steps`, no
/// `violations`, no pass metrics.) The top-level layout stays the SDK's own,
/// keyed by what a program wants rather than by pipeline stage.
fn artifacts_to_dict(
    py: Python<'_>,
    frontend: &str,
    artifacts: &oqci::compile::CompilationArtifacts,
) -> PyResult<PyObject> {
    use oqci::cli::snapshot::{PassRecordView, instructions_of, lowering_view};

    let mut body = serde_json::Map::new();
    body.insert("frontend".to_string(), serde_json::Value::from(frontend));
    body.insert(
        "backend".to_string(),
        match &artifacts.backend_id {
            Some(id) => serde_json::Value::String(id.clone()),
            None => serde_json::Value::Null,
        },
    );
    body.insert(
        "unbound_parameters".to_string(),
        serde_json::to_value(artifacts.final_circuit().parameters()).map_err(json_err)?,
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
        "source_circuit".to_string(),
        serde_json::to_value(instructions_of(&artifacts.source_circuit)).map_err(json_err)?,
    );
    body.insert(
        "optimized_circuit".to_string(),
        serde_json::to_value(instructions_of(&artifacts.optimized)).map_err(json_err)?,
    );
    body.insert(
        "passes".to_string(),
        serde_json::to_value(
            artifacts
                .pass_records
                .iter()
                .map(PassRecordView::new)
                .collect::<Vec<_>>(),
        )
        .map_err(json_err)?,
    );
    if let Some(lowered) = &artifacts.lowered {
        let backend = artifacts.backend_id.as_deref().unwrap_or_default();
        body.insert(
            "lowering".to_string(),
            serde_json::to_value(lowering_view(backend, lowered)).map_err(json_err)?,
        );
        body.insert(
            "lowered_circuit".to_string(),
            serde_json::to_value(instructions_of(&lowered.circuit)).map_err(json_err)?,
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
    module.add_function(wrap_pyfunction!(cudaq_to_qir, module)?)?;
    module.add_function(wrap_pyfunction!(compile_source, module)?)?;
    module.add_function(wrap_pyfunction!(compile_qasm3, module)?)?;
    module.add_function(wrap_pyfunction!(compile_qiskit, module)?)?;
    module.add_function(wrap_pyfunction!(backends, module)?)?;
    module.add_function(wrap_pyfunction!(decomposition_rules, module)?)?;
    Ok(())
}
