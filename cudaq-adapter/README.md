# OQCI CUDA-Q Frontend + Backend Adapters

This repository implements a small, modular CUDA-Q integration for the OQCI
project described in the supplied OQCI presentation.

## Architecture

```text
CUDA-Q Python source
        |
        v
+-----------------------+
| CUDA-Q Frontend       |
| AST parser / adapter  |
+-----------------------+
        |
        v
+-----------------------+
| OQCI Unified IR       |
| Circuit + Operations  |
+-----------------------+
        |
        v
  OQCI optimization
  / analysis passes
        |
        v
+-----------------------+
| CUDA-Q Backend        |
| IR -> CUDA-Q kernel   |
+-----------------------+
        |
        v
  CUDA-Q simulator/QPU
```

The supplied OQCI proposal describes frontend adapters, a common Unified IR,
a configurable pass manager, and backend connectors. This implementation
matches that boundary: the adapters are deliberately independent of the core
compiler so they can be plugged into a future OQCI pass manager.

## What is implemented

### Frontend
`oqci_cudaq.frontend.CudaQFrontend`

Converts a supported CUDA-Q Python kernel into the OQCI JSON/IR form.

Supported operations:

- h, x, y, z
- s, sdg, t, tdg
- cx/cnot
- cz
- swap
- rx, ry, rz, r1
- mz(q) and mz(q[i])

Supported angle expressions:

- numeric literals
- `math.pi`
- unary +/- 
- +, -, *, / combinations of the above

### Backend
`oqci_cudaq.backend.CudaQBackend`

Converts an OQCI `Circuit` into CUDA-Q Python source, compiles it into a
CUDA-Q kernel at runtime, selects a CUDA-Q target, and executes it with
`cudaq.sample()`.

Default target is `qpp-cpu`, which is intended to make the demo runnable
without an NVIDIA GPU. If your CUDA-Q installation exposes a different target,
pass that target name with `--target`.

## 1. Installation

CUDA-Q's current official quick-start recommends Python 3.11+ and:

```bash
python -m venv .venv
# Linux / WSL:
source .venv/bin/activate
# Windows PowerShell:
# .venv\Scripts\Activate.ps1

python -m pip install --upgrade pip
python -m pip install cudaq
python -m pip install -e ".[dev]"
```

CUDA-Q does not require a GPU for basic use. GPU acceleration is available
on Linux when the supported NVIDIA software stack is present.

## 2. Run the unit tests

```bash
pytest -q
```

The tests do not require CUDA-Q because they test the adapter translation
boundary.

## 3. CUDA-Q -> OQCI frontend

Create a CUDA-Q file such as:

```python
import cudaq

@cudaq.kernel
def bell():
    q = cudaq.qvector(2)
    h(q[0])
    x.ctrl(q[0], q[1])
    mz(q)
```

Then:

```bash
oqci-cudaq frontend bell_cudaq.py -o bell_ir.json
```

The resulting IR is approximately:

```json
{
  "num_qubits": 2,
  "operations": [
    {"name": "h", "qubits": [0], "params": []},
    {"name": "cx", "qubits": [0, 1], "params": []}
  ],
  "measurements": [],
  "metadata": {
    "source": "cuda-q-python",
    "adapter": "oqci-cudaq-frontend"
  }
}
```

## 4. OQCI -> CUDA-Q backend

Convert IR to executable CUDA-Q Python:

```bash
oqci-cudaq backend bell_ir.json -o generated_cudaq.py
```

Inspect the generated program:

```python
import cudaq

@cudaq.kernel
def oqci_kernel():
    q = cudaq.qvector(2)
    h(q[0])
    x.ctrl(q[0], q[1])
    mz(q)
```

Run it directly:

```bash
python generated_cudaq.py
```

or use the adapter runner:

```bash
oqci-cudaq run bell_ir.json --shots 1000
```

## 5. Programmatic integration into OQCI

Frontend:

```python
from oqci_cudaq import CudaQFrontend

frontend = CudaQFrontend()
ir = frontend.from_file("bell_cudaq.py")
```

Your OQCI pass manager can now modify `ir.operations`.

Backend:

```python
from oqci_cudaq import CudaQBackend

backend = CudaQBackend(target="qpp-cpu")
result = backend.run(ir, shots=1000)
print(result)
```

## 6. Where optimization belongs

Do not put OQCI optimizations inside the CUDA-Q adapter. Keep the boundary:

```text
Frontend Adapter
      |
      v
Unified IR
      |
      +--> analysis passes
      +--> gate cancellation
      +--> gate fusion
      +--> depth reduction
      +--> rotation simplification
      +--> qubit remapping / noise-aware pass
      |
      v
Backend Adapter
```

This follows the modular architecture proposed in the supplied OQCI
presentation.

## 7. Important scope note

This is a working reference adapter, not a full CUDA-Q language compiler.

It intentionally supports a controlled gate subset. CUDA-Q programs containing
arbitrary Python control flow, dynamic qubit allocation, custom operations,
templates, kernels calling other kernels, measurements with complex classical
logic, or advanced CUDA-Q features should be rejected or handled by additional
frontend passes.

For a student/research prototype, this is preferable to depending on private
CUDA-Q internals. You can expand the AST mapping as your OQCI IR grows.

## 8. Suggested next integration

If your OQCI core is in Rust, use this adapter as the Python execution boundary:

```text
Qiskit/Cirq/CUDA-Q/OpenQASM
          |
     Frontend adapters
          |
     OQCI Unified IR
          |
      Rust core
   Pass Manager + IR
          |
    Backend adapters
       /    |     \
   Qiskit  Cirq  CUDA-Q
```

The Rust core can own optimization and analysis, while the CUDA-Q Python
adapter remains a thin interoperability layer.
