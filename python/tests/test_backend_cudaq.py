"""The CUDA-Q execution adapter, tested without CUDA-Q.

These tests never import CUDA-Q, so they run wherever the rest of the suite
does. They pin down everything that can be checked without it:

- the translation — by compiling the generated kernel *back* through OQCI's
  own CUDA-Q frontend and requiring the same unitary as the executable it
  came from (checked by Qiskit's ``Operator``, not by OQCI);
- every refusal;
- the bit-order conversion, against Aer's counts for the same executable;
- ``run``'s plumbing (seed, target, shots, kernel loading), against a
  stand-in ``cudaq`` module.

What they cannot show is that real CUDA-Q returns what its documentation
says; ``docs/adapters.md`` records that as the open verification step.
"""

from __future__ import annotations

import json
import math
import sys
import types

import pytest

oqci = pytest.importorskip("oqci")
qiskit = pytest.importorskip("qiskit")

from qiskit.quantum_info import Operator  # noqa: E402

from oqci.backends import UnsupportedOperation  # noqa: E402
from oqci.backends import cudaq as backend  # noqa: E402


def executable(num_qubits, ops, num_clbits=0, **extra):
    """A hand-built executable, so each test controls every operation."""
    return {
        "backend_id": "simulator",
        "num_qubits": num_qubits,
        "num_clbits": num_clbits,
        "ops": [
            {"op": op, "qubits": list(qubits), "params": list(params), "clbit": clbit}
            for op, qubits, params, clbit in ops
        ],
        "settings": {"shots": 1000, "seed": None},
        "provenance": {"backend_id": "simulator"},
        **extra,
    }


def gate(op, *qubits, params=()):
    return (op, qubits, params, None)


def measure(qubit, clbit):
    return ("measure", (qubit,), (), clbit)


EVERY_GATE = [
    gate("h", 0),
    gate("x", 1),
    gate("y", 2),
    gate("z", 0),
    gate("s", 1),
    gate("sdg", 2),
    gate("t", 0),
    gate("tdg", 1),
    gate("sx", 2),
    gate("sxdg", 0),
    gate("rx", 1, params=(0.3,)),
    gate("ry", 2, params=(-1.1,)),
    gate("rz", 0, params=(2.5e-05,)),
    gate("p", 1, params=(math.pi / 3,)),
    gate("u", 2, params=(0.4, 1.2, -0.7)),
    gate("cx", 0, 1),
    gate("cy", 1, 2),
    gate("cz", 2, 0),
    gate("swap", 0, 2),
    gate("ccx", 0, 1, 2),
    gate("id", 1),
]


def unitary_of(exe):
    circuit = oqci.backends.aer.to_qiskit(exe)
    circuit.remove_final_measurements()
    return Operator(circuit)


def test_the_generated_kernel_compiles_back_to_the_same_unitary():
    original = executable(3, EVERY_GATE)
    source = backend.to_cudaq_source(original)

    # OQCI's own CUDA-Q frontend reads the kernel this adapter wrote. If any
    # template used syntax outside the documented subset, this would refuse.
    # `source_circuit` is the frontend's output before any pass or lowering,
    # so nothing downstream can mask a translation error.
    parsed = oqci.compile(source, frontend="cudaq")["source_circuit"]
    roundtrip = {
        "num_qubits": original["num_qubits"],
        "num_clbits": 0,
        "ops": [
            {
                "op": view["gate"],
                "qubits": view["qubits"],
                "params": [p["radians"] for p in view.get("params", [])],
            }
            for view in parsed
        ],
    }

    # Equal up to global phase: sx/sxdg become rotations, which differ from
    # them by exactly a global phase — invisible to sampling.
    assert unitary_of(original).equiv(unitary_of(roundtrip))


def test_every_template_uses_the_documented_spelling():
    source = backend.to_cudaq_source(executable(3, EVERY_GATE))
    for expected in (
        "s.adj(q[2])",
        "t.adj(q[1])",
        "r1(1.0471975511965976, q[1])",
        "u3(0.4, 1.2, -0.7, q[2])",
        "x.ctrl([q[0], q[1]], q[2])",
        "y.ctrl(q[1], q[2])",
        "rz(2.5e-05, q[0])",
    ):
        assert expected in source, source
    # The undocumented spellings the prototype emitted are gone.
    assert "sdg(" not in source and "tdg(" not in source


def test_measurements_come_last_in_qubit_order():
    exe = executable(
        3,
        [gate("h", 0), measure(2, 0), gate("cx", 0, 1), measure(0, 2), measure(1, 1)],
        num_clbits=3,
    )
    body = [line.strip() for line in backend.to_cudaq_source(exe).splitlines()]
    assert body[-3:] == ["mz(q[0])", "mz(q[1])", "mz(q[2])"]
    assert body.index("x.ctrl(q[0], q[1])") < body.index("mz(q[0])")


@pytest.mark.parametrize(
    "ops, fragment",
    [
        ([measure(0, 0), gate("x", 0)], "after it was measured"),
        ([measure(0, 0), measure(0, 1)], "after it was measured"),
        ([measure(0, 0), measure(1, 0)], "clbit 0 is written"),
        ([gate("reset", 0)], "no Python form of reset"),
        ([gate("frobnicate", 0)], "no CUDA-Q equivalent"),
        ([gate("rx", 0, params=(math.inf,))], "non-finite"),
        ([gate("x", 5)], "outside"),
    ],
)
def test_what_cannot_be_mapped_exactly_is_refused(ops, fragment):
    with pytest.raises(UnsupportedOperation, match=fragment):
        backend.to_cudaq_source(executable(2, ops, num_clbits=2))


def test_the_same_exception_type_as_aer():
    assert backend.UnsupportedOperation is oqci.backends.aer.UnsupportedOperation


def test_counts_are_rekeyed_to_clbit_zero_rightmost():
    # qubit 0 -> clbit 1, qubit 2 -> clbit 0, clbit 2 never written.
    measurements = [(0, 1), (2, 0)]
    # CUDA-Q: first char = qubit 0, second = qubit 2.
    counts = backend.counts_from_cudaq([("10", 7), ("01", 3)], measurements, 3)
    assert counts == {"010": 7, "001": 3}


def test_an_unexplained_bitstring_length_is_refused_not_guessed():
    with pytest.raises(RuntimeError, match="refusing to guess"):
        backend.counts_from_cudaq([("101", 1)], [(0, 0), (1, 1)], 2)


class FakeSampleResult:
    def __init__(self, counts):
        self._counts = counts

    def items(self):
        return list(self._counts.items())


def fake_cudaq(monkeypatch, cudaq_counts):
    calls = {}
    module = types.ModuleType("cudaq")
    module.kernel = lambda function: function
    module.set_target = lambda name, **options: calls.update(target=(name, options))
    module.set_random_seed = lambda seed: calls.update(seed=seed)

    def sample(kernel, shots_count):
        calls.update(kernel=kernel.__name__, shots=shots_count)
        return FakeSampleResult(cudaq_counts)

    module.sample = sample
    monkeypatch.setitem(sys.modules, "cudaq", module)
    return calls


def to_cudaq_order(aer_counts, measurements, num_clbits):
    """What CUDA-Q's documented order would report for Aer's counts."""
    result = {}
    for key, count in aer_counts.items():
        bits = "".join(key[num_clbits - 1 - clbit] for _, clbit in measurements)
        result[bits] = result.get(bits, 0) + count
    return result


def test_run_reproduces_aer_counts_through_the_documented_bit_order(monkeypatch):
    pytest.importorskip("qiskit_aer")
    # Asymmetric on purpose: qubit 0 is always 1, qubit 1 always 0, and the
    # clbits are crossed, so any bit-order mistake changes the key.
    exe = executable(
        3,
        [gate("x", 0), gate("h", 2), measure(0, 2), measure(1, 0), measure(2, 1)],
        num_clbits=3,
    )
    aer = oqci.backends.aer.run(exe, shots=2000, seed=7)
    _, measurements = backend._plan(exe)
    calls = fake_cudaq(monkeypatch, to_cudaq_order(aer.counts, measurements, 3))

    result = backend.run(exe, shots=2000, seed=7, target="qpp-cpu")

    assert result.counts == aer.counts
    assert set(result.counts) <= {"100", "110"}
    assert calls == {
        "target": ("qpp-cpu", {}),
        "seed": 7,
        "kernel": "oqci_kernel",
        "shots": 2000,
    }
    assert result.provenance["shots"] == 2000 and result.provenance["seed"] == 7
    assert isinstance(result, backend.CudaQResult)


def test_run_leaves_the_target_alone_unless_asked(monkeypatch):
    calls = fake_cudaq(monkeypatch, {"1": 5})
    result = backend.run(executable(1, [gate("x", 0), measure(0, 0)], num_clbits=1))
    assert "target" not in calls and "seed" not in calls
    assert calls["shots"] == 1000  # from the executable's settings
    assert result.counts == {"1": 5}


def test_run_refuses_an_executable_with_no_measurement(monkeypatch):
    fake_cudaq(monkeypatch, {})
    with pytest.raises(UnsupportedOperation, match="no measurement"):
        backend.run(executable(1, [gate("x", 0)]))


def test_a_compiled_program_runs_end_to_end(monkeypatch):
    exe = oqci.compile(
        "qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;",
        backend="simulator-nisq",
    )["executable"]
    fake_cudaq(monkeypatch, {"00": 500, "11": 500})
    assert backend.run(exe, seed=1).counts == {"00": 500, "11": 500}


def test_the_cli_emits_the_kernel(tmp_path, capsys):
    path = tmp_path / "exe.json"
    path.write_text(json.dumps(executable(1, [gate("h", 0), measure(0, 0)], num_clbits=1)))
    assert backend.main(["emit", str(path)]) == 0
    out = capsys.readouterr().out
    assert "@cudaq.kernel" in out and "h(q[0])" in out and "mz(q[0])" in out


def test_the_cli_reports_a_missing_cudaq_without_a_traceback(tmp_path, capsys, monkeypatch):
    monkeypatch.setitem(sys.modules, "cudaq", None)  # makes `import cudaq` fail
    path = tmp_path / "exe.json"
    path.write_text(json.dumps(executable(1, [gate("h", 0), measure(0, 0)], num_clbits=1)))
    assert backend.main(["run", str(path)]) == 1
    assert "CUDA-Q is not installed" in capsys.readouterr().err
