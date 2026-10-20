"""OQCI, through its Python SDK — the same path a user takes.

The circuit enters as the Qiskit ``QuantumCircuit`` every compiler receives
(through OQCI's Qiskit frontend), is compiled for the target's profile, and
the prepared executable is turned back into a Qiskit circuit over physical
qubits with ``oqci.backends.aer.to_qiskit`` — the same translation the Aer
adapter executes. Layouts come from the executable's provenance.
"""

from __future__ import annotations

from typing import Any

from . import Compiled


def compile_item(item: Any, target: Any, config: dict) -> Compiled:
    import oqci
    from oqci.backends.aer import to_qiskit

    artifacts = oqci.compile(
        item.circuit,
        target=target.oqci_profile(),
        name=item.circuit_id,
        layout=config.get("layout", "dense"),
        passes=config.get("passes"),
        disable=config.get("disable"),
    )
    executable = artifacts["executable"]
    provenance = executable["provenance"]
    return Compiled(
        compiler="oqci",
        config_id=config["id"],
        circuit=to_qiskit(executable),
        initial=[int(p) for p in provenance["initial_layout"]],
        final=[int(p) for p in provenance["final_layout"]],
        swaps=int(provenance["swaps_inserted"]),
        internal={
            "timings": artifacts["timings"],
            "passes": [
                {
                    "id": p["id"],
                    "enabled": p["enabled"],
                    "changed": p["changed"],
                    "before_ops": p["before"]["op_count"],
                    "after_ops": p["after"]["op_count"],
                    "before_two_qubit": p["before"]["two_qubit_count"],
                    "after_two_qubit": p["after"]["two_qubit_count"],
                }
                for p in artifacts["passes"]
            ],
            "cost": artifacts["cost"],
            "provenance": provenance,
            "qir": artifacts["qir"],
        },
    )
