"""Tests for the PyO3 boundary, driven through real Qiskit circuits.

The translation *decisions* are tested in Rust (`src/frontend/qiskit/mod.rs`
and `tests/qiskit_adapter.rs`), with no Python involved. What these tests
cover is the part that can only be checked against a live Qiskit: that a real
``QuantumCircuit`` is read correctly — instruction order, flat bit indices,
bound floats vs. unbound ``Parameter``s vs. compound expressions.

Run with::

    pip install -r python/requirements-dev.txt
    maturin develop --manifest-path python/Cargo.toml
    pytest python/tests
"""

import pytest

qiskit = pytest.importorskip("qiskit")
oqci_native = pytest.importorskip("oqci_native")

from qiskit import QuantumCircuit  # noqa: E402
from qiskit.circuit import Parameter  # noqa: E402


def body(qir):
    """The emitted call sequence, without declarations or the module header.

    Declarations are emitted alphabetically at the top of the module, so
    matching on a bare intrinsic name would find the `declare` line rather
    than the `call`. Comparing bodies also ignores the circuit name, which
    Qiskit rewrites in `assign_parameters`.
    """
    return [line.strip() for line in qir.splitlines() if line.startswith("  call ")]


def test_bell_circuit_compiles_to_qir():
    qc = QuantumCircuit(2, 2, name="bell")
    qc.h(0)
    qc.cx(0, 1)
    qc.measure([0, 1], [0, 1])

    qir = oqci_native.qiskit_to_qir(qc)

    assert "@__quantum__qis__h__body" in qir
    assert "@__quantum__qis__cnot__body" in qir
    assert qir.count("call void @__quantum__qis__mz__body") == 2
    assert 'required_num_qubits"="2"' in qir


def test_empty_circuit_compiles():
    qir = oqci_native.qiskit_to_qir(QuantumCircuit(name="empty"))
    assert "ret void" in qir


def test_ghz3_preserves_instruction_order():
    qc = QuantumCircuit(3, name="ghz")
    qc.h(0)
    qc.cx(0, 1)
    qc.cx(1, 2)

    calls = body(oqci_native.qiskit_to_qir(qc))
    assert len(calls) == 3
    assert "h__body" in calls[0]
    assert all("cnot__body" in call for call in calls[1:])


def test_mid_circuit_measurement_is_emitted_in_order():
    qc = QuantumCircuit(2, 1)
    qc.h(0)
    qc.measure(0, 0)
    qc.x(1)

    calls = body(oqci_native.qiskit_to_qir(qc))
    assert "h__body" in calls[0]
    assert "mz__body" in calls[1]
    assert "x__body" in calls[2]


def test_multiple_registers_map_to_flat_indices():
    from qiskit import ClassicalRegister, QuantumRegister

    a, b = QuantumRegister(1, "a"), QuantumRegister(1, "b")
    c = ClassicalRegister(2, "c")
    qc = QuantumCircuit(a, b, c)
    qc.cx(a[0], b[0])
    qc.measure([a[0], b[0]], [c[0], c[1]])

    qir = oqci_native.qiskit_to_qir(qc)
    # `b[0]` is the second qubit overall, so the CNOT targets index 1.
    assert "inttoptr (i64 1 to %Qubit*)" in qir


def test_reset_is_translated():
    qc = QuantumCircuit(1)
    qc.reset(0)
    assert "@__quantum__qis__reset__body" in oqci_native.qiskit_to_qir(qc)


def test_barrier_is_dropped():
    with_barrier = QuantumCircuit(2, name="c")
    with_barrier.h(0)
    with_barrier.barrier()
    with_barrier.cx(0, 1)

    without = QuantumCircuit(2, name="c")
    without.h(0)
    without.cx(0, 1)

    assert oqci_native.qiskit_to_qir(with_barrier) == oqci_native.qiskit_to_qir(without)


def test_bound_rotation_matches_a_literal_angle():
    import math

    qc = QuantumCircuit(1, name="c")
    qc.rz(math.pi / 2, 0)

    qir = oqci_native.qiskit_to_qir(qc)
    assert "@__quantum__qis__rz__body" in qir


def test_unknown_gate_becomes_an_extended_intrinsic():
    qc = QuantumCircuit(2)
    qc.iswap(0, 1)
    assert "@__quantum__qis__iswap__body" in oqci_native.qiskit_to_qir(qc)


def test_unbound_parameter_is_reported():
    theta = Parameter("theta")
    qc = QuantumCircuit(1)
    qc.rz(theta, 0)

    assert oqci_native.qiskit_parameters(qc) == ["theta"]
    with pytest.raises(oqci_native.OqciError, match="theta"):
        oqci_native.qiskit_to_qir(qc)


def test_parameter_can_be_bound_by_name_or_object():
    theta = Parameter("theta")
    qc = QuantumCircuit(1, name="c")
    qc.ry(theta, 0)

    by_name = oqci_native.qiskit_to_qir(qc, {"theta": 0.75})
    by_object = oqci_native.qiskit_to_qir(qc, {theta: 0.75})
    assert by_name == by_object

    # Binding through OQCI must agree with binding through Qiskit. Compare
    # bodies: `assign_parameters` gives the circuit a fresh name.
    via_qiskit = oqci_native.qiskit_to_qir(qc.assign_parameters({theta: 0.75}))
    assert body(by_name) == body(via_qiskit)


def test_compound_parameter_expression_is_refused():
    theta = Parameter("theta")
    qc = QuantumCircuit(1)
    qc.rx(2 * theta, 0)

    with pytest.raises(oqci_native.OqciError, match="compound parameter expression"):
        oqci_native.qiskit_to_qir(qc)


def test_out_of_subset_control_flow_is_refused():
    qc = QuantumCircuit(1, 1)
    with qc.if_test((qc.clbits[0], 1)):
        qc.x(0)

    with pytest.raises(oqci_native.OqciError, match="static circuits"):
        oqci_native.qiskit_to_qir(qc)


def test_openqasm_path_agrees_with_the_qiskit_path():
    qc = QuantumCircuit(2, 2, name="main")
    qc.h(0)
    qc.cx(0, 1)
    qc.measure([0, 1], [0, 1])

    via_qiskit = oqci_native.qiskit_to_qir(qc)
    via_qasm = oqci_native.qasm3_to_qir(
        """
        OPENQASM 3.0;
        include "stdgates.inc";
        qubit[2] q;
        bit[2] c;
        h q[0];
        cx q[0], q[1];
        c = measure q;
        """
    )
    assert via_qiskit == via_qasm


def test_qasm_syntax_error_is_reported_with_a_position():
    with pytest.raises(oqci_native.OqciError, match="line"):
        oqci_native.qasm3_to_qir("qubit[2] q;\nh q[0]\n")
