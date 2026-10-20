"""Benchmark targets (Stage G §5): one definition, three compilers' views.

Each target is defined once — a qubit count, a set of directed couplings and
a basis — and rendered for every compiler from that single definition:

- an OQCI basis-profile dict (``oqci.compile(..., target=...)``);
- a Qiskit ``CouplingMap`` (``None`` for all-to-all) and basis-gate list;
- a pytket ``Architecture`` (``None`` for all-to-all) and ``OpType`` basis.

So every compiler sees exactly the same constraints: connectivity and basis.
No compiler is given calibration or error data, because OQCI's profiles carry
none (they would be fabricated for the synthetic targets); comparing error-
aware against error-blind compilers would measure the data, not the compiler.

- **T1** ``all-to-all``: n qubits, every pair coupled both ways, ``{rz, sx, x, cx}``.
- **T2** ``linear``: n qubits in a line, both directions, ``{rz, sx, x, cx}``.
- **T3** ``heron``: the coupling map of a Heron-family fake backend shipped
  with the pinned ``qiskit-ibm-runtime`` — a snapshot of a real device —
  with its ``{rz, sx, x, cz}`` basis.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Optional


@dataclass(frozen=True)
class Target:
    target_id: str
    kind: str  # "all-to-all" | "linear" | "heron"
    num_qubits: int
    edges: tuple[tuple[int, int], ...]
    two_qubit: str  # "cx" | "cz"
    #: Where the coupling map came from, for provenance.
    source: str
    extra: dict[str, Any] = field(default_factory=dict, compare=False)

    @property
    def basis(self) -> list[str]:
        return ["rz", "sx", "x", self.two_qubit]

    @property
    def all_to_all(self) -> bool:
        return self.kind == "all-to-all"

    # --- OQCI ---------------------------------------------------------------

    def oqci_profile(self) -> dict[str, Any]:
        import oqci

        rules = oqci.basis_rule_sets()[self.two_qubit]
        return {
            "id": self.target_id,
            "version": "1",
            "backend_id": "simulator",
            "topology": {"qubit_count": self.num_qubits, "edges": [list(e) for e in self.edges]},
            "supported_operations": sorted(self.basis + ["measure"]),
            "parameter_constraints": {},
            "decomposition_rules": rules,
            "measurement": {"measurement": True, "mid_circuit_measurement": False, "reset": False},
            "capabilities": sorted({f"benchmark-target:{self.kind}", f"source:{self.source}"}),
            "cost_model_id": "nisq-weighted",
        }

    def digest(self) -> str:
        """A content digest of the target definition, recorded with results."""
        import hashlib

        body = json.dumps(
            {"n": self.num_qubits, "edges": self.edges, "basis": self.basis}, sort_keys=True
        )
        return hashlib.sha256(body.encode()).hexdigest()[:16]

    # --- Qiskit -------------------------------------------------------------

    def qiskit_coupling_map(self):
        if self.all_to_all:
            return None
        from qiskit.transpiler import CouplingMap

        return CouplingMap(couplinglist=[list(e) for e in self.edges])

    # --- pytket -------------------------------------------------------------

    def tket_architecture(self):
        if self.all_to_all:
            return None
        from pytket.architecture import Architecture

        return Architecture([tuple(e) for e in self.edges])

    def tket_basis(self):
        from pytket.circuit import OpType

        two = OpType.CX if self.two_qubit == "cx" else OpType.CZ
        return {OpType.Rz, OpType.SX, OpType.X, two}


def all_to_all(n: int) -> Target:
    edges = tuple((a, b) for a in range(n) for b in range(n) if a != b)
    return Target(f"all-to-all-{n}", "all-to-all", n, edges, "cx", "generated")


def linear(n: int) -> Target:
    edges = tuple(sorted({(i, i + 1) for i in range(n - 1)} | {(i + 1, i) for i in range(n - 1)}))
    return Target(f"linear-{n}", "linear", n, edges, "cx", "generated")


_HERON_CACHE: dict[str, Target] = {}


def heron(fake_backend: str) -> Target:
    """The coupling map of a fake backend from ``qiskit_ibm_runtime.fake_provider``."""
    if fake_backend in _HERON_CACHE:
        return _HERON_CACHE[fake_backend]
    import qiskit_ibm_runtime
    from qiskit_ibm_runtime import fake_provider

    backend = getattr(fake_provider, fake_backend)()
    target = backend.target
    if "cz" not in target.operation_names:
        raise ValueError(f"{fake_backend} is not CZ-native")
    edges = tuple(sorted((int(a), int(b)) for a, b in target.qargs_for_operation_name("cz")))
    result = Target(
        f"heron-{backend.name}",
        "heron",
        int(target.num_qubits),
        edges,
        "cz",
        f"{fake_backend}@{getattr(backend, 'backend_version', '?')}"
        f" (qiskit-ibm-runtime {qiskit_ibm_runtime.__version__})",
        extra={"backend_name": backend.name},
    )
    _HERON_CACHE[fake_backend] = result
    return result


def for_item(kind: str, width: int, config: dict) -> Optional[Target]:
    """The target of ``kind`` sized for a circuit of ``width`` qubits, or
    ``None`` if the circuit does not fit (only possible for a fixed device)."""
    if kind == "all-to-all":
        return all_to_all(width)
    if kind == "linear":
        return linear(width)
    if kind == "heron":
        t = heron(config["targets"]["heron"]["fake_backend"])
        return t if width <= t.num_qubits else None
    raise ValueError(f"unknown target kind {kind!r}")
