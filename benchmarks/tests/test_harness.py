"""The benchmark harness, checked: generators, correctness checking,
statistics, schema, coverage, and a miniature end-to-end run.

The harness judges compilers, so it must be able to tell right from wrong:
several tests below hand it deliberately broken compiled outputs and require
a ``fail``.
"""

from __future__ import annotations

import copy
import json
import math
from pathlib import Path

import pytest

from oqci_bench import circuits, config, coverage, equivalence, metrics, schema, seeds, stats, targets
from oqci_bench.compilers import Compiled, registry
from oqci_bench.determinism import strip_measurements

pytest.importorskip("oqci")

CONFIG = config.load("configs/smoke.toml")
REGISTERED = {"h", "x", "y", "z", "s", "sdg", "t", "tdg", "sx", "rx", "ry", "rz", "p", "cx", "cy",
              "cz", "swap", "ccx", "measure"}


def check(source, compiled):
    return equivalence.check(source, compiled, operator_max_qubits=10, state_max_qubits=20,
                             tolerance=1e-6)


def test_seeds_are_stable_and_distinct():
    assert seeds.item_seed(20261020, "random", 4, 0, 0) == seeds.item_seed(20261020, "random", 4, 0, 0)
    assert seeds.item_seed(20261020, "random", 4, 0, 0) != seeds.item_seed(20261020, "random", 4, 0, 1)
    # Pinned: the policy must not drift silently between versions.
    import hashlib

    expected = int.from_bytes(hashlib.sha256(b"20261020|random|4|0|0").digest()[:8], "big")
    assert seeds.item_seed(20261020, "random", 4, 0, 0) == expected


def test_the_corpus_is_reproducible_and_uses_only_registered_gates():
    first = circuits.corpus(CONFIG)
    second = circuits.corpus(CONFIG)
    assert [i.circuit_id for i in first] == [i.circuit_id for i in second]
    for a, b in zip(first, second):
        assert a.circuit == b.circuit
        assert {inst.operation.name for inst in a.circuit.data} <= REGISTERED


def test_the_stage_g_corpus_has_the_locked_size():
    full = circuits.corpus(config.load("configs/stage_g.toml"))
    assert len(full) == 64
    assert len({i.circuit_id for i in full}) == 64


def test_generated_circuits_compute_what_they_claim():
    # QFT then inverse QFT is the identity on a basis state.
    exact = stats.exact_distribution(circuits.qft_roundtrip(4, [0, 2]))
    assert exact == pytest.approx({"0101": 1.0})
    # Grover on 3 qubits finds 111 with high probability.
    grover = stats.exact_distribution(circuits.grover(3))
    assert grover.get("111", 0) > 0.9
    # Grover with ancillas returns them clean: they always read 0.
    g5 = stats.exact_distribution(circuits.grover(5))
    assert all(key[:2] == "00" for key in g5)  # the two ancillas are the leftmost bits
    assert g5.get("0011111", 0) > 0.9
    ghz = stats.exact_distribution(circuits.ghz(3))
    assert ghz == pytest.approx({"000": 0.5, "111": 0.5})


@pytest.fixture(scope="module")
def item():
    return circuits.find(circuits.corpus(CONFIG), "qft_4")


@pytest.mark.parametrize("compiler, entry", [
    ("oqci", {"id": "oqci-default", "layout": "dense"}),
    ("qiskit", {"id": "qiskit-o3", "optimization_level": 3}),
    ("tket", {"id": "tket"}),
])
@pytest.mark.parametrize("kind", ["all-to-all", "linear"])
def test_every_compiler_output_passes_the_correctness_check(item, compiler, entry, kind):
    target = targets.for_item(kind, item.width, CONFIG)
    compiled = registry()[compiler](item, target, entry)
    verdict = check(item.circuit, compiled)
    assert verdict.status == "pass", verdict
    assert set(metrics.metrics(compiled.circuit, target.basis)["gate_counts"]) <= set(target.basis)


def _oqci(item, kind="linear"):
    target = targets.for_item(kind, item.width, CONFIG)
    return registry()["oqci"](item, target, {"id": "oqci-default", "layout": "dense"})


def test_a_dropped_gate_fails(item):
    compiled = _oqci(item)
    data = list(compiled.circuit.data)
    index = next(i for i, d in enumerate(data) if d.operation.name == "sx")
    broken = compiled.circuit.copy_empty_like()
    for i, d in enumerate(data):
        if i != index:
            broken.append(d)
    bad = copy.copy(compiled)
    bad.circuit = broken
    assert check(item.circuit, bad).status == "fail"


def test_a_forged_layout_fails(item):
    compiled = _oqci(item)
    bad = copy.copy(compiled)
    bad.final = list(reversed(compiled.final))
    assert check(item.circuit, bad).status == "fail"


def test_a_relative_phase_error_fails_even_though_magnitudes_match():
    # The measured-phase freedom covers a diagonal *after* the computation,
    # not a phase in the middle of it: S vs Sdg between two H's must fail.
    from qiskit import QuantumCircuit

    source = QuantumCircuit(1, 1)
    source.h(0)
    source.s(0)
    source.h(0)
    source.measure(0, 0)
    wrong = QuantumCircuit(1, 1)
    wrong.h(0)
    wrong.sdg(0)
    wrong.h(0)
    wrong.measure(0, 0)
    compiled = Compiled("test", "test", wrong, [0], [0])
    # Note: from |0> these two give the same distribution; only an operator
    # check (all inputs) can tell them apart, and it must.
    assert check(source, compiled).status == "fail"


def test_a_final_diagonal_is_accepted_as_measured_phase():
    from qiskit import QuantumCircuit

    source = QuantumCircuit(2, 2)
    source.h(0)
    source.cx(0, 1)
    source.measure([0, 1], [0, 1])
    with_phase = QuantumCircuit(2, 2)
    with_phase.h(0)
    with_phase.cx(0, 1)
    with_phase.rz(0.4, 1)  # a diagonal right before measurement
    with_phase.measure([0, 1], [0, 1])
    verdict = check(source, Compiled("t", "t", with_phase, [0, 1], [0, 1]))
    assert verdict.status == "pass"
    assert verdict.method == "operator-mod-measured-phase"


def test_measuring_into_the_wrong_classical_bit_fails(item):
    compiled = _oqci(item)
    swapped = compiled.circuit.copy_empty_like()
    measures = [d for d in compiled.circuit.data if d.operation.name == "measure"]
    for d in compiled.circuit.data:
        if d.operation.name != "measure":
            swapped.append(d)
    for d, other in zip(measures, reversed(measures)):
        swapped.measure(d.qubits[0], other.clbits[0])
    bad = copy.copy(compiled)
    bad.circuit = swapped
    assert check(item.circuit, bad).status == "fail"


def test_chi_square_accepts_a_fair_sample_and_rejects_a_biased_one():
    expected = {"00": 0.5, "11": 0.5}
    fair = stats.chi_square_gof({"00": 5010, "11": 4990}, expected, min_expected=5)
    biased = stats.chi_square_gof({"00": 6000, "11": 4000}, expected, min_expected=5)
    impossible = stats.chi_square_gof({"00": 5000, "11": 4990, "01": 10}, expected, min_expected=5)
    assert fair["p_value"] > 0.05
    assert biased["p_value"] < 1e-6
    assert impossible["p_value"] < 1e-3


def test_distances_and_ratios():
    assert stats.total_variation({"0": 1.0}, {"1": 1.0}) == 1.0
    assert stats.hellinger_fidelity({"0": 0.5, "1": 0.5}, {"0": 0.5, "1": 0.5}) == pytest.approx(1.0)
    r = stats.paired_ratio([2, 4, 8], [1, 2, 4], resamples=200, confidence=0.95, seed=1)
    assert r["geometric_mean_ratio"] == pytest.approx(2.0)
    assert r["ci_low"] == pytest.approx(2.0) and r["ci_high"] == pytest.approx(2.0)


def test_strip_measurements_matches_the_rust_definition():
    value = {"a": 1, "timings": {"x": 1}, "passes": [{"duration_us": 3, "id": "p"}],
             "n": {"execution_duration_ms": 1, "k": 2}}
    assert strip_measurements(value) == {"a": 1, "passes": [{"id": "p"}], "n": {"k": 2}}


def test_the_coverage_matrix_cites_only_real_checks():
    matrix = coverage.build()
    assert len(matrix["rows"]) >= 14
    by_id = {r["transformation"]: r for r in matrix["rows"]}
    assert by_id["qir-emission"]["checkers"]["semantic-in-compiler"] == []
    assert by_id["pass:single-qubit-fusion"]["strongest"] == "semantic-in-compiler"


def test_a_missing_coverage_reference_is_an_error(monkeypatch, tmp_path):
    manifest = tmp_path / "checkers.toml"
    manifest.write_text(
        '[[transformation]]\nid = "x"\nstage = "y"\n'
        'checkers = [{ kind = "structural", ref = "src/lib.rs::no_such_function" }]\n'
    )
    monkeypatch.setattr(coverage, "MANIFEST", manifest)
    with pytest.raises(coverage.MissingChecker):
        coverage.build()


def test_a_miniature_run_writes_schema_valid_records(tmp_path):
    from oqci_bench import report, runner

    mini = copy.deepcopy(CONFIG)
    mini["families"] = {"qft": {"sizes": [4]}, "grover": {"sizes": [3]}}
    mini["compilers"] = [c for c in mini["compilers"] if c["id"] in ("oqci-default", "qiskit-o3", "oqci-no-passes")]
    for c in mini["compilers"]:
        c["targets"] = [t for t in c["targets"] if t != "heron"]
    mini["statistics"]["baselines"] = ["qiskit-o3"]
    runner.run(mini, tmp_path, progress=lambda _: None)
    lines = (tmp_path / "records.jsonl").read_text().splitlines()
    assert lines
    for line in lines:
        record = json.loads(line)
        assert schema.validate(record) == [], record["record_id"]
        assert record["correctness"]["status"] == "pass"
        assert (tmp_path / record["result_artifact_paths"]["compiled_qasm"]).exists()
    built = report.build(tmp_path, mini)
    assert (built / "README.md").exists()
    assert (built / "tables" / "paired_ratios.csv").exists()


def test_the_schema_checker_reports_violations():
    errors = schema.validate({"compiler": "gcc"})
    assert any("missing required key 'record_id'" in e for e in errors)
    assert any("'gcc' not in" in e for e in errors)
