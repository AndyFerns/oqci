"""Running an OQCI executable on Qiskit Aer.

This is where §15.1's "usable simulator execution path" lives. The Rust side
does everything up to a validated, target-legal executable; this module
replays it and returns the result.

Why the boundary falls here
---------------------------

The project's non-goals rule out "a custom quantum simulator replacing
established simulator frameworks", so the Rust ``Backend::execute`` returns a
typed *not available in this process* error rather than a simulator written to
satisfy the trait. Execution belongs to Aer.

Why the artifact is not OpenQASM
--------------------------------

``qiskit.qasm3.loads`` needs the separate ``qiskit_qasm3_import`` package,
which is not a dependency of this project. More importantly, putting a
third-party parser between "the circuit OQCI verified" and "the circuit that
runs" would mean a parser bug could silently change the program *after*
verification. So the executable is a structured operation list, and this
module replays it by calling one ``QuantumCircuit`` method per operation —
every one of which was checked to exist on the pinned Qiskit, not assumed.

Noise models
------------

Accepted from the caller and passed straight through. **None is ever
invented.** Noise policy belongs to the benchmarking protocol, which is not
yet locked, and a built-in model with plausible-looking parameters would be
fabricated experimental data wearing a library's name.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Any, Mapping, Optional

#: Operations this adapter can replay, mapped to the ``QuantumCircuit`` method
#: that implements each. Every entry was verified to exist by introspecting
#: the installed Qiskit rather than recalled from memory — the one rule the
#: anti-hallucination guidance is most specific about (§33.3, §33.4).
#:
#: ``Opaque`` gates have no entry, and lowering refuses them before an
#: executable is ever built, so a missing key here means a genuine gap rather
#: than a gate this adapter chose not to support.
_METHODS = {
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
    "reset": "reset",
}


class UnsupportedOperation(RuntimeError):
    """An executable named an operation this adapter cannot replay.

    Raised rather than skipped. Dropping an operation would run a different
    circuit from the one OQCI verified, and the counts would look perfectly
    reasonable.
    """


@dataclass
class AerResult:
    """What Aer returned, with the provenance of what produced it."""

    #: Measured bitstrings and how often each occurred.
    counts: dict[str, int]
    #: Per-shot outcomes, when requested.
    memory: Optional[list[str]] = None
    #: Aer's own metadata, verbatim.
    backend_metadata: dict[str, Any] = field(default_factory=dict)
    #: Wall-clock time inside :func:`run`, in milliseconds.
    #:
    #: Kept separate from any compilation timing. Stage C §8 forbids
    #: conflating the two, and this number covers execution only — the
    #: compiler had already finished before the executable reached here.
    execution_duration_ms: Optional[float] = None
    #: Everything needed to cite this result, carried from the executable.
    provenance: dict[str, Any] = field(default_factory=dict)

    @property
    def observed_shots(self) -> int:
        """Shots actually observed, summed from the counts.

        Derived from the data rather than echoed from the request, so a run
        that returned fewer shots than were asked for is visible.
        """
        return sum(self.counts.values())

    def probabilities(self) -> dict[str, float]:
        """The counts as frequencies."""
        total = self.observed_shots
        if total == 0:
            return {}
        return {bits: count / total for bits, count in self.counts.items()}


def to_qiskit(executable: Mapping[str, Any]):
    """Rebuild an OQCI executable as a Qiskit ``QuantumCircuit``.

    Useful on its own for inspection — ``print(to_qiskit(exe))`` draws the
    circuit OQCI actually produced, which is a fast way to sanity-check a
    lowering against a real diagram.

    :raises UnsupportedOperation: if the executable names an operation with no
        known Qiskit method.
    """
    from qiskit import QuantumCircuit

    circuit = QuantumCircuit(
        int(executable["num_qubits"]),
        int(executable["num_clbits"]),
    )

    for index, op in enumerate(executable["ops"]):
        name = op["op"]
        qubits = [int(q) for q in op["qubits"]]
        params = [float(p) for p in op.get("params") or []]

        if name == "measure":
            clbit = op.get("clbit")
            if clbit is None:
                raise UnsupportedOperation(
                    f"operation {index}: a measurement with no destination bit"
                )
            circuit.measure(qubits[0], int(clbit))
            continue

        method = _METHODS.get(name)
        if method is None:
            raise UnsupportedOperation(
                f"operation {index}: `{name}` has no Qiskit equivalent in this adapter. "
                "Lowering should have removed it; this is a gap, not a limitation."
            )
        getattr(circuit, method)(*params, *qubits)

    return circuit


def run(
    executable: Mapping[str, Any],
    *,
    shots: Optional[int] = None,
    seed: Optional[int] = None,
    memory: bool = False,
    noise_model: Any = None,
    **run_options: Any,
) -> AerResult:
    """Execute an OQCI executable on Aer.

    ``shots`` and ``seed`` default to whatever the executable was prepared
    with, so a run reproduces what the provenance record says it did. Passing
    them explicitly overrides that — and the returned provenance is updated to
    match, rather than continuing to claim the original values.

    :param noise_model: a ``qiskit_aer.noise.NoiseModel``, supplied by the
        caller. This module never constructs one; see the module docstring.
    :raises UnsupportedOperation: if the executable contains an operation this
        adapter cannot replay.
    """
    from qiskit_aer import AerSimulator

    settings = executable.get("settings") or {}
    shots = int(settings.get("shots", 1024)) if shots is None else int(shots)
    if seed is None:
        seed = settings.get("seed")
    memory = memory or bool(settings.get("memory", False))

    circuit = to_qiskit(executable)
    if not any(op["op"] == "measure" for op in executable["ops"]):
        raise UnsupportedOperation(
            "this executable has no measurement, so it would produce no counts. "
            "Add one to the source circuit."
        )

    simulator = (
        AerSimulator(noise_model=noise_model) if noise_model is not None else AerSimulator()
    )
    options = dict(run_options)
    if seed is not None:
        options["seed_simulator"] = int(seed)
    if memory:
        options["memory"] = True

    started = time.perf_counter()
    job = simulator.run(circuit, shots=shots, **options)
    result = job.result()
    elapsed_ms = (time.perf_counter() - started) * 1000.0

    raw = result.to_dict()
    metadata = {
        key: raw.get(key)
        for key in ("backend_name", "backend_version", "job_id", "date", "time_taken")
        if raw.get(key) is not None
    }

    # The provenance travels with the result, updated to describe the run that
    # actually happened rather than the one the artifact was prepared for.
    provenance = dict(executable.get("provenance") or {})
    provenance["shots"] = shots
    provenance["seed"] = seed

    return AerResult(
        counts={str(bits): int(count) for bits, count in result.get_counts().items()},
        memory=list(result.get_memory()) if memory else None,
        backend_metadata=metadata,
        execution_duration_ms=elapsed_ms,
        provenance=provenance,
    )
