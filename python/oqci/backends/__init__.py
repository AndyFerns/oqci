"""Execution adapters.

The Rust compiler prepares executables; it does not run them. Every shipped
`Backend::execute` returns a typed "not available in this process" error, and
these adapters are what that error points at.

- `oqci.backends.aer` runs a prepared executable on Qiskit Aer.

There is deliberately no IBM adapter. Submission needs `qiskit-ibm-runtime`
and account credentials, neither of which this project has, and writing an
untested submission path between a verified circuit and real hardware would be
worse than an explicit gap. The Rust side does everything up to the executable
and validates it against the supplied target description; no claim is made
that it will run on any real device.
"""

from __future__ import annotations

__all__ = ["aer"]


def __getattr__(name: str):
    # Imported lazily so `import oqci` works without Aer installed: the
    # compiler is the point, and an execution adapter is optional.
    #
    # `importlib` rather than `from . import aer`, which re-enters this
    # function and recurses until the stack gives out.
    if name == "aer":
        import importlib

        return importlib.import_module(".aer", __name__)
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
