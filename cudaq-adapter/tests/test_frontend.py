import math
from oqci_cudaq.frontend import CudaQFrontend


def test_bell_frontend():
    source = """
import cudaq

@cudaq.kernel
def bell():
    q = cudaq.qvector(2)
    h(q[0])
    x.ctrl(q[0], q[1])
    mz(q)
"""
    circuit = CudaQFrontend().from_source(source)
    assert circuit.num_qubits == 2
    assert [op.name for op in circuit.operations] == ["h", "cx"]
    assert circuit.operations[1].qubits == [0, 1]


def test_rotation():
    source = """
import cudaq, math

@cudaq.kernel
def k():
    q = cudaq.qvector(1)
    rz(math.pi / 2, q[0])
    mz(q)
"""
    circuit = CudaQFrontend().from_source(source)
    assert circuit.operations[0].name == "rz"
    assert abs(circuit.operations[0].params[0] - math.pi / 2) < 1e-9
