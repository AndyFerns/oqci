from oqci_cudaq.backend import CudaQBackend
from oqci_cudaq.ir import Circuit, Operation


def test_backend_generates_bell():
    circuit = Circuit(
        num_qubits=2,
        operations=[
            Operation("h", [0]),
            Operation("cx", [0, 1]),
        ],
        measurements=[],
    )
    source = CudaQBackend().generate_source(circuit)
    assert "cudaq.qvector(2)" in source
    assert "h(q[0])" in source
    assert "x.ctrl(q[0], q[1])" in source
    assert "mz(q)" in source
