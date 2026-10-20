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

import json
from typing import Any, Mapping, Optional, Sequence, Union

from . import _native as oqci_native
from . import backends  # noqa: F401  (re-exported for `oqci.backends.aer`)

__version__ = oqci_native.__version__

#: Raised for every compiler-side failure, with the message OQCI wrote.
OqciError = oqci_native.OqciError

#: Text frontends ``compile`` accepts via ``frontend=``. A Qiskit
#: ``QuantumCircuit`` needs no ``frontend``: it is recognised by not being text.
FRONTENDS = ("openqasm3", "cudaq")

__all__ = [
    "FRONTENDS",
    "OqciError",
    "__version__",
    "available_backends",
    "backends",
    "basis_rule_sets",
    "compile",
    "cudaq_to_qir",
    "decomposition_rules",
    "lint",
    "load_target",
    "qasm3_to_qir",
    "qiskit_parameters",
    "qiskit_to_qir",
]

#: A target description: a basis-profile dict, or its JSON text.
Target = Union[Mapping[str, Any], str]


def _target_json(target: Optional[Target]) -> Optional[str]:
    if target is None:
        return None
    if isinstance(target, str):
        return target
    return json.dumps(dict(target))


def compile(  # noqa: A001  — mirrors the Rust entry point's name deliberately
    program: Any,
    *,
    frontend: Optional[str] = None,
    backend: Optional[str] = None,
    bindings: Optional[Mapping[Any, float]] = None,
    name: Optional[str] = None,
    shots: int = 1024,
    seed: Optional[int] = None,
    layout: str = "trivial",
    passes: Optional[Sequence[str]] = None,
    disable: Optional[Sequence[str]] = None,
    qir_profile: str = "any",
    qir_pointers: str = "opaque",
    target: Optional[Target] = None,
    target_kind: Optional[str] = None,
    verify: bool = False,
    verify_max_qubits: int = 8,
    lint: bool = False,
) -> dict[str, Any]:
    """Compile a program through the whole pipeline, optionally for a backend.

    ``program`` is either source text — OpenQASM 3 by default, or a CUDA-Q
    kernel with ``frontend="cudaq"`` — or a Qiskit ``QuantumCircuit``. All
    three reach the same pipeline: a Qiskit circuit is no longer limited to
    QIR, and gets lowering, an executable and provenance like any other.

    With no ``backend``, compilation is target-independent and stops after
    optimization — a complete result, not a degraded one, since there is
    nothing to lower *to*.

    With one, the result also carries ``lowering`` (layouts, SWAP count, the
    rules that fired, the step-by-step schedule and any violations),
    ``lowered_circuit``, ``cost`` as the *target* assigns it, and
    ``executable``: the artifact an execution adapter replays. The
    ``passes``, ``lowering`` and circuit entries use the same schema as the
    CLI's ``--json`` report.

    :param frontend: ``"openqasm3"`` (default) or ``"cudaq"`` for text. Must
        be omitted (or ``"qiskit"``) for a ``QuantumCircuit``.
    :param bindings: values for symbolic parameters. OQCI never invents one —
        a circuit with a free parameter and no binding is refused rather than
        run with a guess. For a Qiskit circuit, keys may be ``Parameter``
        objects.
    :param name: circuit name for provenance. Defaults to ``"main"`` for
        text and to the circuit's own ``name`` for a ``QuantumCircuit``.
    :param layout: ``"trivial"`` or ``"dense"``. Layout affects how many SWAPs
        routing needs and nothing else; it cannot make a circuit incorrect.
    :param passes: run only these optimization passes.
    :param disable: run every pass except these — the shape of an ablation.
        Mutually exclusive with ``passes``; unknown ids are refused.
    :param seed: recorded in the provenance, never chosen here.
    :param qir_profile: ``"any"`` (default) labels the result's QIR
        ``base_profile`` when the circuit qualifies and ``custom`` otherwise;
        ``"base"`` raises instead of producing a ``custom`` module.
    :param qir_pointers: ``"opaque"`` (default, ``ptr``) or ``"typed"``
        (``%Qubit*``).
    :param target: compile for this device description instead of a named
        ``backend``: a basis-profile dict (or its JSON), such as one built
        from a real device by :func:`oqci.backends.ibm.target_from_backend`.
        It is validated — rule closure, cost model — before anything
        compiles. Mutually exclusive with ``backend``.
    :param target_kind: the backend kind to load ``target`` as (``"ibm"`` or
        ``"simulator"``); defaults to the profile's own ``backend_id``.
    :param verify: check that binding, every optimization pass and lowering
        preserved the circuit's semantics, refusing to produce an executable
        if one did not. The verdicts are in ``verification``.
    :param verify_max_qubits: the widest circuit ``verify`` simulates; wider
        ones are reported inconclusive, never passed.
    :param lint: include lint findings on the source circuit, against the
        selected target when there is one, in ``lint``.
    :raises OqciError: if any stage refuses, with the compiler's own message.

    ``timings`` holds per-stage compile times in microseconds. They are
    measurements, never part of a deterministic comparison.

    The result's ``qir`` entry is the compilation's one canonical QIR module:
    ``{"text", "profile", "stage", "pointers", "base_violations"}``, emitted
    from the lowered circuit when there is one and the optimized circuit
    otherwise, or ``None`` if parameters are still unbound.
    """
    bound = dict(bindings) if bindings else None
    passes = list(passes) if passes is not None else None
    disable = list(disable) if disable is not None else None

    if isinstance(program, str):
        return oqci_native.compile_source(
            program,
            frontend or "openqasm3",
            backend,
            bound,
            name or "main",
            shots,
            seed,
            layout,
            passes,
            disable,
            qir_profile,
            qir_pointers,
            _target_json(target),
            target_kind,
            verify,
            verify_max_qubits,
            lint,
        )

    if frontend not in (None, "qiskit"):
        raise OqciError(
            f"frontend {frontend!r} reads source text, but a {type(program).__name__} "
            "was given; pass the source as a string, or omit `frontend` for a "
            "Qiskit QuantumCircuit"
        )
    return oqci_native.compile_qiskit(
        program,
        backend,
        bound,
        name,
        shots,
        seed,
        layout,
        passes,
        disable,
        qir_profile,
        qir_pointers,
        _target_json(target),
        target_kind,
        verify,
        verify_max_qubits,
        lint,
    )


def lint(
    program: Any,
    *,
    frontend: Optional[str] = None,
    backend: Optional[str] = None,
    target: Optional[Target] = None,
    target_kind: Optional[str] = None,
    bindings: Optional[Mapping[Any, float]] = None,
    optimized: bool = False,
) -> dict[str, Any]:
    """Run the lint rules over a program (§13.5).

    ``program`` is source text or a Qiskit ``QuantumCircuit``. Rules that
    concern a target run only when ``backend`` or ``target`` names one.
    Returns ``{"rules": [...], "diagnostics": [...]}``; each diagnostic has
    ``rule``, ``severity`` (``info``/``warning``/``error``),
    ``instructions``, ``qubits`` and ``message``. Lint never changes a
    circuit.
    """
    if not isinstance(program, str) and frontend not in (None, "qiskit"):
        raise OqciError("`frontend` applies to source text only")
    return oqci_native.lint_program(
        program,
        frontend or "openqasm3",
        backend,
        _target_json(target),
        target_kind,
        dict(bindings) if bindings else None,
        optimized,
    )


def load_target(target: Target, kind: Optional[str] = None) -> dict[str, Any]:
    """Validate a target description and return it as OQCI reads it.

    Returns ``{"id", "description", "qualified_id", "profile"}``. Raises
    :class:`OqciError` if the profile is malformed, names an unknown cost
    model, or has decomposition rules that do not reach its own basis.
    """
    return oqci_native.load_target(_target_json(target), kind)


def basis_rule_sets() -> dict[str, list[str]]:
    """The complete decomposition-rule sets for the two superconducting bases.

    ``{"cx": [...], "cz": [...]}``: the rules a profile with basis
    ``{rz, sx, x, cx}`` or ``{rz, sx, x, cz}`` lists so that every registered
    gate can be lowered onto it.
    """
    return oqci_native.basis_rule_sets()


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


def qasm3_to_qir(
    source: str,
    bindings: Optional[Mapping[str, float]] = None,
    *,
    backend: Optional[str] = None,
    qir_profile: str = "any",
    qir_pointers: str = "opaque",
) -> str:
    """Compile OpenQASM 3 to textual QIR 1.0.

    The circuit goes through the same pipeline as :func:`compile`: it is
    optimized, and lowered too when ``backend`` is given, and the text returned
    is the compilation's canonical QIR module. It is labeled ``base_profile``
    only when the circuit qualifies for the QIR Base Profile, ``custom``
    otherwise; ``backend="qir-base"`` lowers any supported circuit into the
    Base instruction set first, and ``qir_profile="base"`` raises instead of
    returning a ``custom`` module. See ``docs/qir_lowering.md``.

    QIR is an interoperability artifact, **not** an execution guarantee:
    Stage C §5 is explicit that emitted QIR does not by itself mean a circuit
    will run on hardware. Use :func:`compile` with a backend for something
    executable.
    """
    return oqci_native.qasm3_to_qir(
        source,
        dict(bindings) if bindings else None,
        backend,
        qir_profile,
        qir_pointers,
    )


def cudaq_to_qir(
    source: str,
    bindings: Optional[Mapping[str, float]] = None,
    *,
    backend: Optional[str] = None,
    qir_profile: str = "any",
    qir_pointers: str = "opaque",
) -> str:
    """Compile a CUDA-Q kernel's source text to textual QIR.

    Reads the ``@cudaq.kernel`` function out of Python *source* — CUDA-Q
    itself need not be installed. The supported subset is in
    ``docs/cudaq_frontend.md``. See :func:`qasm3_to_qir` on what QIR does and
    does not promise.
    """
    return oqci_native.cudaq_to_qir(
        source,
        dict(bindings) if bindings else None,
        backend,
        qir_profile,
        qir_pointers,
    )


def qiskit_parameters(circuit: Any) -> list[str]:
    """A ``QuantumCircuit``'s free parameters, as OQCI sees them, sorted.

    Worth cross-checking against Qiskit's own view: a name Qiskit reports and
    this does not means the parameter reached OQCI inside a compound
    expression, which the IR cannot represent. Better to find that out here
    than to discover a binding silently did nothing.
    """
    return oqci_native.qiskit_parameters(circuit)


def qiskit_to_qir(
    circuit: Any,
    bindings: Optional[Mapping[str, Any]] = None,
    *,
    backend: Optional[str] = None,
    qir_profile: str = "any",
    qir_pointers: str = "opaque",
) -> str:
    """Compile a Qiskit ``QuantumCircuit`` to textual QIR 1.0.

    See :func:`qasm3_to_qir` on the options and on what QIR does and does not
    promise.
    """
    return oqci_native.qiskit_to_qir(
        circuit,
        dict(bindings) if bindings else None,
        backend,
        qir_profile,
        qir_pointers,
    )
