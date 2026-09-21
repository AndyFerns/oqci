"""OQCI's Python SDK.

The compiler itself is Rust. This package is the layer §17 of the deliverables
specification asks for: circuit import, compiler invocation, configuration,
backend selection, analysis and result access, over the stable contracts the
`oqci_native` extension exposes.

Everything here delegates. No compilation decision is made in Python — the
same rule the CLI follows, and for the same reason: two implementations of the
pipeline would eventually disagree, and the one a user could see would be the
wrong one.

Typical use::

    import oqci

    artifacts = oqci.compile(source, backend="simulator-nisq")
    result = oqci.backends.aer.run(artifacts["executable"], shots=2048)
    print(result.counts)

Execution is deliberately not part of the compiler. The Rust `Backend` trait's
``execute`` returns a typed "not available in this process" error for every
shipped backend, because the project's non-goals rule out writing a simulator
to satisfy it. :mod:`oqci.backends.aer` is where a prepared executable
actually runs.
"""

from __future__ import annotations

from typing import Any, Mapping, Optional

from . import _native as oqci_native
from . import backends  # noqa: F401  (re-exported for `oqci.backends.aer`)

__version__ = oqci_native.__version__

#: Raised for every compiler-side failure, with the message OQCI wrote.
OqciError = oqci_native.OqciError

__all__ = [
    "OqciError",
    "__version__",
    "available_backends",
    "backends",
    "compile",
    "decomposition_rules",
    "qasm3_to_qir",
    "qiskit_parameters",
    "qiskit_to_qir",
]


def compile(  # noqa: A001  — mirrors the Rust entry point's name deliberately
    source: str,
    *,
    backend: Optional[str] = None,
    bindings: Optional[Mapping[str, float]] = None,
    name: str = "main",
    shots: int = 1024,
    seed: Optional[int] = None,
    layout: str = "trivial",
) -> dict[str, Any]:
    """Compile OpenQASM 3 source, optionally for a specific backend.

    With no ``backend``, compilation is target-independent and stops after
    optimization — a complete result, not a degraded one, since there is
    nothing to lower *to*.

    With one, the result also carries ``lowering`` (layouts, SWAP count, the
    rules that fired, and whether the output is legal), ``cost`` as the
    *target* assigns it, and ``executable``: the artifact an execution adapter
    replays.

    :param bindings: values for symbolic parameters. OQCI never invents one —
        a circuit with a free parameter and no binding is refused rather than
        run with a guess.
    :param layout: ``"trivial"`` or ``"dense"``. Layout affects how many SWAPs
        routing needs and nothing else; it cannot make a circuit incorrect.
    :param seed: recorded in the provenance, never chosen here.
    :raises OqciError: if any stage refuses, with the compiler's own message.
    """
    return oqci_native.compile_qasm3(
        source,
        backend,
        dict(bindings) if bindings else None,
        name,
        shots,
        seed,
        layout,
    )


def available_backends() -> list[tuple[str, str]]:
    """The backends this build can compile for, as ``(id, description)``.

    None of them executes in the compiler process; see :mod:`oqci.backends`.
    """
    return oqci_native.backends()


def decomposition_rules() -> list[dict[str, Any]]:
    """The decomposition-rule library, with each rule's declared exactness.

    Exported so the identities can be re-checked against an independent
    implementation of gate semantics — which is what
    ``python/tests/test_rules.py`` does with Qiskit's ``quantum_info``.
    """
    return oqci_native.decomposition_rules()


def qasm3_to_qir(source: str, bindings: Optional[Mapping[str, float]] = None) -> str:
    """Compile OpenQASM 3 to textual QIR.

    QIR is a lowering artifact, **not** an execution guarantee: Stage C §5 is
    explicit that emitted QIR does not by itself mean a circuit will run on
    hardware. Use :func:`compile` with a backend for something executable.
    """
    return oqci_native.qasm3_to_qir(source, dict(bindings) if bindings else None)


def qiskit_parameters(circuit: Any) -> list[str]:
    """A ``QuantumCircuit``'s free parameters, as OQCI sees them, sorted.

    Worth cross-checking against Qiskit's own view: a name Qiskit reports and
    this does not means the parameter reached OQCI inside a compound
    expression, which the IR cannot represent. Better to find that out here
    than to discover a binding silently did nothing.
    """
    return oqci_native.qiskit_parameters(circuit)


def qiskit_to_qir(circuit: Any, bindings: Optional[Mapping[str, Any]] = None) -> str:
    """Compile a Qiskit ``QuantumCircuit`` to textual QIR.

    See :func:`qasm3_to_qir` on what QIR does and does not promise.
    """
    return oqci_native.qiskit_to_qir(circuit, dict(bindings) if bindings else None)
