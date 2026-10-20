"""Compiled-circuit metrics (Stage G §4), computed one way for every compiler.

Every number here is computed from the compiled Qiskit circuit, by the same
function, whichever compiler produced it — so a difference between compilers
is a difference in their output, not in how each counts. Measurements and
barriers are excluded from gate counts and from depth.

Routing overhead is derived in the report, uniformly: a configuration's
two-qubit count on a constrained target minus its two-qubit count on the
all-to-all target of the same width.
"""

from __future__ import annotations

from typing import Any

_NON_GATES = ("measure", "barrier", "delay", "reset")


def metrics(circuit: Any, basis: list[str]) -> dict[str, Any]:
    one = two = total = native = 0
    counts: dict[str, int] = {}
    for instruction in circuit.data:
        name = instruction.operation.name
        if name in _NON_GATES:
            continue
        total += 1
        counts[name] = counts.get(name, 0) + 1
        if len(instruction.qubits) == 1:
            one += 1
        else:
            two += 1
        if name in basis:
            native += 1
    depth = circuit.depth(lambda i: i.operation.name not in _NON_GATES)
    return {
        "total_gates": total,
        "one_qubit_gates": one,
        "two_qubit_gates": two,
        "depth": int(depth),
        "native_gates": native,
        "non_native_gates": total - native,
        "gate_counts": dict(sorted(counts.items())),
    }
