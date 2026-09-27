"""Running an OQCI executable on CUDA-Q.

The execution half of §5.5/§15.2's CUDA-Q adapter, alongside
:mod:`oqci.backends.aer`. (The frontend half — CUDA-Q kernel text into
QC-IR — is in Rust, ``src/frontend/cudaq/``.) It relocates and rewrites the
execution half of the ``cudaq-adapter/`` prototype; see
``docs/adapters.md`` for what changed and why.

What goes in, what comes out
----------------------------

In: the ``executable`` dict from ``oqci.compile(..., backend=...)`` or
``oqci prepare`` — a target-validated operation list, the same artifact
:func:`oqci.backends.aer.run` replays. Out: a :class:`CudaQResult` whose
counts use the shared key convention (clbit 0 rightmost), so they compare
key by key with Aer's.

How the kernel is built
-----------------------

:func:`to_cudaq_source` writes a ``@cudaq.kernel`` function using only the
operations NVIDIA's CUDA-Q documentation shows in Python — the same subset
``docs/cudaq_frontend.md`` lists, checked against the ``latest`` docs on
2026-09-26/27 rather than recalled (§33.4). Because it is that subset, the
generated source is also valid input to OQCI's own CUDA-Q frontend, and the
test suite compiles it back to prove the translation preserves the circuit's
unitary — without needing CUDA-Q installed.

:func:`run` writes that source to a temporary ``.py`` file and imports it,
exactly as a user's own kernel would be. That, rather than ``exec`` on a
string (what the prototype did), matters because CUDA-Q's kernel decorator
works from the function's *source*, and code ``exec``'d from a string has
no source file to read.

What is refused
---------------

Anything this module cannot map exactly. ``reset`` has no documented Python
form. A gate after a qubit's measurement, or measuring a qubit twice, would
change meaning: CUDA-Q's default sampling reports each measured qubit's
value *at the end of the kernel*. Two measurements into one classical bit
have no single answer. Each raises :class:`UnsupportedOperation`.

Verification status
-------------------

No CUDA-Q installation was available where this was written (a Windows
machine), so :func:`run` itself has been exercised
against a stand-in module, not the real runtime. The translation, the
refusals and the bit-order conversion are tested for real; the first run on
a real CUDA-Q install should be checked against :mod:`oqci.backends.aer` on
the same executable before its numbers are trusted.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import math
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Iterable, Mapping, Optional, Sequence

from ._common import RunResult, UnsupportedOperation

__all__ = [
    "CudaQResult",
    "UnsupportedOperation",
    "counts_from_cudaq",
    "main",
    "run",
    "to_cudaq_source",
]

#: Documented CUDA-Q statement for each executable operation.
#:
#: ``{q0}``, ``{q1}`` … are the operand qubits; ``{p0}`` … the parameters.
#: Every template uses a form NVIDIA's CUDA-Q "Quantum Operations" page shows
#: in Python: ``s.adj``/``t.adj`` for the inverses (``sdg``/``tdg`` do not
#: appear there), ``r1`` for the phase gate, ``u3(θ, φ, λ, q)`` — whose
#: documented matrix is Qiskit's ``U(θ, φ, λ)`` — and ``.ctrl`` with a list for
#: the Toffoli.
_TEMPLATES: dict[str, str] = {
    "x": "x({q0})",
    "y": "y({q0})",
    "z": "z({q0})",
    "h": "h({q0})",
    "s": "s({q0})",
    "sdg": "s.adj({q0})",
    "t": "t({q0})",
    "tdg": "t.adj({q0})",
    # `sx` does not appear in CUDA-Q's documentation. SX = e^{iπ/4}·RX(π/2),
    # and a global phase changes no measurement probability, so the rotation
    # samples identically. (It would not be exact under a control; nothing
    # here controls it.)
    "sx": "rx(1.5707963267948966, {q0})",
    "sxdg": "rx(-1.5707963267948966, {q0})",
    "rx": "rx({p0}, {q0})",
    "ry": "ry({p0}, {q0})",
    "rz": "rz({p0}, {q0})",
    "p": "r1({p0}, {q0})",
    "u": "u3({p0}, {p1}, {p2}, {q0})",
    "cx": "x.ctrl({q0}, {q1})",
    "cy": "y.ctrl({q0}, {q1})",
    "cz": "z.ctrl({q0}, {q1})",
    "swap": "swap({q0}, {q1})",
    "ccx": "x.ctrl([{q0}, {q1}], {q2})",
}

#: Operations with no effect on what is sampled, emitted as nothing.
_IDENTITIES = frozenset({"id"})

#: Why each known-but-refused operation is refused.
_REFUSED: dict[str, str] = {
    "reset": "CUDA-Q's documentation shows no Python form of reset, so "
    "there is nothing verified to emit",
}


class CudaQResult(RunResult):
    """What CUDA-Q returned, converted to the shared key convention.

    The fields are :class:`oqci.backends._common.RunResult`'s. ``counts`` has
    already been re-keyed from CUDA-Q's order (first allocated measured
    qubit leftmost) to clbit 0 rightmost; the raw keys are not kept, because
    two conventions in one result is how bit-order bugs happen.
    """


def _plan(executable: Mapping[str, Any]) -> tuple[list[str], list[tuple[int, int]]]:
    """Checks an executable and splits it into gates and measurements.

    Returns the gate statements in program order and the ``(qubit, clbit)``
    measurements sorted by qubit.

    :raises UnsupportedOperation: for anything :mod:`this module <oqci.backends.cudaq>`
        cannot map exactly — see the module docstring.
    """
    num_qubits = int(executable["num_qubits"])
    num_clbits = int(executable["num_clbits"])
    statements: list[str] = []
    measured: dict[int, int] = {}
    written: dict[int, int] = {}

    for index, op in enumerate(executable["ops"]):
        name = op["op"]
        qubits = [int(q) for q in op["qubits"]]
        params = [float(p) for p in op.get("params") or []]

        for qubit in qubits:
            if not 0 <= qubit < num_qubits:
                raise UnsupportedOperation(
                    f"operation {index}: qubit {qubit} is outside the "
                    f"executable's {num_qubits} qubits"
                )
            if qubit in measured:
                raise UnsupportedOperation(
                    f"operation {index}: `{name}` acts on qubit {qubit} after it "
                    "was measured. CUDA-Q's sampling reports a measured qubit's "
                    "value at the end of the kernel, so this would not mean what "
                    "it means on Aer"
                )

        if name == "measure":
            clbit = op.get("clbit")
            if clbit is None:
                raise UnsupportedOperation(
                    f"operation {index}: a measurement with no destination bit"
                )
            clbit = int(clbit)
            if not 0 <= clbit < num_clbits:
                raise UnsupportedOperation(
                    f"operation {index}: clbit {clbit} is outside the "
                    f"executable's {num_clbits} classical bits"
                )
            if clbit in written:
                raise UnsupportedOperation(
                    f"operation {index}: clbit {clbit} is written by qubits "
                    f"{written[clbit]} and {qubits[0]}; only the last write "
                    "survives, which CUDA-Q's result cannot express"
                )
            measured[qubits[0]] = clbit
            written[clbit] = qubits[0]
            continue

        if name in _IDENTITIES:
            continue
        if not all(math.isfinite(p) for p in params):
            raise UnsupportedOperation(f"operation {index}: `{name}` has a non-finite angle")
        if name in _REFUSED:
            raise UnsupportedOperation(f"operation {index}: `{name}` — {_REFUSED[name]}")
        template = _TEMPLATES.get(name)
        if template is None:
            raise UnsupportedOperation(
                f"operation {index}: `{name}` has no CUDA-Q equivalent in this adapter"
            )
        fields = {f"q{i}": f"q[{q}]" for i, q in enumerate(qubits)}
        # repr() of a float round-trips exactly, so the angle CUDA-Q receives
        # is bit-for-bit the one OQCI verified.
        fields.update({f"p{i}": repr(p) for i, p in enumerate(params)})
        try:
            statements.append(template.format(**fields))
        except KeyError as missing:
            raise UnsupportedOperation(
                f"operation {index}: `{name}` is missing operand {missing}"
            ) from None

    return statements, sorted(measured.items())


def to_cudaq_source(executable: Mapping[str, Any], *, kernel_name: str = "oqci_kernel") -> str:
    """The CUDA-Q kernel that runs ``executable``, as Python source.

    Needs no CUDA-Q install; useful on its own for inspecting what would run.
    Gates are emitted in program order and measurements last, one ``mz`` per
    measured qubit in ascending order. Moving a measurement to the end is
    exact here because :func:`_plan` refuses any gate after a qubit's
    measurement — so nothing that follows can touch the measured qubit.

    :raises UnsupportedOperation: if the executable cannot be mapped exactly.
    """
    if not kernel_name.isidentifier():
        raise ValueError(f"kernel name {kernel_name!r} is not a Python identifier")
    statements, measurements = _plan(executable)
    body = [f"q = cudaq.qvector({int(executable['num_qubits'])})"]
    body += statements
    body += [f"mz(q[{qubit}])" for qubit, _ in measurements]

    backend = executable.get("backend_id", "unknown")
    lines = [
        f"# Generated by oqci.backends.cudaq from an executable prepared for `{backend}`.",
        "import cudaq",
        "",
        "",
        "@cudaq.kernel",
        f"def {kernel_name}():",
    ]
    lines += [f"    {statement}" for statement in body]
    return "\n".join(lines) + "\n"


def counts_from_cudaq(
    items: Iterable[tuple[str, int]],
    measurements: Sequence[tuple[int, int]],
    num_clbits: int,
) -> dict[str, int]:
    """Re-keys CUDA-Q counts into the shared convention.

    CUDA-Q's documented order: when a kernel measures, only the measured
    qubits appear, and "the ``[0]`` element in the ``__global__`` bitstring
    corresponds with the first remaining declared qubit". With one
    ``qvector`` that is the lowest-indexed measured qubit — so character
    ``k`` belongs to ``measurements[k]`` (sorted by qubit). The shared
    convention puts clbit ``c`` at position ``num_clbits - 1 - c``; clbits
    nothing measured read ``0``, as on Aer.

    :param measurements: ``(qubit, clbit)`` pairs sorted by qubit.
    :raises RuntimeError: if a bitstring's length is not the number of
        measured qubits — the documented order would not apply, and guessing
        another would silently permute the results.
    """
    counts: dict[str, int] = {}
    for bits, count in items:
        bits = str(bits)
        if len(bits) != len(measurements):
            raise RuntimeError(
                f"CUDA-Q returned the bitstring {bits!r} for {len(measurements)} "
                "measured qubits; refusing to guess which bit is which"
            )
        key = ["0"] * num_clbits
        for character, (_, clbit) in zip(bits, measurements):
            key[num_clbits - 1 - clbit] = character
        joined = "".join(key)
        counts[joined] = counts.get(joined, 0) + int(count)
    return counts


def _import_cudaq():
    try:
        import cudaq
    except ImportError as exc:
        raise RuntimeError(
            "CUDA-Q is not installed (see NVIDIA's CUDA-Q install guide for the "
            "platforms it supports). Compiling needs no CUDA-Q; only running does."
        ) from exc
    return cudaq


def _load_kernel(source: str, directory: str, kernel_name: str):
    """Imports ``source`` from a real file, so the decorator can read it."""
    path = Path(directory) / f"{kernel_name}.py"
    path.write_text(source, encoding="utf-8")
    spec = importlib.util.spec_from_file_location(f"_oqci_cudaq_{kernel_name}", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load the generated kernel from {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return getattr(module, kernel_name)


def run(
    executable: Mapping[str, Any],
    *,
    shots: Optional[int] = None,
    seed: Optional[int] = None,
    target: Optional[str] = None,
    **target_options: Any,
) -> CudaQResult:
    """Execute an OQCI executable on CUDA-Q.

    ``shots`` and ``seed`` default to what the executable was prepared with,
    as in :func:`oqci.backends.aer.run`, and the returned provenance records
    the values actually used.

    :param target: passed to ``cudaq.set_target`` with ``target_options``.
        When omitted the target is left as CUDA-Q has it — this module does
        not pick one on the caller's behalf.
    :raises UnsupportedOperation: if the executable cannot be mapped exactly,
        or measures nothing (there would be no counts).
    :raises RuntimeError: if CUDA-Q is not installed, or returns bitstrings
        the documented ordering does not explain.
    """
    settings = executable.get("settings") or {}
    shots = int(settings.get("shots", 1024)) if shots is None else int(shots)
    if seed is None:
        seed = settings.get("seed")

    source = to_cudaq_source(executable)
    _, measurements = _plan(executable)
    if not measurements:
        raise UnsupportedOperation(
            "this executable has no measurement, so it would produce no counts. "
            "Add one to the source circuit."
        )

    cudaq = _import_cudaq()
    if target is not None:
        cudaq.set_target(target, **target_options)
    if seed is not None:
        cudaq.set_random_seed(int(seed))

    with tempfile.TemporaryDirectory(prefix="oqci-cudaq-") as directory:
        kernel = _load_kernel(source, directory, "oqci_kernel")
        started = time.perf_counter()
        sampled = cudaq.sample(kernel, shots_count=shots)
        elapsed_ms = (time.perf_counter() - started) * 1000.0

    counts = counts_from_cudaq(
        sampled.items(), measurements, int(executable["num_clbits"])
    )

    provenance = dict(executable.get("provenance") or {})
    provenance["shots"] = shots
    provenance["seed"] = seed
    metadata = {"runtime": "cudaq", "target": target}
    version = getattr(cudaq, "__version__", None)
    if version is not None:
        metadata["cudaq_version"] = str(version)

    return CudaQResult(
        counts=counts,
        backend_metadata=metadata,
        execution_duration_ms=elapsed_ms,
        provenance=provenance,
    )


def main(argv: Optional[Sequence[str]] = None) -> int:
    """``python -m oqci.backends.cudaq`` — the prototype's CLI, on executables.

    ``emit`` writes the kernel for an executable; ``run`` executes it. The
    executable comes from ``oqci prepare <program> --backend <id> -o exe.json``.
    (The prototype's ``frontend`` subcommand is ``oqci compile kernel.py``.)
    """
    parser = argparse.ArgumentParser(
        prog="python -m oqci.backends.cudaq",
        description="Run an OQCI executable on CUDA-Q.",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    emit = sub.add_parser("emit", help="write the CUDA-Q kernel for an executable")
    emit.add_argument("executable", help="executable JSON from `oqci prepare`")
    emit.add_argument("-o", "--output", help="write here instead of to stdout")

    execute = sub.add_parser("run", help="execute an executable on CUDA-Q")
    execute.add_argument("executable", help="executable JSON from `oqci prepare`")
    execute.add_argument("--shots", type=int)
    execute.add_argument("--seed", type=int)
    execute.add_argument("--target", help="CUDA-Q target; default: CUDA-Q's own")

    args = parser.parse_args(argv)
    executable = json.loads(Path(args.executable).read_text(encoding="utf-8"))

    if args.command == "emit":
        source = to_cudaq_source(executable)
        if args.output:
            Path(args.output).write_text(source, encoding="utf-8")
        else:
            sys.stdout.write(source)
        return 0

    try:
        result = run(executable, shots=args.shots, seed=args.seed, target=args.target)
    except RuntimeError as error:  # includes UnsupportedOperation
        print(f"error: {error}", file=sys.stderr)
        return 1
    json.dump(
        {
            "counts": result.counts,
            "observed_shots": result.observed_shots,
            "execution_duration_ms": result.execution_duration_ms,
            "backend_metadata": result.backend_metadata,
            "provenance": result.provenance,
        },
        sys.stdout,
        indent=2,
    )
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
