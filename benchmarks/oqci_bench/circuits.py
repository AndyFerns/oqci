"""Benchmark circuit generators (§20.1, Stage G §5).

Every generator returns a Qiskit ``QuantumCircuit`` that uses **only gates in
OQCI's registered set** — ``h x y z s sdg t tdg sx rx ry rz p cx cy cz swap
ccx`` and ``measure`` — so every compiler receives the identical circuit and
no frontend has to approximate anything. Where a textbook construction needs
a gate outside that set (controlled phase, multi-controlled Z) it is expanded
here, exactly, into gates inside it.

Every circuit ends by measuring every qubit into the classical bit of the same
index. Sizes, seeds and instance counts are parameters; the values used come
from the Stage G config.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Callable, Optional

import numpy as np
from qiskit import QuantumCircuit

from .seeds import item_seed


@dataclass(frozen=True)
class Item:
    """One benchmark circuit and everything needed to regenerate it."""

    circuit_id: str
    family: str
    n: int
    reps: int
    instance: int
    seed: int
    circuit: QuantumCircuit = field(compare=False, repr=False)

    @property
    def width(self) -> int:
        return self.circuit.num_qubits


# --- Building blocks -----------------------------------------------------------


def controlled_phase(qc: QuantumCircuit, theta: float, control: int, target: int) -> None:
    """CP(θ) = diag(1, 1, 1, e^{iθ}), exactly, from p and cx."""
    qc.p(theta / 2, control)
    qc.cx(control, target)
    qc.p(-theta / 2, target)
    qc.cx(control, target)
    qc.p(theta / 2, target)


def multi_controlled_z(qc: QuantumCircuit, qubits: list[int], ancillas: list[int]) -> None:
    """Z on the all-ones state of ``qubits``, exactly, with a CCX ladder.

    Two qubits: CZ. Three: H·CCX·H on the last. More: compute the AND of all
    but the last into clean ancillas with Toffolis, apply the three-qubit
    case, and uncompute — ancillas are returned to |0⟩.
    """
    k = len(qubits)
    if k == 1:
        qc.z(qubits[0])
        return
    if k == 2:
        qc.cz(qubits[0], qubits[1])
        return
    target = qubits[-1]
    controls = qubits[:-1]
    if len(controls) == 2:
        qc.h(target)
        qc.ccx(controls[0], controls[1], target)
        qc.h(target)
        return
    needed = len(controls) - 2
    if len(ancillas) < needed:
        raise ValueError(f"{len(controls)} controls need {needed} ancillas, got {len(ancillas)}")
    ladder = []
    qc.ccx(controls[0], controls[1], ancillas[0])
    ladder.append((controls[0], controls[1], ancillas[0]))
    for i in range(2, len(controls) - 1):
        qc.ccx(controls[i], ancillas[i - 2], ancillas[i - 1])
        ladder.append((controls[i], ancillas[i - 2], ancillas[i - 1]))
    qc.h(target)
    qc.ccx(controls[-1], ancillas[needed - 1], target)
    qc.h(target)
    for a, b, c in reversed(ladder):
        qc.ccx(a, b, c)


def qft_block(qc: QuantumCircuit, qubits: list[int], *, inverse: bool = False) -> None:
    """The quantum Fourier transform (with its final swaps) on ``qubits``."""
    n = len(qubits)
    ops: list[tuple[str, tuple]] = []
    for j in reversed(range(n)):
        ops.append(("h", (qubits[j],)))
        for k in reversed(range(j)):
            ops.append(("cp", (math.pi / 2 ** (j - k), qubits[k], qubits[j])))
    for i in range(n // 2):
        ops.append(("swap", (qubits[i], qubits[n - 1 - i])))
    if inverse:
        ops = [(name, (-args[0],) + args[1:] if name == "cp" else args) for name, args in reversed(ops)]
    for name, args in ops:
        if name == "h":
            qc.h(*args)
        elif name == "swap":
            qc.swap(*args)
        else:
            controlled_phase(qc, args[0], args[1], args[2])


def _measured(qc: QuantumCircuit) -> QuantumCircuit:
    qc.measure(range(qc.num_qubits), range(qc.num_qubits))
    return qc


# --- Families ------------------------------------------------------------------


def qft(n: int) -> QuantumCircuit:
    qc = QuantumCircuit(n, n, name=f"qft_{n}")
    qft_block(qc, list(range(n)))
    return _measured(qc)


def grover(n: int) -> QuantumCircuit:
    """Grover search over ``n`` data qubits for the all-ones string.

    ⌊π/4·√2ⁿ⌋ iterations of oracle (multi-controlled Z) and diffusion, with
    ``max(0, n - 3)`` clean ancillas for the Toffoli ladder. The ancillas are
    measured too; ideally they read 0.
    """
    ancillas = max(0, n - 3)
    qc = QuantumCircuit(n + ancillas, n + ancillas, name=f"grover_{n}")
    data = list(range(n))
    anc = list(range(n, n + ancillas))
    for q in data:
        qc.h(q)
    for _ in range(int(math.floor(math.pi / 4 * math.sqrt(2**n)))):
        multi_controlled_z(qc, data, anc)  # oracle: phase-flip |1...1>
        for q in data:
            qc.h(q)
            qc.x(q)
        multi_controlled_z(qc, data, anc)  # reflection about |0...0>, up to phase
        for q in data:
            qc.x(q)
            qc.h(q)
    return _measured(qc)


def vqe_hea(n: int, reps: int, seed: int) -> QuantumCircuit:
    """A hardware-efficient ansatz: ``reps`` layers of RY+RZ on every qubit and
    a linear CX chain, then a final RY+RZ layer. Angles are drawn uniformly
    from [0, 2π) with ``seed`` and bound — the circuit is concrete."""
    rng = np.random.default_rng(seed)
    qc = QuantumCircuit(n, n, name=f"vqe_{n}_r{reps}")
    for layer in range(reps + 1):
        for q in range(n):
            qc.ry(float(rng.uniform(0, 2 * math.pi)), q)
            qc.rz(float(rng.uniform(0, 2 * math.pi)), q)
        if layer < reps:
            for q in range(n - 1):
                qc.cx(q, q + 1)
    return _measured(qc)


_ONE_QUBIT = ("h", "x", "y", "z", "s", "sdg", "t", "tdg", "sx")
_ROTATIONS = ("rx", "ry", "rz")
_TWO_QUBIT = ("cx", "cz", "swap")


def random_circuit(n: int, gates: int, two_qubit_fraction: float, seed: int) -> QuantumCircuit:
    """A QASMBench-style random circuit: ``gates`` gates, each two-qubit with
    probability ``two_qubit_fraction`` (operands a uniformly random distinct
    pair) and otherwise a uniformly random single-qubit gate on a uniformly
    random qubit; rotation angles uniform in [0, 2π)."""
    rng = np.random.default_rng(seed)
    qc = QuantumCircuit(n, n, name=f"random_{n}")
    singles = _ONE_QUBIT + _ROTATIONS
    for _ in range(gates):
        if n >= 2 and rng.random() < two_qubit_fraction:
            a, b = (int(x) for x in rng.choice(n, size=2, replace=False))
            getattr(qc, _TWO_QUBIT[int(rng.integers(len(_TWO_QUBIT)))])(a, b)
        else:
            q = int(rng.integers(n))
            name = singles[int(rng.integers(len(singles)))]
            if name in _ROTATIONS:
                getattr(qc, name)(float(rng.uniform(0, 2 * math.pi)), q)
            else:
                getattr(qc, name)(q)
    return _measured(qc)


# --- Hardware subset (Stage G §5) ----------------------------------------------


def bell() -> QuantumCircuit:
    qc = QuantumCircuit(2, 2, name="bell")
    qc.h(0)
    qc.cx(0, 1)
    return _measured(qc)


def ghz(n: int) -> QuantumCircuit:
    qc = QuantumCircuit(n, n, name=f"ghz_{n}")
    qc.h(0)
    for q in range(n - 1):
        qc.cx(q, q + 1)
    return _measured(qc)


def qft_roundtrip(n: int, ones: list[int]) -> QuantumCircuit:
    """Prepare the basis state with ``ones`` set, apply QFT then QFT†: the
    ideal outcome is that basis state with certainty."""
    qc = QuantumCircuit(n, n, name=f"qft_roundtrip_{n}")
    for q in ones:
        qc.x(q)
    qft_block(qc, list(range(n)))
    qft_block(qc, list(range(n)), inverse=True)
    return _measured(qc)


# --- Corpus --------------------------------------------------------------------


def corpus(config: dict) -> list[Item]:
    """Every benchmark item the config describes, in a fixed order."""
    master = int(config["seeds"]["master"])
    families = config["families"]
    items: list[Item] = []

    def add(family: str, n: int, reps: int, instance: int, build: Callable[[int], QuantumCircuit]):
        seed = item_seed(master, family, n, reps, instance)
        circuit = build(seed)
        suffix = (f"_r{reps}" if reps else "") + (f"_i{instance}" if family == "random" else "")
        items.append(
            Item(
                circuit_id=f"{family}_{n}{suffix}",
                family=family,
                n=n,
                reps=reps,
                instance=instance,
                seed=seed,
                circuit=circuit,
            )
        )

    if "qft" in families:
        for n in families["qft"]["sizes"]:
            add("qft", n, 0, 0, lambda _s, n=n: qft(n))
    if "grover" in families:
        for n in families["grover"]["sizes"]:
            add("grover", n, 0, 0, lambda _s, n=n: grover(n))
    if "vqe" in families:
        for n in families["vqe"]["sizes"]:
            for reps in families["vqe"]["reps"]:
                add("vqe", n, reps, 0, lambda s, n=n, reps=reps: vqe_hea(n, reps, s))
    if "random" in families:
        spec = families["random"]
        for n in spec["sizes"]:
            for instance in range(int(spec["instances"])):
                add(
                    "random",
                    n,
                    0,
                    instance,
                    lambda s, n=n: random_circuit(
                        n, int(spec["gates_per_qubit"]) * n, float(spec["two_qubit_fraction"]), s
                    ),
                )
    return items


def hardware_subset(config: dict) -> list[Item]:
    """The hardware circuits named by the Stage G config."""
    master = int(config["seeds"]["master"])
    spec = config["hardware"]
    out: list[Item] = []
    for entry in spec["circuits"]:
        kind = entry["kind"]
        n = int(entry.get("n", 0))
        reps = int(entry.get("reps", 0))
        instance = int(entry.get("instance", 0))
        # The same seed the corpus gives the same item, so `random_4` instance
        # 0 on hardware is the very circuit benchmarked in simulation.
        seed = item_seed(master, kind, n, reps, instance)
        if kind == "bell":
            qc, n = bell(), 2
        elif kind == "ghz":
            qc = ghz(n)
        elif kind == "grover":
            qc = grover(n)
        elif kind == "vqe":
            qc = vqe_hea(n, reps, seed)
        elif kind == "random":
            r = config["families"]["random"]
            qc = random_circuit(n, int(r["gates_per_qubit"]) * n, float(r["two_qubit_fraction"]), seed)
        elif kind == "qft_roundtrip":
            qc = qft_roundtrip(n, [int(q) for q in entry["ones"]])
        else:
            raise ValueError(f"unknown hardware circuit kind {kind!r}")
        cid = entry.get("id") or f"{kind}_{n}"
        out.append(Item(cid, kind, n, reps, instance, seed, qc))
    return out


def find(items: list[Item], circuit_id: str) -> Optional[Item]:
    return next((item for item in items if item.circuit_id == circuit_id), None)
