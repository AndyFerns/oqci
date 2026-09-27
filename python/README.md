# OQCI Python SDK

Two things live here: the compiled PyO3 extension that exposes OQCI's compiler
to Python, and the pure-Python package that wraps it into the SDK
`final-deliverables-spec.md` §17 describes — circuit import, compiler
invocation, configuration, backend selection, analysis and result access.

It is also where circuits actually **run**. The Rust compiler prepares
executables and stops there. `oqci.backends.aer` hands one to Qiskit Aer, and
`oqci.backends.cudaq` hands one to CUDA-Q. That split is deliberate, and
explained under [Execution](#execution).

## Layout

| Path | What it is |
|---|---|
| `src/lib.rs` | The PyO3 boundary. Reads a Qiskit `QuantumCircuit` into the vendor-neutral `QiskitCircuitIr` struct and hands it to `oqci::frontend::qiskit::translate`, which holds all the translation logic and is tested from Rust without a Python interpreter. Compiler entry points delegate to `oqci::compile`. |
| `oqci/` | The pure-Python package. Delegates; makes no compilation decision of its own. |
| `oqci/backends/_common.py` | What every execution adapter shares: `RunResult`, `UnsupportedOperation`, and the counts-key convention (clbit 0 rightmost). |
| `oqci/backends/aer.py` | Execution on Qiskit Aer. |
| `oqci/backends/cudaq.py` | Execution on CUDA-Q, which replaces the old root-level `cudaq-adapter/` package. See [`../docs/adapters.md`](../docs/adapters.md). |
| `oqci_native.py` | A compatibility shim, see below. |

The compiled module is `oqci._native`, nested inside the package because
maturin's mixed layout requires the extension to live under the pure-Python
package it ships with. **`import oqci_native` still works** — `oqci_native.py`
re-exports everything — so code and documentation written against the older
flat layout is unaffected. New code should prefer `import oqci`.

## Setup

```bash
pip install -r requirements-dev.txt
maturin develop --manifest-path Cargo.toml
pytest tests
```

`maturin develop` installs into the currently active virtualenv.

`requirements-dev.txt` now includes `qiskit-aer`, which the execution tests
need. The repository build script skips its Python stages when a dependency is
missing, so a machine without Aer is unaffected — the tests simply do not run
there rather than failing.

## Compiling

```python
import oqci

source = "qubit[3] q; bit[3] c; h q[0]; cx q[0], q[2]; c = measure q;"
artifacts = oqci.compile(source, backend="simulator-nisq", name="bell3")
```

`q0` and `q2` are two hops apart on a linear device, so this is a circuit that
does not fit as written:

```python
low = artifacts["lowering"]
low["swaps_inserted"]        # 1   — routing had to move a qubit
low["rules_applied"]         # ['h-to-rz-sx', 'swap-to-cx']
low["initial_layout"]        # [0, 1, 2]
low["final_layout"]          # [1, 0, 2]  — q0 ended up on physical #q1
low["legal"]                 # True
```

With no `backend`, compilation is target-independent and stops after
optimization. That is a complete result rather than a degraded one: there is
nothing to lower *to*, so `lowering` and `executable` are simply absent.

`oqci.compile` takes any of the three frontends, and every one reaches the
whole pipeline:

```python
oqci.compile(qasm_text, backend="simulator-nisq")                       # OpenQASM 3
oqci.compile(kernel_text, frontend="cudaq", backend="simulator-nisq")   # CUDA-Q source
oqci.compile(quantum_circuit, backend="simulator-nisq")                 # Qiskit QuantumCircuit
```

CUDA-Q is read as *source text*, so `cudaq` itself need not be installed. See
[`../docs/cudaq_frontend.md`](../docs/cudaq_frontend.md) for the subset.

Ablations take the CLI's shape: `passes=[...]` runs only those passes, and
`disable=[...]` runs every pass except those. Unknown ids, or both together,
are refused rather than silently running a different pipeline.

The dict's `passes`, `lowering` and circuit entries (`source_circuit`,
`optimized_circuit`, `lowered_circuit`) use the same schema as the CLI's
`--json` report; see [`../docs/compiler.md`](../docs/compiler.md#the-python-sdk).

`oqci.compile` never invents a parameter value. A circuit with a free
parameter and no binding is refused, with advice:

```python
oqci.compile("input float[64] t; qubit[1] q; rz(t) q[0];", backend="simulator")
# OqciError: parameter `t` is still symbolic; bind parameters before ...

oqci.compile(source, backend="simulator", bindings={"t": 0.5})   # fine
```

Other entry points:

| Function | What it gives you |
|---|---|
| `oqci.available_backends()` | `(id, description)` for every backend this build can compile for. |
| `oqci.decomposition_rules()` | The rule library, with each rule's source, steps and declared exactness. |
| `oqci.qasm3_to_qir(source)` | QIR text. A *lowering artifact*, not an execution guarantee — Stage C §5 is explicit that emitted QIR does not by itself mean a circuit will run. |
| `oqci.cudaq_to_qir(source)` | The same, from CUDA-Q kernel source. |
| `oqci.qiskit_to_qir(circuit)` | The same, from a live `QuantumCircuit`. |
| `oqci.qiskit_parameters(circuit)` | A circuit's free parameters as OQCI sees them. A useful cross-check: a name Qiskit reports and this does not means the parameter reached OQCI inside a compound expression, which is not representable. |

Failures raise `oqci.OqciError`, a `ValueError` subclass, carrying the
compiler's own message.

## Execution

```python
result = oqci.backends.aer.run(artifacts["executable"], shots=8000, seed=20260921)
result.counts            # {'000': 3975, '101': 4025}
result.probabilities()   # {'000': 0.4969, '101': 0.5031}
result.observed_shots    # 8000
```

`shots` and `seed` default to whatever the executable was prepared with, so a
run reproduces what its provenance record says it did; passing them explicitly
overrides both, and the returned provenance is updated to match rather than
continuing to claim the original values.

`oqci.backends.aer.to_qiskit(executable)` rebuilds the artifact as a
`QuantumCircuit` without running it — `print(to_qiskit(exe))` draws what OQCI
actually produced, which is a fast way to sanity-check a lowering against a
real diagram.

**Noise models are accepted and never invented.** Pass a
`qiskit_aer.noise.NoiseModel` and it is used; none is supplied by default.
Noise policy belongs to the benchmarking protocol, which is not locked, and a
built-in model with plausible-looking parameters would be fabricated
experimental data wearing a library's name.

### On CUDA-Q

```python
import oqci.backends.cudaq as cudaq_backend

print(cudaq_backend.to_cudaq_source(artifacts["executable"]))  # needs no CUDA-Q
result = cudaq_backend.run(artifacts["executable"], seed=7)     # needs `pip install oqci[cudaq]`
result.counts   # same keys as Aer's: clbit 0 rightmost
```

The same run from a shell, on an artifact written by `oqci prepare`:
`python -m oqci.backends.cudaq emit exe.json` or
`python -m oqci.backends.cudaq run exe.json --seed 7 [--target <name>]`.

The generated kernel uses only operations NVIDIA's CUDA-Q docs show in
Python. Anything the adapter cannot map exactly (`reset`, a gate after a
measurement, two measurements into one bit) raises `UnsupportedOperation`.
CUDA-Q's bit order is converted to Aer's, so the two runtimes' counts compare
key by key. **Not yet verified against a real CUDA-Q install**; see
[`../docs/adapters.md`](../docs/adapters.md#verification-status).

### Why execution is here and not in Rust

Every Rust `Backend::execute` returns a typed *not available in this process*
error. Two different reasons:

- **Simulators** — the project's non-goals rule out "a custom quantum
  simulator replacing established simulator frameworks". Writing one in Rust
  to satisfy the trait would trade an honest boundary for a violation.
- **Hardware** — submission needs `qiskit-ibm-runtime` and credentials,
  neither of which this project has, and §33.4 forbids implementing a vendor
  API from memory. **No claim is made that anything OQCI produces will run on
  a real quantum device.**

### Why the artifact is not OpenQASM

`qiskit.qasm3.loads` requires the separate `qiskit_qasm3_import` package,
which is not a dependency here. More importantly, putting a third-party parser
between "the circuit OQCI verified" and "the circuit that runs" would mean a
parser bug could silently change the program *after* verification. So the
executable is a structured operation list, and the adapter replays it by
calling one `QuantumCircuit` method per operation — every one of which was
checked to exist on the pinned Qiskit rather than recalled from memory.

## Tests

| File | What it covers |
|---|---|
| `tests/test_adapter.py` | The PyO3 boundary against a live Qiskit: instruction order, flat bit indices, bound floats vs. unbound `Parameter`s. |
| `tests/test_rules.py` | Every decomposition rule re-checked against `qiskit.quantum_info.Operator`, with each rule's exactness **derived from Qiskit's verdict** rather than trusted. The rule table is read out of the compiler, so it cannot drift from a hand-copied list. |
| `tests/test_aer.py` | Compile-and-execute, checking measured distributions. |
| `tests/test_frontends.py` | Qiskit and CUDA-Q through the full pipeline (including runs on Aer), ablations from Python, and the shared schema with the CLI. |
| `tests/test_backend_cudaq.py` | The CUDA-Q execution adapter, without CUDA-Q. The generated kernel is compiled back through OQCI's CUDA-Q frontend and must match the executable's unitary (`Operator.equiv`). Also covered: every refusal, the bit-order conversion checked against Aer's counts, and `run` against a stand-in `cudaq` module. |

`test_rules.py` is why the rule library can claim two independent oracles: the
Rust suite checks it against OQCI's own state-vector harness, and this checks
it against an implementation written by different people with its own
conventions. An error has to be made identically in both to survive.

`test_aer.py` is the first place in the project where OQCI's output is checked
against something other than OQCI. A lowering that is internally consistent
and physically wrong fails there and nowhere else — which is why it compiles a
Bell pair between `q0` and `q2` onto a *linear* device, forcing a SWAP and an
`h` that has to be rewritten, and then checks that the correlation is still
between the qubits the program asked for.

## Scope

Covered: all three frontends (OpenQASM 3, CUDA-Q, Qiskit) through the whole
pipeline, backend selection, layout and pass configuration, the rule library,
and simulator execution on Aer (and on CUDA-Q, unverified).

Not covered yet, and worth knowing before you reach for them:

- **Circuit construction.** There is no builder API; build a circuit in Qiskit
  and pass it in, or write OpenQASM / CUDA-Q text.
- **Verified execution on CUDA-Q.** `oqci.backends.cudaq` exists and is
  tested up to the runtime call, but it has never run against a real CUDA-Q.
  Compare it against Aer on the same executable before trusting its counts.
- **Hardware execution**, per above.
