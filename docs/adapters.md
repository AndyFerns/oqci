# Language and Runtime Adapters — Layout

This document fixes **where** each quantum-language adapter lives and **what
shape** it has, so that OpenQASM 3, CUDA-Q, Qiskit and, later, Cirq all look
the same. It also documents the CUDA-Q execution adapter, and what became of
the root-level `cudaq-adapter/` prototype it replaces.

## Two halves, two homes

Stage C and `final-deliverables-spec.md` §5 and §15 split every vendor
integration into two independent halves:

| Half | Contract | Lives in | Why there |
|---|---|---|---|
| **Frontend** — a program *in* the language → QC-IR | `external program → validated QC-IR` (§5.1) | Rust, `src/frontend/<language>/` | One IR, one validator (`CircuitBuilder`), one gate table (`map_gate`). The CLI, the server, the Python SDK and the tests all reach it, and none needs the vendor's SDK installed. |
| **Execution adapter** — a prepared `Executable` → counts | `executable → RunResult` | Python, `python/oqci/backends/<runtime>.py` | The runtimes (Aer, CUDA-Q) are Python libraries. The compiler stops at a validated executable; running it is deliberately outside the Rust crate (`Backend::execute` says so with a typed error). |

A language whose programs are **objects** rather than text (Qiskit's
`QuantumCircuit`, Cirq's `Circuit`) needs one more piece. The PyO3 layer
(`python/src/lib.rs`) reads the object into a vendor-neutral handoff struct,
such as `QiskitCircuitIr`. The Rust frontend then translates that struct into
QC-IR, so no vendor type crosses into the compiler.

```text
                         Rust (src/)                                  Python (python/oqci/)
 text ──────────► frontend/<lang>/ ──┐
                                      ├─► QC-IR ─► passes ─► lowering ─► Executable ─► backends/<runtime>.py ─► counts
 object ─► lib.rs handoff struct ─► frontend/<lang>/ ─┘
```

### Where each adapter stands

| Language / runtime | Frontend | Execution adapter |
|---|---|---|
| OpenQASM 3 | `src/frontend/openqasm/` | — (a language, not a runtime) |
| CUDA-Q | `src/frontend/cudaq/` ([subset](cudaq_frontend.md)) | `python/oqci/backends/cudaq.py` ([below](#the-cuda-q-execution-adapter)) |
| Qiskit | `src/frontend/qiskit/` + `python/src/lib.rs` ([adapter](qiskit_adapter.md)) | `python/oqci/backends/aer.py` (Qiskit Aer) |
| Cirq | not started | not started |

### Rules every adapter follows

**Frontends**

- Live in `src/frontend/<language>/`. They build only through
  `CircuitBuilder`, resolve every gate name through `map_gate`, and report
  what they cannot represent as `FrontendError::Unsupported`, naming the
  construct.
- A text frontend is a `Frontend` enum variant (`src/compile.rs`): one
  variant, one arm in `Frontend::parse`, one id, and optionally a file
  extension in `Frontend::for_path`. An object frontend enters through
  `compile_circuit` instead.
- Tested in `tests/<language>_frontend.rs`, by equivalence with the same
  circuit written in OpenQASM 3.
- Documented in `docs/<language>_frontend.md`, with the exact accepted
  subset and the vendor pages it was verified against (§33.4).

**Execution adapters**

- One module per runtime, `python/oqci/backends/<runtime>.py`, imported
  lazily by `oqci.backends`. Each defers importing its runtime until it
  actually runs, so `import oqci` never needs it.
- Take the `executable` dict and nothing else. They never re-parse a
  program, because the artifact that runs must be the one OQCI verified.
- Expose `run(executable, *, shots=None, seed=None, …)` returning a subclass
  of `RunResult` (`python/oqci/backends/_common.py`). `shots` and `seed`
  default to the executable's settings, and the returned provenance records
  the values actually used.
- Expose a pure translation that needs no runtime installed
  (`to_qiskit`, `to_cudaq_source`), so the artifact can be inspected
  without being run.
- Raise the shared `UnsupportedOperation` for anything they cannot replay
  **exactly**, and never approximate silently.
- Return counts in **one key convention**: one character per classical
  bit, with **clbit 0 rightmost** (Qiskit's). A runtime that orders bits
  differently is converted inside its adapter, so two runtimes' results
  compare key by key.
- Declare their runtime as an optional extra in `python/pyproject.toml`.
- Tested in `python/tests/test_backend_<runtime>.py`, or `test_aer.py` for
  the adapter that predates this layout.

### Adding Cirq, as a worked checklist

1. `python/src/lib.rs`: read a `cirq.Circuit` into a `CirqCircuitIr`
   handoff struct, following `read_circuit` for Qiskit.
2. `src/frontend/cirq/`: translate the handoff struct into QC-IR through
   `CircuitBuilder`/`map_gate`, then `compile_circuit`.
3. `oqci.compile(cirq_circuit, ...)`: dispatch on the object's type, as is
   done for `QuantumCircuit`.
4. `python/oqci/backends/cirq.py`: `to_cirq(executable)`, plus
   `run(executable, ...)` on `cirq.Simulator`, returning a `RunResult`
   re-keyed to clbit 0 rightmost.
5. `docs/cirq_frontend.md`, a row in the table above, and tests on both
   halves.

## The CUDA-Q execution adapter

`oqci.backends.cudaq` runs an executable on CUDA-Q:

```python
import oqci
import oqci.backends.cudaq

exe = oqci.compile(program, backend="simulator-nisq")["executable"]
print(oqci.backends.cudaq.to_cudaq_source(exe))   # inspect; needs no CUDA-Q
result = oqci.backends.cudaq.run(exe, seed=7)     # needs CUDA-Q installed
```

It can also be run from a shell, on an executable written by `oqci prepare`:

```bash
oqci prepare examples/bell_cudaq.py --backend simulator-nisq -o bell.json
python -m oqci.backends.cudaq emit bell.json      # print the kernel
python -m oqci.backends.cudaq run bell.json --seed 7 [--target <cudaq-target>]
```

### Translation

Each operation becomes a statement in a generated `@cudaq.kernel`. Only
forms that NVIDIA's CUDA-Q *Quantum Operations* page shows in Python are
used (checked 2026-09-26/27):

| Executable op | CUDA-Q statement | Note |
|---|---|---|
| `x y z h s t` | same name | |
| `sdg`, `tdg` | `s.adj(q)`, `t.adj(q)` | `sdg`/`tdg` do not appear in CUDA-Q's docs |
| `rx ry rz` | `rx(θ, q)` … | angle first |
| `p` | `r1(λ, q)` | documented matrix `diag(1, e^{iλ})` |
| `u` | `u3(θ, φ, λ, q)` | documented matrix equals Qiskit's `U(θ, φ, λ)` |
| `sx`, `sxdg` | `rx(±π/2, q)` | not in CUDA-Q's docs. Equal up to a global phase, which no sampled probability can see; it would not be exact under a control, and nothing controls it here. |
| `cx cy cz` | `x.ctrl(c, t)` … | |
| `ccx` | `x.ctrl([c0, c1], t)` | list-of-controls form |
| `swap` | `swap(a, b)` | |
| `id` | nothing | no effect on anything sampled |
| `measure` | `mz(q[i])`, all at the end, ascending by qubit | see below |

Angles are written with Python's `repr`, which round-trips a float exactly,
so CUDA-Q receives bit for bit the angle OQCI verified.

### Refused

The adapter refuses rather than risk running a different circuit:

- `reset`: no documented Python form.
- A gate on a qubit after it was measured, or a second measurement of the same
  qubit. CUDA-Q's default sampling reports each measured qubit's value *at the
  end of the kernel*, so either would mean something different from what it
  means on Aer.
- Two measurements into one classical bit.
- An operation with no mapping, or a non-finite angle.
- `run` on an executable that measures nothing, as `aer.run` also refuses.

Because gates after a measurement are refused, moving every `mz` to the
end is exact.

### Bit order

CUDA-Q documents that when a kernel measures, only the measured qubits
appear in the `__global__` register, and "the `[0]` element … corresponds
with the first remaining declared qubit". With one `qvector`, character `k`
therefore belongs to the `k`-th measured qubit in ascending order.
`counts_from_cudaq` moves each character to its clbit's position in the shared
convention. It raises an error, rather than guessing, if a bitstring's
length is not the number of measured qubits.

### Kernel loading

CUDA-Q's kernel decorator works from a function's **source**. `run` writes
the generated kernel to a temporary `.py` file and imports it, exactly as a
user's own kernel would be loaded. The prototype called `exec` on a string,
which leaves the function with no source file to read.

### Verification status

| Checked | How |
|---|---|
| The generated kernel implements the executable's unitary | Compiled *back* through OQCI's own CUDA-Q frontend, and compared with Qiskit's `Operator.equiv` against the executable's circuit. A swapped `u3` argument, `sdg`→`s`, or a mis-ordered Toffoli all fail it. |
| Every refusal | One parametrized test per refusal. |
| The bit-order conversion | Aer's counts for an asymmetric, cross-wired executable are converted to CUDA-Q's documented order and back, and must come back unchanged. |
| `run`'s plumbing (seed, target, shots, kernel loading) | A stand-in `cudaq` module in `python/tests/test_backend_cudaq.py`. |
| **Real CUDA-Q returns what its docs say** | **Not yet.** No CUDA-Q installation was available (Windows). Before trusting its numbers, run the same executable through `aer.run` and `cudaq.run` on a machine with CUDA-Q and compare distributions, then pin that CUDA-Q version in `pyproject.toml`'s `cudaq` extra. |

## What became of `cudaq-adapter/`

The prototype sat at the repository root as a separate package
(`oqci-cudaq-adapters`) with its own private IR. Every part that does a job
OQCI needs has been kept, either moved or rewritten against the real
contracts:

| Prototype file | Now | Change |
|---|---|---|
| `oqci_cudaq/frontend.py` + `ir.py` | `src/frontend/cudaq/` | Rewritten earlier in Rust against QC-IR. It fixed two defects: per-qubit `mz` calls kept only the last, and non-finite angles were accepted. |
| `oqci_cudaq/backend.py` | `python/oqci/backends/cudaq.py` | Rewritten to consume the real `Executable` rather than the private IR. The rewrite fixes six defects: undocumented `sdg`/`tdg` emitted; clbit destinations ignored; bit order never converted; `exec`'d source the kernel decorator cannot read; a hard-coded `qpp-cpu` target; and a silent `{"result": str(result)}` fallback when counts could not be read. |
| `oqci_cudaq/cli.py` | `python -m oqci.backends.cudaq` (`emit`, `run`) | The `frontend` subcommand is now `oqci compile kernel.py`, and `backend` is now `emit`. |
| `examples/bell_cudaq.py` | `examples/bell_cudaq.py` | Now covered by `tests/cli.rs::every_example_compiles`. |
| `tests/test_backend.py` | `python/tests/test_backend_cudaq.py` | Its Bell check is subsumed by the round-trip unitary test. |
| `tests/test_frontend.py` | `tests/cudaq_frontend.rs`, `python/tests/test_frontends.py` | |
| `pyproject.toml`'s `cudaq` extra | `python/pyproject.toml`'s `cudaq` extra | |
| `README.md` | this document | |

The whole prototype remains in git history, at `ae31dbb` on `master`.
