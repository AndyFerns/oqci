"""Execution adapters.

The Rust compiler prepares executables; it does not run them. Every shipped
`Backend::execute` returns a typed "not available in this process" error, and
these adapters are what that error points at.

- `oqci.backends.aer` runs a prepared executable on Qiskit Aer.
- `oqci.backends.cudaq` runs one on CUDA-Q.

Both take the same `executable` dict, expose `run(executable, *, shots, seed,
...)` plus a runtime-free translation (`to_qiskit`, `to_cudaq_source`), raise
the same `UnsupportedOperation`, and return counts in one key convention
(clbit 0 rightmost) — see `_common.py` and `docs/adapters.md`.

There is deliberately no IBM adapter. Submission needs `qiskit-ibm-runtime`
and account credentials, neither of which this project has, and writing an
untested submission path between a verified circuit and real hardware would be
worse than an explicit gap. The Rust side does everything up to the executable
and validates it against the supplied target description; no claim is made
that it will run on any real device.
"""

from __future__ import annotations

from ._common import RunResult, UnsupportedOperation

__all__ = ["RunResult", "UnsupportedOperation", "aer", "cudaq"]

_ADAPTERS = ("aer", "cudaq")


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
