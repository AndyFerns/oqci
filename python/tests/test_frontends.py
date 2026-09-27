"""Every frontend reaches the whole pipeline through ``oqci.compile``.

Until this file's features existed, a Qiskit ``QuantumCircuit`` could reach
QIR but never a backend (``compile`` only accepted OpenQASM text), the SDK
could not run an ablation, and its dict carried neither the circuits nor the
same pass and lowering schema as the CLI. These tests pin each of those
down, and check the new paths against Qiskit Aer actually running the result
rather than against OQCI's own view of it.

CUDA-Q is exercised as *source text*: the frontend reads a kernel's Python
source without importing ``cudaq``, so these tests need no CUDA-Q install.
"""

from __future__ import annotations

import pytest

oqci = pytest.importorskip("oqci")
qiskit = pytest.importorskip("qiskit")

from qiskit import QuantumCircuit  # noqa: E402
from qiskit.circuit import Parameter  # noqa: E402

SHOTS = 4000
SEED = 20260927
TOLERANCE = 0.05

BELL_QASM = "qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;"

GHZ_CUDAQ = '''
import cudaq

@cudaq.kernel
def ghz():
    q = cudaq.qvector(3)
    h(q[0])
    x.ctrl(q[0], q[1])
    x.ctrl(q[1], q[2])
    mz(q[0])
    mz(q[1])
    mz(q[2])
'''

BELL_CUDAQ = '''
import cudaq

@cudaq.kernel
def bell():
    q = cudaq.qvector(2)
    h(q[0])
    x.ctrl(q[0], q[1])
    mz(q)
'''


def qiskit_bell() -> QuantumCircuit:
    circuit = QuantumCircuit(2, 2, name="bell")
    circuit.h(0)
    circuit.cx(0, 1)
    circuit.measure([0, 1], [0, 1])
    return circuit


def run_on_aer(artifacts):
    aer = pytest.importorskip("qiskit_aer")  # noqa: F841 — skip if absent
    result = oqci.backends.aer.run(artifacts["executable"], shots=SHOTS, seed=SEED)
    return result.probabilities()


def assert_only(observed, expected):
    for bits, probability in observed.items():
        if bits not in expected:
            assert probability < TOLERANCE, f"unexpected outcome {bits}: {observed}"
    for bits, probability in expected.items():
        assert abs(observed.get(bits, 0.0) - probability) < TOLERANCE, observed


# --- Qiskit through the whole pipeline --------------------------------------


def test_a_qiskit_circuit_reaches_an_executable():
    artifacts = oqci.compile(qiskit_bell(), backend="simulator-nisq")
    assert artifacts["frontend"] == "qiskit"
    assert artifacts["lowering"]["legal"]
    assert "executable" in artifacts
    assert artifacts["executable"]["provenance"]["circuit"] == "bell"


def test_a_qiskit_circuit_compiles_to_the_same_executable_as_its_openqasm_twin():
    from_qiskit = oqci.compile(qiskit_bell(), backend="simulator-nisq", name="main")
    from_qasm = oqci.compile(BELL_QASM, backend="simulator-nisq")
    assert from_qiskit["executable"] == from_qasm["executable"]


def test_a_qiskit_circuit_compiled_by_oqci_runs_correctly_on_aer():
    artifacts = oqci.compile(qiskit_bell(), backend="simulator-nisq", seed=SEED)
    assert_only(run_on_aer(artifacts), {"00": 0.5, "11": 0.5})


def test_qiskit_parameters_bind_by_object_through_compile():
    theta = Parameter("theta")
    circuit = QuantumCircuit(1, 1)
    circuit.ry(theta, 0)
    circuit.measure(0, 0)

    with pytest.raises(oqci.OqciError, match="theta"):
        oqci.compile(circuit, backend="simulator")

    artifacts = oqci.compile(circuit, backend="simulator", bindings={theta: 0.5})
    assert "executable" in artifacts


def test_a_text_frontend_cannot_be_forced_onto_a_quantum_circuit():
    with pytest.raises(oqci.OqciError, match="source text"):
        oqci.compile(qiskit_bell(), frontend="cudaq")


# --- CUDA-Q as source text --------------------------------------------------


def test_a_cudaq_kernel_compiles_through_the_sdk():
    artifacts = oqci.compile(GHZ_CUDAQ, frontend="cudaq", backend="simulator-nisq")
    assert artifacts["frontend"] == "cudaq"
    measures = [op for op in artifacts["source_circuit"] if op["op"] == "measure"]
    assert [(m["qubits"], m["clbit"]) for m in measures] == [([0], 0), ([1], 1), ([2], 2)]


def test_a_cudaq_kernel_compiled_by_oqci_runs_correctly_on_aer():
    artifacts = oqci.compile(GHZ_CUDAQ, frontend="cudaq", backend="simulator-nisq", seed=SEED)
    assert_only(run_on_aer(artifacts), {"000": 0.5, "111": 0.5})


def test_cudaq_and_openqasm_emit_identical_qir():
    assert oqci.cudaq_to_qir(BELL_CUDAQ) == oqci.qasm3_to_qir(BELL_QASM)


def test_an_unknown_frontend_is_refused_with_the_alternatives():
    with pytest.raises(oqci.OqciError, match="openqasm3, cudaq"):
        oqci.compile(BELL_QASM, frontend="cirq")


def test_a_cudaq_refusal_names_the_construct():
    loop = "import cudaq\n@cudaq.kernel\ndef k():\n    q = cudaq.qvector(2)\n    for i in range(2):\n        h(q[i])\n"
    with pytest.raises(oqci.OqciError, match="for"):
        oqci.compile(loop, frontend="cudaq")


# --- Ablation from Python ---------------------------------------------------


CANCELLABLE = "qubit[1] q; bit[1] c; h q[0]; h q[0]; c = measure q;"


def test_disabling_a_pass_changes_the_result():
    full = oqci.compile(CANCELLABLE)
    ablated = oqci.compile(CANCELLABLE, disable=["gate-cancellation"])
    assert full["optimized_metrics"]["op_count"] == 1
    assert ablated["optimized_metrics"]["op_count"] == 3
    skipped = [p for p in ablated["passes"] if p["id"] == "gate-cancellation"]
    assert skipped and not skipped[0]["enabled"]


def test_running_only_some_passes_works_for_qiskit_too():
    circuit = QuantumCircuit(1)
    circuit.h(0)
    circuit.h(0)
    artifacts = oqci.compile(circuit, passes=["schedule"])
    assert artifacts["optimized_metrics"]["op_count"] == 2


def test_unknown_or_combined_pass_selections_are_refused():
    with pytest.raises(oqci.OqciError, match="unknown pass"):
        oqci.compile(CANCELLABLE, disable=["no-such-pass"])
    with pytest.raises(oqci.OqciError, match="cannot be combined"):
        oqci.compile(CANCELLABLE, passes=["schedule"], disable=["canonicalize"])


# --- One schema with the CLI ------------------------------------------------


def test_pass_and_lowering_entries_use_the_cli_schema():
    artifacts = oqci.compile(BELL_QASM, backend="simulator-nisq")

    record = artifacts["passes"][0]
    assert {"id", "description", "enabled", "changed", "before", "after", "duration_us", "notes"} <= set(record)

    lowering = artifacts["lowering"]
    assert [step["id"] for step in lowering["steps"]][-1] == "verify"
    assert lowering["violations"] == []
    assert lowering["backend"] == "simulator-nisq"


def test_the_circuits_themselves_are_returned():
    artifacts = oqci.compile(CANCELLABLE, backend="simulator")
    assert [op["text"] for op in artifacts["source_circuit"]][:2] == ["h %q0", "h %q0"]
    assert len(artifacts["optimized_circuit"]) == 1
    assert artifacts["lowered_circuit"][-1]["op"] == "measure"
    assert artifacts["unbound_parameters"] == []
