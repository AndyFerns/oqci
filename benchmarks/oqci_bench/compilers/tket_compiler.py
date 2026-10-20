"""The t|ket⟩ baseline (§20.2), with the fixed recipe Stage G locks:

1. ``FullPeepholeOptimise(allow_swaps=False)`` — t|ket⟩'s strongest
   general-purpose optimization. ``allow_swaps=False`` keeps wires explicit:
   with swaps allowed, t|ket⟩ may leave an *implicit* qubit permutation that
   a layout-blind reader would misattribute.
2. ``DefaultMappingPass(architecture)`` — placement and routing on the same
   coupling map every compiler gets (skipped for all-to-all).
3. ``AutoRebase`` to the target's basis ``{Rz, SX, X, CX|CZ}``.
4. ``RemoveRedundancies``.

One fallback, decided with the user after the Stage G reference run found the
case: ``FullPeepholeOptimise(allow_swaps=False)`` can *still* leave an implicit
qubit permutation (pytket 2.18.5 does so on ``random_4_i6``, with or without
its explicit SWAPs decomposed). The permutation is then made explicit, but not
by appending SWAPs to the measured circuit: the resulting gates would act
after the final measurements, which no checker can judge. Instead,
step 1 is redone on the circuit *without* its final measurements, the
permutation becomes SWAP gates (``replace_implicit_wire_swaps``), and the
measurements are put back before steps 2–4. The fallback runs only when the
recipe leaves a permutation: re-running every circuit unmeasured would change
t|ket⟩'s output on circuits the recipe already handles. A record that used it
says so in ``compiler_notes``, and its output is equivalence-checked like any
other.

The result is converted to a Qiskit circuit over physical qubit indices by
:func:`to_physical_qiskit`, not by ``tk_to_qiskit``, so physical node numbers
survive exactly. Layouts come from the compilation unit's initial and final
maps.
"""

from __future__ import annotations

import math
import re
from typing import Any

from . import Compiled

_NODE = re.compile(r"^(?:node|q)\[(\d+)\]$")


def _index(unit: Any) -> int:
    match = _NODE.match(str(unit))
    if not match:
        raise ValueError(f"unexpected t|ket> unit {unit}")
    return int(match.group(1))


def to_physical_qiskit(circuit: Any, width: int):
    """A t|ket⟩ circuit over ``node[i]`` / ``q[i]`` as a Qiskit circuit whose
    qubit ``i`` is node ``i``. Only the target basis, measurement, barriers
    and global phase are accepted; anything else is an error, not dropped."""
    from pytket.circuit import OpType
    from qiskit import QuantumCircuit

    clbits = [b for b in circuit.bits]
    out = QuantumCircuit(width, len(clbits))
    bit_index = {str(b): _index_bit(b) for b in clbits}
    for command in circuit.get_commands():
        kind = command.op.type
        qubits = [_index(q) for q in command.qubits]
        if kind == OpType.Rz:
            out.rz(float(command.op.params[0]) * math.pi, qubits[0])
        elif kind == OpType.SX:
            out.sx(qubits[0])
        elif kind == OpType.X:
            out.x(qubits[0])
        elif kind == OpType.CX:
            out.cx(qubits[0], qubits[1])
        elif kind == OpType.CZ:
            out.cz(qubits[0], qubits[1])
        elif kind == OpType.Measure:
            out.measure(qubits[0], bit_index[str(command.bits[0])])
        elif kind in (OpType.Barrier, OpType.Phase):
            continue  # no semantics on the state (Phase is a global phase)
        else:
            raise ValueError(f"t|ket> left a {kind} outside the target basis")
    return out


def _index_bit(bit: Any) -> int:
    match = re.match(r"^c\[(\d+)\]$", str(bit))
    if not match:
        raise ValueError(f"unexpected t|ket> bit {bit}")
    return int(match.group(1))


def compile_item(item: Any, target: Any, config: dict) -> Compiled:
    from pytket.extensions.qiskit import qiskit_to_tk
    from pytket.passes import (
        AutoRebase,
        DefaultMappingPass,
        FullPeepholeOptimise,
        RemoveRedundancies,
    )
    from pytket.predicates import CompilationUnit

    notes = []
    unit = CompilationUnit(qiskit_to_tk(item.circuit))
    FullPeepholeOptimise(allow_swaps=False).apply(unit)
    if _has_permutation(unit.circuit):
        unit = CompilationUnit(_explicit_permutation(item.circuit))
        notes.append(
            "fallback: FullPeepholeOptimise left an implicit qubit permutation, so it was "
            "rerun without the final measurements, the permutation made explicit with "
            "replace_implicit_wire_swaps, and the measurements restored"
        )
    architecture = target.tket_architecture()
    if architecture is not None:
        DefaultMappingPass(architecture).apply(unit)
    AutoRebase(target.tket_basis()).apply(unit)
    RemoveRedundancies().apply(unit)

    n = item.circuit.num_qubits
    initial = [0] * n
    final = [0] * n
    for logical, placed in unit.initial_map.items():
        if str(logical).startswith("q["):
            initial[_index(logical)] = _index(placed)
    for logical, placed in unit.final_map.items():
        if str(logical).startswith("q["):
            final[_index(logical)] = _index(placed)
    width = target.num_qubits if architecture is not None else n
    if _has_permutation(unit.circuit):
        raise ValueError("t|ket> left an implicit qubit permutation despite allow_swaps=False")
    return Compiled(
        compiler="tket",
        config_id=config["id"],
        circuit=to_physical_qiskit(unit.circuit, width),
        initial=initial,
        final=final,
        swaps=None,  # not reported in a comparable form
        internal={
            "recipe": "FullPeepholeOptimise(allow_swaps=False) > DefaultMappingPass > AutoRebase > RemoveRedundancies",
            "notes": notes,
        },
    )


def _has_permutation(circuit: Any) -> bool:
    return any(str(a) != str(b) for a, b in circuit.implicit_qubit_permutation().items())


def _explicit_permutation(source: Any):
    """Step 1 on ``source`` without its final measurements, with any implicit
    permutation turned into SWAP gates, then the same measurements again."""
    from pytket.circuit import Bit, Qubit
    from pytket.extensions.qiskit import qiskit_to_tk
    from pytket.passes import FullPeepholeOptimise
    from pytket.predicates import CompilationUnit

    body = source.remove_final_measurements(inplace=False)
    if any(instruction.operation.name == "measure" for instruction in body.data):
        raise ValueError("t|ket> fallback needs every measurement to be final")
    measures = [
        (source.find_bit(i.qubits[0]).index, source.find_bit(i.clbits[0]).index)
        for i in source.data
        if i.operation.name == "measure"
    ]
    unit = CompilationUnit(qiskit_to_tk(body))
    FullPeepholeOptimise(allow_swaps=False).apply(unit)
    if any(str(a) != str(b) for a, b in unit.initial_map.items()):
        raise ValueError("FullPeepholeOptimise relabelled qubits")  # it never places
    circuit = unit.circuit  # a copy: the unit is not changed below
    circuit.replace_implicit_wire_swaps()
    circuit.add_c_register("c", source.num_clbits)
    for qubit, bit in measures:
        circuit.Measure(Qubit("q", qubit), Bit("c", bit))
    return circuit
