# OQCI Python Bindings

PyO3 bindings exposing OQCI's frontends and QIR emission to Python. The
extension module is `oqci_native`.

This crate is deliberately thin: it reads a Qiskit `QuantumCircuit` into the
vendor-neutral `QiskitCircuitIr` struct and hands it to
`oqci::frontend::qiskit::translate`, which holds all the translation logic and
is tested from Rust without a Python interpreter. See
[`../docs/qiskit_adapter.md`](../docs/qiskit_adapter.md) for the full contract.

## Setup

```bash
pip install -r requirements-dev.txt
maturin develop --manifest-path Cargo.toml
pytest tests
```

`maturin develop` installs into the currently active virtualenv.

## Usage

```python
import oqci_native
from qiskit import QuantumCircuit
from qiskit.circuit import Parameter

qc = QuantumCircuit(2, 2, name="bell")
qc.h(0)
qc.cx(0, 1)
qc.measure([0, 1], [0, 1])
print(oqci_native.qiskit_to_qir(qc))

# Parameterized circuits must be bound before lowering.
theta = Parameter("theta")
ansatz = QuantumCircuit(1)
ansatz.ry(theta, 0)
print(oqci_native.qiskit_parameters(ansatz))          # ['theta']
print(oqci_native.qiskit_to_qir(ansatz, {"theta": 0.75}))

# OpenQASM 3 goes through the same IR and the same gate table.
print(oqci_native.qasm3_to_qir('qubit[1] q; h q[0];'))
```

Failures raise `oqci_native.OqciError`, a `ValueError` subclass.

## Scope

These bindings cover the frontend → QC-IR → QIR path only. The compiler API,
pass configuration, backend selection and analysis surfaces described in
`final-deliverables-spec.md` §17 arrive with the stages that implement them —
bindings are only added over contracts that already exist.
