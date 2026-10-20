"""Correctness of every compiled output (Stage G §5, locked).

Each compiler's output is checked against the source circuit, layouts
applied:

- **operator** (``n ≤ operator_max_qubits``): the compiled circuit, run on
  every computational basis input placed on its initial layout, must
  reproduce the source's output read from its final layout. Two strengths
  are recorded:

  - ``operator`` — equal up to one global phase over all inputs (the trace,
    modulus taken last);
  - ``operator-mod-measured-phase`` — ``U_compiled = D·U_source`` with ``D``
    diagonal in the computational basis. When every qubit is measured at the
    end this is *exactly* "the same outcome distribution for every input
    state" (a diagonal before a Z measurement is unobservable), which is the
    freedom optimizers legitimately use — Qiskit's
    ``RemoveDiagonalGatesBeforeMeasure``, for one. Checked row by row:
    ``|(U_c U_s†)_jj| = 1`` for every output ``j``.

- **state from |0⟩** (``n ≤ state_max_qubits``): the same comparison for the
  all-zeros input only: ``state-from-zero`` (equal up to global phase) or,
  when every qubit is measured, ``distribution-from-zero`` (equal amplitude
  magnitudes on every outcome, nothing left on ancillas).

Measurements must be terminal, and each classical bit must be measured from
the physical qubit that holds, at the end, the logical qubit the source
measures into it. Gate semantics come from Qiskit's own matrices
(``Operation.to_matrix``), independent of OQCI's.

A ``fail`` verdict aborts the run (Stage G: a failed check aborts). A circuit
too wide to simulate is ``not-checked``, never ``pass``, and is reported.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Optional

import numpy as np

#: Structural bound on the simulation: 2^24 amplitudes per input column.
MAX_SIMULATED_WIDTH = 24
#: Rough memory budget per simulated batch, in amplitudes.
_BATCH_AMPLITUDES = 1 << 24


@dataclass
class Verdict:
    status: str  # "pass" | "fail" | "not-checked"
    method: str  # "operator" | "state-from-zero" | "none"
    residual: Optional[float]
    detail: str = ""

    def to_dict(self) -> dict[str, Any]:
        return {
            "status": self.status,
            "method": self.method,
            "residual": self.residual,
            "detail": self.detail,
        }


class NotTerminal(ValueError):
    pass


def _program(circuit) -> tuple[list[tuple[np.ndarray, list[int]]], dict[int, int], set[int]]:
    """Gates (matrix, qubits), the terminal measurement map clbit -> qubit,
    and the set of touched qubits."""
    gates: list[tuple[np.ndarray, list[int]]] = []
    measures: dict[int, int] = {}
    measured: set[int] = set()
    touched: set[int] = set()
    cache: dict[tuple, np.ndarray] = {}
    for instruction in circuit.data:
        operation = instruction.operation
        name = operation.name
        qubits = [circuit.find_bit(q).index for q in instruction.qubits]
        if name == "barrier":
            continue
        if name == "measure":
            q = qubits[0]
            if q in measured:
                raise NotTerminal(f"qubit {q} is measured twice")
            measured.add(q)
            measures[circuit.find_bit(instruction.clbits[0]).index] = q
            touched.add(q)
            continue
        if any(q in measured for q in qubits):
            raise NotTerminal(f"`{name}` acts on {qubits} after a measurement")
        if name in ("reset", "delay"):
            raise NotTerminal(f"`{name}` is not a unitary")
        key = (name, tuple(float(p) for p in operation.params))
        matrix = cache.get(key)
        if matrix is None:
            matrix = np.asarray(operation.to_matrix(), dtype=complex)
            cache[key] = matrix
        gates.append((matrix, qubits))
        touched.update(qubits)
    return gates, measures, touched


def _apply(state: np.ndarray, width: int, matrix: np.ndarray, qubits: list[int]) -> np.ndarray:
    """Apply a k-qubit Qiskit matrix to a (batch, 2^width) state.

    Qiskit matrices are little-endian in their qargs: bit i of the matrix
    index belongs to qargs[i]. Axis 1 + (width-1-q) of the reshaped state
    is qubit q (little-endian flat index).
    """
    k = len(qubits)
    batch = state.shape[0]
    tensor = state.reshape((batch,) + (2,) * width)
    m = matrix.reshape((2,) * (2 * k))  # out bits (q_{k-1}..q_0), in bits likewise
    axes = [1 + (width - 1 - q) for q in reversed(qubits)]
    moved = np.tensordot(m, tensor, axes=(list(range(k, 2 * k)), axes))
    restored = np.moveaxis(moved, list(range(k)), axes)
    return restored.reshape(batch, 1 << width)


def _run(gates, compact: dict[int, int], width: int, inputs: np.ndarray) -> np.ndarray:
    state = np.zeros((len(inputs), 1 << width), dtype=complex)
    state[np.arange(len(inputs)), inputs] = 1.0
    for matrix, qubits in gates:
        state = _apply(state, width, matrix, [compact[q] for q in qubits])
    return state


def _place(bits: np.ndarray, positions: list[int]) -> np.ndarray:
    """Map logical bit patterns onto compact physical indices."""
    out = np.zeros_like(bits)
    for logical, physical in enumerate(positions):
        out |= ((bits >> logical) & 1) << physical
    return out


def check(source, compiled, *, operator_max_qubits: int, state_max_qubits: int,
          tolerance: float) -> Verdict:
    """Is ``compiled`` (a :class:`~oqci_bench.compilers.Compiled`) the source?"""
    n = source.num_qubits
    try:
        src_gates, src_measures, _ = _program(source)
        cmp_gates, cmp_measures, cmp_touched = _program(compiled.circuit)
    except NotTerminal as error:
        return Verdict("fail", "none", None, f"not checkable as written: {error}")

    if len(compiled.initial) != n or len(compiled.final) != n:
        return Verdict("fail", "none", None, "layouts do not cover every logical qubit")

    # Measurements: same classical bits, read where each logical qubit ends.
    expected_measures = {c: compiled.final[q] for c, q in src_measures.items()}
    if expected_measures != cmp_measures:
        return Verdict(
            "fail", "none", None,
            f"measurement map differs: expected {expected_measures}, got {cmp_measures}",
        )

    if n > state_max_qubits:
        return Verdict("not-checked", "none", None, f"{n} qubits exceeds the state-check limit")
    active = sorted(set(cmp_touched) | set(compiled.initial) | set(compiled.final))
    width = len(active)
    if width > MAX_SIMULATED_WIDTH:
        return Verdict("not-checked", "none", None, f"{width} active qubits exceeds the simulator bound")
    compact = {q: i for i, q in enumerate(active)}
    src_compact = {q: q for q in range(n)}
    start = [compact[p] for p in compiled.initial]
    end = [compact[p] for p in compiled.final]

    operator = n <= operator_max_qubits
    all_measured = set(src_measures.values()) == set(range(n))
    inputs_all = np.arange(1 << n) if operator else np.zeros(1, dtype=np.int64)
    embed = _place(np.arange(1 << n), end)

    batch = max(1, _BATCH_AMPLITUDES // max(1 << width, 1 << n))
    trace = 0j
    # d[j] = (U_c U_s^dagger)_jj accumulated over inputs (operator mode).
    rows = np.zeros(1 << n, dtype=complex)
    zero_expected = zero_actual = None
    zero_leak = 0.0
    for lo in range(0, len(inputs_all), batch):
        inputs = inputs_all[lo : lo + batch]
        expected = _run(src_gates, src_compact, n, inputs)
        actual_full = _run(cmp_gates, compact, width, _place(inputs, start))
        actual = actual_full[:, embed]
        trace += np.einsum("bj,bj->", np.conj(actual), expected)
        rows += np.einsum("bj,bj->j", actual, np.conj(expected))
        if lo == 0:
            zero_expected, zero_actual = expected[0], actual[0]
            # Probability left outside the logical subspace (ancillas not
            # returned to |0>), for the all-zeros input.
            zero_leak = 1.0 - float(np.sum(np.abs(actual[0]) ** 2))

    if operator:
        residual = abs(abs(trace) / (1 << n) - 1.0)
        if residual < tolerance:
            return Verdict("pass", "operator", float(residual))
        if all_measured:
            row_residual = float(np.max(np.abs(1.0 - np.abs(rows))))
            if row_residual < tolerance:
                return Verdict("pass", "operator-mod-measured-phase", row_residual)
            return Verdict("fail", "operator-mod-measured-phase", row_residual)
        return Verdict("fail", "operator", float(residual))

    residual = abs(abs(np.vdot(zero_actual, zero_expected)) - 1.0)
    if residual < tolerance:
        return Verdict("pass", "state-from-zero", float(residual))
    if all_measured:
        magnitude = float(np.max(np.abs(np.abs(zero_actual) - np.abs(zero_expected))))
        worst = max(magnitude, abs(zero_leak))
        status = "pass" if worst < tolerance else "fail"
        return Verdict(status, "distribution-from-zero", worst)
    return Verdict("fail", "state-from-zero", float(residual))
