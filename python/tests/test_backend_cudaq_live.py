"""The CUDA-Q execution adapter against a real CUDA-Q install.

``test_backend_cudaq.py`` tests the translation with a stand-in module; this
file runs OQCI executables on CUDA-Q itself and checks the counts against the
*exact* distribution of the source circuit (chi-square goodness of fit,
alpha = 0.001, as Stage G locks) and against Aer on the same executable.
It is skipped where CUDA-Q is not installed (it has no Windows wheels); the
``cudaq`` CI job runs it on Linux with a pinned CUDA-Q.
"""

from __future__ import annotations

import os

import pytest

# Aer and CUDA-Q each bundle a libgomp, and glibc's default static TLS
# reserve fits only one: on Linux this process needs
# GLIBC_TUNABLES=glibc.rtld.optional_static_tls=16384, as the `cudaq` CI
# job sets (docs/adapters.md, "Aer and CUDA-Q in one Linux process").
pytest.importorskip("qiskit_aer")

# Skipped where CUDA-Q is absent — unless OQCI_REQUIRE_CUDAQ=1 (the `cudaq` CI
# job), where a missing or broken install is a failure, with its error, not a
# silent skip that would look like verification.
try:
    import cudaq
except Exception as error:  # noqa: BLE001 — a broken install may raise anything
    if os.environ.get("OQCI_REQUIRE_CUDAQ") == "1":
        raise RuntimeError(f"CUDA-Q is required here but failed to import: {error!r}") from error
    pytest.skip(f"CUDA-Q is not available: {error!r}", allow_module_level=True)
stats = pytest.importorskip("scipy.stats")

import oqci  # noqa: E402
from oqci.backends import aer  # noqa: E402
from oqci.backends import cudaq as cudaq_adapter  # noqa: E402
from qiskit import QuantumCircuit  # noqa: E402
from qiskit.quantum_info import Statevector  # noqa: E402

SHOTS = 8192
ALPHA = 0.001


def circuits():
    bell = QuantumCircuit(2, 2, name="bell")
    bell.h(0)
    bell.cx(0, 1)
    ghz = QuantumCircuit(4, 4, name="ghz4")
    ghz.h(0)
    for q in range(3):
        ghz.cx(q, q + 1)
    rotated = QuantumCircuit(3, 3, name="rotated")
    rotated.ry(0.7, 0)
    rotated.cx(0, 2)
    rotated.rx(1.1, 1)
    rotated.cz(1, 2)
    rotated.h(2)
    rotated.t(1)
    rotated.swap(0, 1)
    out = []
    for qc in (bell, ghz, rotated):
        exact = Statevector(qc).probabilities_dict()
        qc.measure(range(qc.num_qubits), range(qc.num_qubits))
        out.append((qc, {k: float(v) for k, v in exact.items() if v > 1e-12}))
    return out


def chi_square_p(counts, exact):
    keys = sorted(set(exact) | set(counts))
    if any(counts.get(k, 0) and exact.get(k, 0) == 0 for k in keys):
        return 0.0
    kept = [k for k in keys if exact.get(k, 0) * SHOTS >= 5]
    observed = [counts.get(k, 0) for k in kept]
    expected = [exact[k] * SHOTS for k in kept]
    rest_obs = SHOTS - sum(observed)
    rest_exp = SHOTS - sum(expected)
    if rest_exp >= 5:
        observed.append(rest_obs)
        expected.append(rest_exp)
    scale = sum(observed) / sum(expected)
    return float(stats.chisquare(observed, [e * scale for e in expected]).pvalue)


@pytest.mark.parametrize("backend", ["simulator", "simulator-nisq"])
@pytest.mark.parametrize("case", range(3))
def test_cudaq_agrees_with_the_exact_distribution_and_with_aer(backend, case):
    circuit, exact = circuits()[case]
    if circuit.num_qubits > 5 and backend == "simulator-nisq":
        pytest.skip("wider than simulator-nisq")
    executable = oqci.compile(circuit, backend=backend, shots=SHOTS, seed=17)["executable"]
    on_cudaq = cudaq_adapter.run(executable).counts
    on_aer = aer.run(executable).counts
    assert sum(on_cudaq.values()) == SHOTS
    # One key convention: clbit 0 rightmost, one character per clbit.
    assert all(len(k) == circuit.num_clbits for k in on_cudaq)
    assert chi_square_p(on_cudaq, exact) >= ALPHA, (on_cudaq, exact)
    assert chi_square_p(on_aer, exact) >= ALPHA, (on_aer, exact)
