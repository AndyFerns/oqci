# Qiskit Adapter

Status: normative
Implemented by: `src/frontend/qiskit/` (translation core) and `python/src/lib.rs` (PyO3 boundary)
Verified against: **Qiskit 2.5.2** (see [Version sensitivity](#version-sensitivity))

`final-deliverables-spec.md` §5.3 requires translation from a Qiskit
`QuantumCircuit` into QC-IR, with the adapter living "at the integration
boundary rather than contaminating QC-IR with Qiskit types".

## Architecture: where the decisions live

The adapter is split in two, and the split is the point:

```text
QuantumCircuit ──▶ QiskitCircuitIr ──▶ Circuit
   (Python)          (plain Rust)       (QC-IR)
      │                    │
  python/src/lib.rs   src/frontend/qiskit/
  thin extraction     every decision, no Python
```

- **`QiskitCircuitIr`** is a plain Rust struct: register widths plus an
  ordered instruction list carrying flat bit indices. No Qiskit type reaches
  it, and nothing downstream of it can tell the circuit came from Qiskit.
- **`translate`** turns that into a validated `Circuit`. It holds all the
  logic that can be wrong — gate mapping, operand order, measurement
  destinations, parameter handling — and is exercised by ordinary
  `cargo test` with **no Python interpreter present**
  (`tests/qiskit_adapter.rs`).
- **The PyO3 layer** only reads a live `QuantumCircuit` into that struct.

So the fragile dependency (Qiskit's Python API) and the substantive logic are
separated: the logic is testable without Qiskit, and the Qiskit-facing code is
short enough to audit by eye.

## What is translated

| Qiskit | QC-IR |
|---|---|
| `num_qubits` / `num_clbits` | register sizes |
| `circuit.data` order | instruction order (preserved exactly) |
| `find_bit(bit).index` | flat `QubitId` / `ClbitId` — collapsing Qiskit's multiple named registers into QC-IR's dense space |
| `operation.name` | `GateKind`, via [`gate_mapping.md`](gate_mapping.md) |
| `operation.params` (float) | `Param::Concrete` |
| `operation.params` (unbound `Parameter`) | `Param::Symbol` |
| `measure` | `Instruction::Measure`, paired element-wise |
| `reset` | `Instruction::Reset` |

No reordering occurs: `circuit.data` is already in program order and QC-IR is
an ordered list.

## Refusals and known limitations

| Case | Behaviour | Why |
|---|---|---|
| `if_else`, `while_loop`, `for_loop`, `switch_case`, `break_loop`, `continue_loop` | `FrontendError::Unsupported` | Dynamic control flow, deferred by Stage F. QC-IR cannot represent a conditional operation, so refusing is the only honest option. |
| Compound `ParameterExpression` (`2*theta`, `theta + phi`) | `FrontendError::Unsupported`, quoting the expression | QC-IR represents a *symbol*, not a symbolic expression (see [`ir_spec.md`](ir_spec.md) §1.1). Folding one would silently discard the arithmetic. Bind it in Qiskit first (`assign_parameters`) if a concrete value is what you want. |
| `barrier`, `delay` | **Dropped**, no instruction emitted | QC-IR has no barrier concept. This is recorded rather than silent: a later scheduling pass must not read a barrier-free circuit as evidence that none was written. |
| Unrecognised gate name | `GateKind::Opaque` | The escape hatch; see [`gate_mapping.md`](gate_mapping.md). |
| Out-of-range bit, duplicate operand, wrong arity | `FrontendError::Ir(..)` | Surfaced from QC-IR validation. The adapter does not re-implement those checks. |

## Python API

Built with `maturin`; the extension module is `oqci_native`.

```python
import oqci_native
from qiskit import QuantumCircuit
from qiskit.circuit import Parameter

qc = QuantumCircuit(2, 2, name="bell")
qc.h(0); qc.cx(0, 1); qc.measure([0, 1], [0, 1])

qir = oqci_native.qiskit_to_qir(qc)
```

| Function | Purpose |
|---|---|
| `qiskit_to_qir(circuit, bindings=None)` | Compile a `QuantumCircuit` to QIR text. `bindings` keys may be parameter names or `Parameter` objects. |
| `qiskit_parameters(circuit)` | The circuit's free parameters *as OQCI sees them*. A name Qiskit reports but this omits reached OQCI inside a compound expression. |
| `qasm3_to_qir(source, bindings=None)` | The OpenQASM 3 path, through the same IR and the same gate table. |

Errors raise `oqci_native.OqciError` (a `ValueError` subclass) carrying the
underlying diagnostic. A circuit with unbound parameters and no bindings is an
error — OQCI never invents a value.

## Building and testing

```bash
pip install -r python/requirements-dev.txt
maturin develop --manifest-path python/Cargo.toml
pytest python/tests
```

The Rust-side tests need none of this:

```bash
cargo test            # includes the full adapter core, no Python involved
```

## Version sensitivity

The extraction **duck-types** rather than importing Qiskit classes for
`isinstance` checks, so it survives Qiskit moving a class between modules — as
happened when `Parameter` moved into the Rust accelerator in 2.x. The checks:

1. a parameter that converts to `float` is concrete;
2. otherwise, one exposing `.name` is a bare `Parameter`;
3. otherwise it is a compound expression, reported via its `str()` form.

What it does rely on is the shape of `QuantumCircuit.data` (entries exposing
`.operation.name`, `.operation.params`, `.qubits`, `.clbits`) and
`find_bit(bit).index`. If a future Qiskit changes those, `python/tests` is what
will tell you: re-run it after raising the pin in
`python/requirements-dev.txt`.
