"""End-to-end execution: OQCI compiles, Aer runs, the distribution is checked.

Every other test in this project checks OQCI against OQCI — its own
state-vector harness, its own legality checker, its own metrics. This file is
the first that checks it against something else *running the result*. A
lowering that is internally consistent and physically wrong fails here and
nowhere else.

The circuits are chosen so the expected distribution is known exactly and is
not what an unlowered circuit would produce. Compiling a Bell pair between
``q0`` and ``q2`` onto a **linear** device forces a SWAP and an ``h`` that has
to be rewritten as ``rz``/``sx``; if any of that were wrong, the counts would
say so.

Seeds are fixed for reproducibility and are *recorded*, never invented as
experimental parameters: the benchmarking protocol owns those, and this is a
correctness test, not an experiment.
"""

from __future__ import annotations

import pytest

oqci = pytest.importorskip("oqci")
pytest.importorskip("qiskit")
pytest.importorskip("qiskit_aer")

SHOTS = 8000
SEED = 20260921

#: How far a measured frequency may sit from its expected value.
#:
#: At 8000 shots the standard error on a 50/50 split is about 0.56%, so 4%
#: is roughly seven sigma — loose enough never to flake, tight enough that a
#: genuinely wrong circuit cannot slip through. A wrong lowering does not
#: shift a distribution slightly; it produces a different one.
TOLERANCE = 0.04

BELL_ADJACENT = "qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;"
BELL_DISTANT = "qubit[3] q; bit[3] c; h q[0]; cx q[0], q[2]; c = measure q;"
GHZ3 = "qubit[3] q; bit[3] c; h q[0]; cx q[0], q[1]; cx q[1], q[2]; c = measure q;"


def run(source, backend, **kwargs):
    """Compile and execute, returning the measured frequencies."""
    artifacts = oqci.compile(source, backend=backend, shots=SHOTS, seed=SEED, **kwargs)
    assert artifacts["lowering"]["legal"], (
        f"compilation produced an illegal circuit: {artifacts['lowering']}"
    )
    result = oqci.backends.aer.run(artifacts["executable"])
    return artifacts, result


def assert_distribution(result, expected):
    """Assert the measured frequencies match, and that nothing else appeared."""
    observed = result.probabilities()
    assert result.observed_shots == SHOTS

    unexpected = {bits: p for bits, p in observed.items() if bits not in expected}
    assert not unexpected or max(unexpected.values()) < TOLERANCE, (
        f"outcomes that should be impossible appeared: {unexpected}"
    )
    for bits, probability in expected.items():
        assert abs(observed.get(bits, 0.0) - probability) < TOLERANCE, (
            f"P({bits}) was {observed.get(bits, 0.0):.3f}, expected {probability:.3f}; "
            f"full distribution {observed}"
        )


def test_a_bell_pair_runs_on_an_unconstrained_simulator():
    _, result = run(BELL_ADJACENT, "simulator")
    assert_distribution(result, {"00": 0.5, "11": 0.5})


def test_a_bell_pair_survives_lowering_to_a_restricted_basis():
    """The same circuit, through `h -> rz/sx` decomposition.

    If the Euler decomposition were mis-signed, this is where it shows: the
    state would no longer be an even superposition, or would pick up
    population on ``01``/``10``.
    """
    artifacts, result = run(BELL_ADJACENT, "simulator-nisq")
    assert "h-to-rz-sx" in artifacts["lowering"]["rules_applied"]
    assert_distribution(result, {"00": 0.5, "11": 0.5})


def test_a_distant_bell_pair_survives_routing():
    """q0 and q2 are two hops apart on a line, so routing must insert a SWAP.

    This is the test the whole lowering layer exists to pass. The measured
    correlation is between ``q0`` and ``q2``, so a SWAP applied to the wrong
    pair, or a final layout reported wrongly, changes which bits are
    correlated — and the counts would show ``011`` or ``110`` instead.
    """
    artifacts, result = run(BELL_DISTANT, "simulator-nisq")
    assert artifacts["lowering"]["swaps_inserted"] > 0, "this circuit must need routing"
    # Qiskit prints the most significant bit first, so `101` is c2=1, c1=0,
    # c0=1 — exactly the q0/q2 correlation the program asked for.
    assert_distribution(result, {"000": 0.5, "101": 0.5})


def test_a_ghz_state_survives_lowering():
    _, result = run(GHZ3, "simulator-nisq")
    assert_distribution(result, {"000": 0.5, "111": 0.5})


def test_the_constrained_and_unconstrained_paths_agree():
    """The same program on two very different devices must measure the same.

    One runs the circuit essentially as written; the other routes it and
    rewrites every gate into ``{rz, sx, x, cx}``. Agreement between them is
    evidence about the *lowering*, since that is the only thing that differs.
    """
    _, ideal = run(GHZ3, "simulator")
    _, constrained = run(GHZ3, "simulator-nisq")

    ideal_p = ideal.probabilities()
    constrained_p = constrained.probabilities()
    for bits in set(ideal_p) | set(constrained_p):
        assert abs(ideal_p.get(bits, 0.0) - constrained_p.get(bits, 0.0)) < TOLERANCE, (
            f"the two paths disagree on {bits}: {ideal_p} vs {constrained_p}"
        )


def test_a_dense_layout_reaches_the_same_distribution():
    """Layout choice changes the SWAP count, never the answer."""
    _, trivial = run(BELL_DISTANT, "simulator-nisq", layout="trivial")
    _, dense = run(BELL_DISTANT, "simulator-nisq", layout="dense")
    assert_distribution(trivial, {"000": 0.5, "101": 0.5})
    assert_distribution(dense, {"000": 0.5, "101": 0.5})


def test_the_ibm_shaped_target_compiles_and_the_result_is_still_correct():
    """A directed-coupling target, run on a simulator.

    ``ibm-illustrative`` describes no real device, and nothing here claims it
    does. What it exercises is the directed-edge path: its ring has two
    one-way links, so orientation repair is reachable. Running the result on
    Aer checks that the repair preserves semantics — hardware would too, if
    this project had access to any.
    """
    artifacts = oqci.compile(GHZ3, backend="ibm-illustrative", shots=SHOTS, seed=SEED)
    assert artifacts["lowering"]["legal"]
    result = oqci.backends.aer.run(artifacts["executable"])
    assert_distribution(result, {"000": 0.5, "111": 0.5})


def test_a_seeded_run_is_reproducible():
    _, first = run(GHZ3, "simulator-nisq")
    _, second = run(GHZ3, "simulator-nisq")
    assert first.counts == second.counts


def test_results_carry_the_provenance_of_what_produced_them():
    artifacts, result = run(BELL_DISTANT, "simulator-nisq")
    provenance = result.provenance

    assert provenance["backend_id"] == "simulator-nisq"
    assert provenance["profile_id"] == "linear-nisq@1"
    assert provenance["compiler_version"] == oqci.__version__
    assert provenance["git_commit"]
    assert provenance["cost_model_id"] == "nisq-weighted"
    assert provenance["cost_model_configuration"], "the weights behind the score"
    assert provenance["swaps_inserted"] == artifacts["lowering"]["swaps_inserted"]
    assert provenance["shots"] == SHOTS and provenance["seed"] == SEED

    # Aer's own report is kept verbatim, not normalized away.
    assert result.backend_metadata["backend_name"] == "aer_simulator"
    assert result.execution_duration_ms is not None


def test_the_executable_only_uses_operations_the_target_supports():
    artifacts = oqci.compile(GHZ3, backend="simulator-nisq")
    basis = {"rz", "sx", "x", "cx", "measure"}
    used = {op["op"] for op in artifacts["executable"]["ops"]}
    assert used <= basis, f"{used - basis} is outside the linear-nisq basis"


def test_an_unbound_parameter_is_refused_rather_than_guessed():
    source = "input float[64] theta; qubit[1] q; bit[1] c; rz(theta) q[0]; c[0] = measure q[0];"
    with pytest.raises(oqci.OqciError) as caught:
        oqci.compile(source, backend="simulator")
    assert "bind" in str(caught.value).lower()

    # Bound, the same program compiles and runs.
    artifacts = oqci.compile(source, backend="simulator", bindings={"theta": 0.5})
    result = oqci.backends.aer.run(artifacts["executable"], shots=200, seed=SEED)
    # Rz on |0> is a phase; the outcome is still deterministic.
    assert result.counts == {"0": 200}


def test_an_unmeasured_circuit_is_refused_before_it_runs():
    """No measurement means no counts, which looks like a failed run."""
    artifacts = oqci.compile("qubit[1] q; h q[0];", backend="simulator")
    with pytest.raises(oqci.backends.aer.UnsupportedOperation):
        oqci.backends.aer.run(artifacts["executable"])


def test_an_executable_can_be_inspected_as_a_qiskit_circuit():
    """`to_qiskit` renders what OQCI actually produced."""
    artifacts = oqci.compile(BELL_DISTANT, backend="simulator-nisq")
    circuit = oqci.backends.aer.to_qiskit(artifacts["executable"])
    assert circuit.num_qubits == artifacts["executable"]["num_qubits"]
    assert len(circuit.data) == len(artifacts["executable"]["ops"])


def test_noise_models_are_accepted_but_never_invented():
    """A caller's noise model is passed through; none is supplied by default.

    Noise policy belongs to the benchmarking protocol, which is not locked. A
    built-in model with plausible parameters would be fabricated experimental
    data.
    """
    from qiskit_aer.noise import NoiseModel

    artifacts = oqci.compile(BELL_ADJACENT, backend="simulator")
    result = oqci.backends.aer.run(
        artifacts["executable"], shots=500, seed=SEED, noise_model=NoiseModel()
    )
    assert result.observed_shots == 500


def test_target_independent_compilation_produces_no_executable():
    """Nothing to lower to, so nothing to run — and the SDK says so plainly."""
    artifacts = oqci.compile(BELL_ADJACENT)
    assert artifacts["backend"] is None
    assert "executable" not in artifacts
    assert "lowering" not in artifacts
