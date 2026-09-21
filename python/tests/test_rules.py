"""Cross-checks every decomposition rule against Qiskit's gate semantics.

`tests/decomposition.rs` already verifies each rule against the project's own
state-vector harness. This file verifies the same rules against
`qiskit.quantum_info.Operator` — a completely separate implementation, written
by different people, with its own conventions.

That is the point. A rule can only pass both if it is right, or if two
independent implementations happen to be wrong in exactly the same way. One
oracle catches typos; two catch misunderstandings.

The rule table is read from the compiler with `oqci.decomposition_rules()`
rather than transcribed here. A hand-copied list would be a third source of
truth, and the one most likely to go stale.
"""

from __future__ import annotations

import math

import pytest

oqci = pytest.importorskip("oqci")
qiskit = pytest.importorskip("qiskit")

from qiskit import QuantumCircuit  # noqa: E402
from qiskit.quantum_info import Operator  # noqa: E402

#: Angles for parameterized rules. Deliberately includes zero, a negative, the
#: axis-aligned values where a sign error can hide behind symmetry, and one
#: with no special structure at all.
ANGLES = [0.0, 0.41, -1.13, math.pi / 2, math.pi, 2.4567]

#: Rule step mnemonics to the `QuantumCircuit` methods that implement them.
METHODS = {
    "id": "id",
    "x": "x",
    "y": "y",
    "z": "z",
    "h": "h",
    "s": "s",
    "sdg": "sdg",
    "t": "t",
    "tdg": "tdg",
    "sx": "sx",
    "sxdg": "sxdg",
    "rx": "rx",
    "ry": "ry",
    "rz": "rz",
    "p": "p",
    "u": "u",
    "cx": "cx",
    "cy": "cy",
    "cz": "cz",
    "swap": "swap",
    "ccx": "ccx",
}


def source_params(rule, seed):
    """Concrete parameters for a rule, drawn from `ANGLES`."""
    return [ANGLES[(seed + 3 * i) % len(ANGLES)] for i in range(rule["source_params"])]


def apply_transform(transform, params):
    """Evaluates one `ParamTransform` against the source's parameters."""
    if transform["kind"] == "constant":
        return float(transform["value"])
    return float(transform["scale"]) * params[transform["index"]] + float(transform["offset"])


def source_circuit(rule, params):
    """The rule's source operation, as a Qiskit circuit."""
    circuit = QuantumCircuit(rule["source_arity"])
    getattr(circuit, METHODS[rule["source"]])(*params, *range(rule["source_arity"]))
    return circuit


def expansion_circuit(rule, params):
    """The sequence the rule expands to, as a Qiskit circuit."""
    circuit = QuantumCircuit(rule["source_arity"])
    for step in rule["steps"]:
        step_params = [apply_transform(t, params) for t in step["params"]]
        getattr(circuit, METHODS[step["op"]])(*step_params, *step["operands"])
    return circuit


def test_the_rule_table_is_readable_and_non_empty():
    rules = oqci.decomposition_rules()
    assert rules, "the compiler reported no decomposition rules"
    for rule in rules:
        assert rule["source"] in METHODS, f"unknown source gate `{rule['source']}`"
        for step in rule["steps"]:
            assert step["op"] in METHODS, f"unknown target gate `{step['op']}`"


@pytest.mark.parametrize("rule", oqci.decomposition_rules(), ids=lambda r: r["id"])
def test_every_rule_reproduces_its_source(rule):
    """The rule computes what it says it computes.

    Checked at several parameter values, because a rule can be accidentally
    correct at one: `theta = 0` makes most rotations the identity, and
    `theta = pi/2` sits on an axis where several wrong signs agree with the
    right one.
    """
    for seed in range(len(ANGLES)):
        params = source_params(rule, seed)
        source = Operator(source_circuit(rule, params))
        expansion = Operator(expansion_circuit(rule, params))
        assert source.equiv(expansion), (
            f"rule `{rule['id']}` does not implement `{rule['source']}` "
            f"at parameters {params}"
        )


@pytest.mark.parametrize("rule", oqci.decomposition_rules(), ids=lambda r: r["id"])
def test_every_rule_declares_the_exactness_qiskit_measures(rule):
    """The declared exactness matches what a second implementation says.

    `Operator.__eq__` is entrywise equality; `.equiv` allows a shared global
    phase. A rule claiming to be exact when only the weaker relation holds is
    a claim no runtime test could otherwise catch, because a global phase is
    unobservable — right up until someone adds a controlled construct, at
    which point every such rule becomes retroactively wrong.
    """
    params = source_params(rule, 1)
    source = Operator(source_circuit(rule, params))
    expansion = Operator(expansion_circuit(rule, params))

    measured = "exact" if source == expansion else "up_to_global_phase"
    assert rule["exactness"] == measured, (
        f"rule `{rule['id']}` declares {rule['exactness']} but Qiskit measures {measured}"
    )


def test_a_deliberately_broken_rule_would_be_caught():
    """Evidence that this file can fail.

    `cz-to-cx` with one conjugating H removed — a plausible typo, and one that
    leaves the circuit perfectly legal. If this assertion ever stops holding,
    the checks above have stopped testing anything.
    """
    correct = QuantumCircuit(2)
    correct.cz(0, 1)

    broken = QuantumCircuit(2)
    broken.h(1)
    broken.cx(0, 1)

    assert not Operator(correct).equiv(Operator(broken))


def test_the_orientation_repair_identity_holds():
    """`CX(b,a) = (H x H) CX(a,b) (H x H)`, exactly.

    Not a decomposition rule — orientation repair is a separate phase, so it
    is not in the rule table — but the compiler relies on this identity every
    time it meets a one-way coupling pointing the wrong way, and it is checked
    against the same independent implementation as everything else.
    """
    reversed_cx = QuantumCircuit(2)
    reversed_cx.cx(1, 0)

    conjugated = QuantumCircuit(2)
    conjugated.h(0)
    conjugated.h(1)
    conjugated.cx(0, 1)
    conjugated.h(0)
    conjugated.h(1)

    assert Operator(reversed_cx) == Operator(conjugated)


def test_the_symmetric_orientation_policies_really_are_symmetric():
    """`Cz` and `Swap` are unchanged by exchanging their operands.

    The compiler repairs a reversed one by relabelling, with no added gates.
    Getting this wrong for an asymmetric gate would be silent: a swapped
    control and target agree on every computational basis state.
    """
    for name in ["cz", "swap"]:
        forward = QuantumCircuit(2)
        getattr(forward, name)(0, 1)
        reverse = QuantumCircuit(2)
        getattr(reverse, name)(1, 0)
        assert Operator(forward) == Operator(reverse), f"`{name}` is not symmetric"

    # ...and the contrast that makes the claim meaningful.
    forward_cx = QuantumCircuit(2)
    forward_cx.cx(0, 1)
    reverse_cx = QuantumCircuit(2)
    reverse_cx.cx(1, 0)
    assert not Operator(forward_cx).equiv(Operator(reverse_cx)), (
        "if CX were symmetric, no orientation repair would be needed at all"
    )
