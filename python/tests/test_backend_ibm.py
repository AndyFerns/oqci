"""The IBM execution adapter, tested offline.

Everything here runs through IBM Runtime's local testing mode: a fake backend
from ``qiskit_ibm_runtime.fake_provider`` is a snapshot of a real device's
target and noise, and ``SamplerV2(mode=fake)`` executes on Aer. No account,
token or network is used. ``FakeFez`` is a Heron-family (CZ-native) device;
``FakeBrisbane`` an Eagle (ECR-native) one, which the adapter must refuse.

What these tests establish:

- the device's target becomes an OQCI profile that OQCI accepts and lowers
  onto, with every operation of the executable legal on the real coupling
  map — checked independently by Qiskit's own ``Target``;
- nothing is transpiled: the submitted circuit is the executable, and an
  executable that is not ISA-valid is refused before submission;
- results come back in the shared bit convention, agree with the ideal
  distribution up to the device's noise, carry provenance, and convert into
  OQCI's ``ExecutionResult``.
"""

from __future__ import annotations

import json

import pytest

pytest.importorskip("qiskit_ibm_runtime")
pytest.importorskip("qiskit_aer")

import oqci  # noqa: E402
from oqci.backends import ibm  # noqa: E402
from qiskit_ibm_runtime.fake_provider import FakeBrisbane, FakeFez  # noqa: E402

GHZ = """
OPENQASM 3.0;
include "stdgates.inc";
qubit[3] q;
bit[3] c;
h q[0];
cx q[0], q[1];
cx q[1], q[2];
c = measure q;
"""

#: A circuit whose two-qubit gates span distant logical qubits, so routing
#: has to work on the device's heavy-hex coupling map.
FAR = """
OPENQASM 3.0;
include "stdgates.inc";
qubit[5] q;
bit[5] c;
x q[0];
cx q[0], q[4];
cx q[4], q[2];
c = measure q;
"""


@pytest.fixture(scope="module")
def fez():
    return FakeFez()


@pytest.fixture(scope="module")
def profile(fez):
    return ibm.target_from_backend(fez)


def test_the_device_target_becomes_an_accepted_profile(fez, profile):
    assert profile["backend_id"] == "ibm"
    assert profile["topology"]["qubit_count"] == fez.num_qubits
    assert set(profile["supported_operations"]) == {"cz", "rz", "sx", "x", "measure", "reset"}
    assert profile["decomposition_rules"] == oqci.basis_rule_sets()["cz"]
    assert f"device:{fez.name}" in profile["capabilities"]
    # Every coupling is one the device itself declares.
    cz = set(fez.target.qargs_for_operation_name("cz"))
    assert all(tuple(edge) in cz for edge in profile["topology"]["edges"])
    loaded = oqci.load_target(profile)
    assert loaded["qualified_id"] == f"{fez.name}@{fez.backend_version}"


def test_an_ecr_device_is_refused_by_name():
    with pytest.raises(oqci.backends.UnsupportedOperation, match="not CZ-native"):
        ibm.target_from_backend(FakeBrisbane())


@pytest.mark.parametrize("source", [GHZ, FAR])
def test_a_compiled_executable_is_isa_valid_for_the_real_device(fez, profile, source):
    artifacts = oqci.compile(source, target=profile, layout="dense", verify=True)
    assert artifacts["verification"]["not_equivalent"] == 0
    circuit = ibm.to_isa_circuit(artifacts["executable"], fez)
    # Qiskit's own Target, not OQCI, is the judge here.
    for instruction in circuit.data:
        qargs = tuple(circuit.find_bit(q).index for q in instruction.qubits)
        assert fez.target.instruction_supported(instruction.operation.name, qargs), (
            instruction.operation.name,
            qargs,
        )
    ibm.check_isa(circuit, fez)


def test_a_non_isa_executable_is_refused_not_transpiled(fez):
    # Compiled for the all-to-all simulator: `h` and a long-range CX are not
    # in the device's ISA. Submission must refuse, not quietly fix it.
    artifacts = oqci.compile(FAR, backend="simulator")
    with pytest.raises(ibm.IsaViolation, match="not in `fake_fez`'s instruction set"):
        ibm.submit(artifacts["executable"], fez)


def test_a_run_on_the_fake_device_agrees_with_the_ideal_distribution(fez, profile):
    artifacts = oqci.compile(GHZ, target=profile, layout="dense", shots=4000)
    result = ibm.run(artifacts["executable"], backend=fez)

    assert result.observed_shots == 4000
    assert all(len(key) == 3 and set(key) <= {"0", "1"} for key in result.counts)
    # Ideal GHZ is half 000, half 111. The fake device is noisy, so allow a
    # bounded error mass rather than demand perfection — but the two ideal
    # outcomes must dominate, and be balanced.
    good = result.counts.get("000", 0) + result.counts.get("111", 0)
    assert good / 4000 > 0.85
    assert abs(result.counts.get("000", 0) - result.counts.get("111", 0)) / 4000 < 0.1

    assert result.provenance["backend_id"] == "fake_fez"
    assert result.provenance["shots"] == 4000
    assert result.provenance["seed"] is None
    assert result.backend_metadata["backend_name"] == "fake_fez"
    assert result.backend_metadata["job_id"]
    json.dumps(result.backend_metadata)  # serializable as recorded


def test_x_lands_where_the_final_layout_says(fez, profile):
    # A deterministic circuit: the counts must be exactly the classical
    # answer modulo device noise — and the bit order must be the shared one.
    source = (
        'OPENQASM 3.0; include "stdgates.inc"; qubit[3] q; bit[3] c; '
        "x q[0]; cx q[0], q[2]; c = measure q;"
    )
    artifacts = oqci.compile(source, target=profile, layout="dense", shots=2000)
    result = ibm.run(artifacts["executable"], backend=fez)
    top = max(result.counts, key=result.counts.get)
    assert top == "101", result.counts  # clbit 0 rightmost
    assert result.counts[top] / 2000 > 0.85


def test_hardware_cannot_be_seeded(fez, profile):
    artifacts = oqci.compile(GHZ, target=profile)
    with pytest.raises(oqci.backends.UnsupportedOperation, match="cannot be seeded"):
        ibm.run(artifacts["executable"], backend=fez, seed=7)


def test_a_submission_record_is_serializable_and_carries_no_secret(fez, profile):
    artifacts = oqci.compile(GHZ, target=profile, layout="dense")
    job, submission = ibm.submit(artifacts["executable"], fez, shots=100)
    record = submission.to_dict()
    text = json.dumps(record)
    assert record["job_id"] == job.job_id()
    assert "token" not in text.lower()
    # Nothing may be inserted around OQCI's circuit on the device.
    assert record["options"] == {
        "twirling.enable_gates": False,
        "twirling.enable_measure": False,
        "dynamical_decoupling.enable": False,
    }
    result = ibm.collect(job, submission)
    assert result.observed_shots == 100


def test_an_ibm_result_converts_into_an_execution_result(fez, profile):
    artifacts = oqci.compile(GHZ, target=profile, layout="dense", shots=500)
    result = ibm.run(artifacts["executable"], backend=fez)
    converted = oqci.backends.to_execution_result(result, artifacts)
    assert sum(converted["counts"].values()) == 500
    assert converted["compilation_duration_ms"] is not None
    assert converted["provenance"]["profile_id"] == f"fake_fez@{fez.backend_version}"
    # Local testing mode reports zero-length timestamps; they are carried as
    # reported, separately, never derived from one another.
    assert converted["submission_duration_ms"] is not None
    assert converted["execution_duration_ms"] is not None
