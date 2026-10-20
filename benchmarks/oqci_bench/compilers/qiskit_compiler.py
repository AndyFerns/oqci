"""The Qiskit transpiler baseline (§20.2): ``transpile`` at a fixed
optimization level, given the target's coupling map and basis — and nothing
else — with ``seed_transpiler`` set to the item's seed (Stage G §5).

Layouts come from the transpiled circuit's ``TranspileLayout``. With no
coupling map (all-to-all) Qiskit applies no layout, so the identity is used.
"""

from __future__ import annotations

from typing import Any

from . import Compiled


def compile_item(item: Any, target: Any, config: dict) -> Compiled:
    from qiskit import transpile

    level = int(config["optimization_level"])
    transpiled = transpile(
        item.circuit,
        coupling_map=target.qiskit_coupling_map(),
        basis_gates=target.basis,
        optimization_level=level,
        seed_transpiler=item.seed,
    )
    n = item.circuit.num_qubits
    if transpiled.layout is None:
        initial = final = list(range(n))
    else:
        initial = [int(p) for p in transpiled.layout.initial_index_layout(filter_ancillas=True)]
        final = [int(p) for p in transpiled.layout.final_index_layout(filter_ancillas=True)]
    return Compiled(
        compiler="qiskit",
        config_id=config["id"],
        circuit=transpiled,
        initial=initial,
        final=final,
        swaps=None,  # not reported by transpile
        internal={"optimization_level": level, "seed_transpiler": item.seed},
    )
