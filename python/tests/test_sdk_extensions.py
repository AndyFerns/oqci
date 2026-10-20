"""The SDK surface added for targets, verification, lint, timing and results."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

import oqci

FIXTURE = Path(__file__).resolve().parents[2] / "tests" / "fixtures" / "heron_style_synthetic.json"

BELL = 'OPENQASM 3.0; include "stdgates.inc"; qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;'


def profile():
    return json.loads(FIXTURE.read_text())


def test_a_target_dict_or_json_compiles_like_a_backend():
    for target in (profile(), FIXTURE.read_text()):
        artifacts = oqci.compile(BELL, target=target)
        assert artifacts["backend"] == "heron-style-synthetic"
        ops = {op["op"] for op in artifacts["executable"]["ops"]}
        assert ops <= {"cz", "rz", "sx", "x", "measure"}
        assert artifacts["extensions"] == ["target-file:python backend:heron-style-synthetic"]


def test_backend_and_target_together_are_refused():
    with pytest.raises(oqci.OqciError, match="not both"):
        oqci.compile(BELL, backend="simulator", target=profile())


def test_a_broken_target_is_refused_before_compiling():
    broken = profile()
    broken["decomposition_rules"].remove("cx-to-cz")
    with pytest.raises(oqci.OqciError, match="do not reach the basis"):
        oqci.load_target(broken)
    with pytest.raises(oqci.OqciError, match="do not reach the basis"):
        oqci.compile(BELL, target=broken)


def test_verification_reports_every_transformation():
    artifacts = oqci.compile(BELL, backend="simulator-nisq", verify=True)
    report = artifacts["verification"]
    assert report["not_equivalent"] == 0
    assert report["inconclusive"] == 0
    transformations = [check["transformation"] for check in report["checks"]]
    assert transformations[-1] == "lowering"
    assert any(t.startswith("pass:single-qubit-fusion#") for t in transformations)
    assert report["not_checked"] == ["qir-emission"]
    assert artifacts["timings"]["verification_us"] is not None


def test_verification_is_inconclusive_not_passed_when_too_wide():
    artifacts = oqci.compile(BELL, backend="simulator", verify=True, verify_max_qubits=1)
    report = artifacts["verification"]
    assert report["inconclusive"] > 0
    reasons = [c["verdict"].get("reason", {}).get("kind") for c in report["checks"]]
    assert "too_many_qubits" in reasons


def test_lint_runs_with_and_without_a_target():
    source = "qubit[2] q; bit[1] c; h q[0]; h q[0]; c[0] = measure q[0];"
    plain = oqci.lint(source)
    assert any(d["rule"] == "cancellable-pair" for d in plain["diagnostics"])
    assert not any(d["rule"] == "target-unsupported" for d in plain["diagnostics"])

    targeted = oqci.lint(source, backend="simulator-nisq")
    assert any(d["rule"] == "target-unsupported" for d in targeted["diagnostics"])

    optimized = oqci.lint(source, optimized=True)
    assert not any(d["rule"] == "cancellable-pair" for d in optimized["diagnostics"])


def test_compile_can_lint_too():
    artifacts = oqci.compile("qubit[1] q; h q[0]; h q[0];", lint=True)
    assert artifacts["lint"]["rules"]
    assert oqci.compile("qubit[1] q;")["lint"] is None


def test_timings_are_reported_per_stage():
    artifacts = oqci.compile(BELL, backend="simulator-nisq")
    t = artifacts["timings"]
    for key in ("frontend_us", "ir_construction_us", "optimization_us", "lowering_us", "total_us"):
        assert t[key] is not None, key
    assert t["total_us"] >= t["optimization_us"]


def test_qiskit_circuits_have_no_frontend_time():
    qiskit = pytest.importorskip("qiskit")
    circuit = qiskit.QuantumCircuit(1)
    circuit.h(0)
    t = oqci.compile(circuit)["timings"]
    assert t["frontend_us"] is None and t["ir_construction_us"] is None


def test_an_aer_result_converts_into_an_execution_result():
    pytest.importorskip("qiskit_aer")
    from oqci.backends import aer

    artifacts = oqci.compile(BELL, backend="simulator-nisq", shots=300, seed=11)
    result = aer.run(artifacts["executable"])
    converted = oqci.backends.to_execution_result(result, artifacts)
    assert sum(converted["counts"].values()) == 300
    assert converted["submission_duration_ms"] is None, "a local simulator has no queue"
    assert converted["compilation_duration_ms"] == artifacts["timings"]["total_us"] / 1000.0

    other = oqci.compile(BELL, backend="simulator")
    with pytest.raises(oqci.OqciError, match="does not match"):
        oqci.backends.to_execution_result(result, other)


def test_basis_rule_sets_are_complete_and_distinct():
    sets = oqci.basis_rule_sets()
    assert "cx-to-cz" in sets["cz"] and "cz-to-cx" not in sets["cz"]
    assert "cz-to-cx" in sets["cx"] and "cx-to-cz" not in sets["cx"]
