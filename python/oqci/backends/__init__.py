"""Execution adapters.

The Rust compiler prepares executables; it does not run them. Every shipped
`Backend::execute` returns a typed "not available in this process" error, and
these adapters are what that error points at.

- `oqci.backends.aer` runs a prepared executable on Qiskit Aer.
- `oqci.backends.cudaq` runs one on CUDA-Q.
- `oqci.backends.ibm` submits one to IBM Quantum hardware (CZ-native,
  Heron-family devices) through `qiskit-ibm-runtime`, after checking it is
  already in the device's instruction set: nothing is transpiled. Compile for
  `ibm.target_from_backend(backend)` so lowering targets the real device.
  Credentials come only from the caller's saved account or environment.

All take the same `executable` dict, expose `run(executable, *, shots, seed,
...)` plus a runtime-free translation (`to_qiskit`, `to_cudaq_source`), raise
the same `UnsupportedOperation`, and return counts in one key convention
(clbit 0 rightmost) — see `_common.py` and `docs/adapters.md`.

`to_execution_result` turns any adapter's result into OQCI's structured
`ExecutionResult` (`final-deliverables-spec.md` §10).
"""

from __future__ import annotations

import dataclasses
import json
from typing import Any, Mapping, Optional

from ._common import RunResult, UnsupportedOperation

__all__ = [
    "RunResult",
    "UnsupportedOperation",
    "aer",
    "cudaq",
    "ibm",
    "to_execution_result",
]

_ADAPTERS = ("aer", "cudaq", "ibm")

#: Provenance fields an adapter legitimately rewrites to describe the run
#: that happened (``docs/adapters.md``).
_RUN_FIELDS = ("shots", "seed")


def to_execution_result(
    run: RunResult, artifacts: Optional[Mapping[str, Any]] = None
) -> dict[str, Any]:
    """Convert an adapter's :class:`RunResult` into OQCI's ``ExecutionResult``.

    The conversion is done by the compiler (``ExecutionResult::from_adapter``),
    so the result has exactly the Rust schema: counts, memory, metadata,
    and three separate durations — compilation, queue and execution — none
    derived from another (Stage C §8).

    ``artifacts`` is the ``oqci.compile`` result that produced the
    executable. When given, its measured compile time is attached, but only
    after checking that the run's provenance is that compilation's (apart
    from the shots and seed a run may override). A mismatch raises rather
    than attaching a time from a different compilation.
    """
    from .. import OqciError
    from .. import _native

    payload = dataclasses.asdict(run)
    compilation_ms = None
    if artifacts is not None:
        compiled = dict((artifacts.get("executable") or {}).get("provenance") or {})
        ran = dict(payload.get("provenance") or {})
        for key in _RUN_FIELDS:
            compiled.pop(key, None)
            ran.pop(key, None)
        if compiled != ran:
            raise OqciError(
                "this result's provenance does not match the given compilation; "
                "pass the artifacts that produced the executable"
            )
        total_us = (artifacts.get("timings") or {}).get("total_us")
        compilation_ms = None if total_us is None else total_us / 1000.0
    return _native.execution_result(json.dumps(payload), compilation_ms)


def __getattr__(name: str):
    # Imported lazily so `import oqci` works without Aer or CUDA-Q installed:
    # the compiler is the point, and an execution adapter is optional. (Each
    # adapter also defers importing its runtime until it runs.)
    #
    # `importlib` rather than `from . import aer`, which re-enters this
    # function and recurses until the stack gives out.
    if name in _ADAPTERS:
        import importlib

        return importlib.import_module(f".{name}", __name__)
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
