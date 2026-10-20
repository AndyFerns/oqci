"""The compilers under comparison, behind one interface (§20.2).

Every compiler takes a benchmark item and a target and returns a
:class:`Compiled`: the output as a Qiskit ``QuantumCircuit`` **over physical
qubit indices**, plus where each logical qubit started and ended. That one
shape is what lets a single metric function and a single equivalence checker
judge every compiler the same way.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Callable, Optional


@dataclass
class Compiled:
    compiler: str
    config_id: str
    #: The output over physical qubits 0..num_qubits-1.
    circuit: Any
    #: logical qubit i started on physical qubit initial[i].
    initial: list[int]
    #: logical qubit i ended on physical qubit final[i].
    final: list[int]
    #: SWAPs inserted, when the compiler reports it; None otherwise.
    swaps: Optional[int] = None
    #: Compiler-specific detail (OQCI's stage timings and pass records, ...).
    internal: dict[str, Any] = field(default_factory=dict)


#: A compile function: (item, target, config entry) -> Compiled.
CompileFn = Callable[[Any, Any, dict], Compiled]


def registry() -> dict[str, CompileFn]:
    from . import oqci_compiler, qiskit_compiler, tket_compiler

    return {
        "oqci": oqci_compiler.compile_item,
        "qiskit": qiskit_compiler.compile_item,
        "tket": tket_compiler.compile_item,
    }


def versions() -> dict[str, str]:
    """The exact version of every compiler, for the result records."""
    out: dict[str, str] = {}
    try:
        import oqci

        out["oqci"] = oqci.__version__
    except ImportError:  # pragma: no cover
        pass
    try:
        import qiskit

        out["qiskit"] = qiskit.__version__
    except ImportError:  # pragma: no cover
        pass
    try:
        import pytket

        out["tket"] = pytket.__version__
    except ImportError:  # pragma: no cover
        pass
    return out
