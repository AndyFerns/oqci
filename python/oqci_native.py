"""Compatibility shim for the original flat import path.

The compiled extension moved to ``oqci._native`` when the package gained a
pure-Python layer: maturin's mixed layout requires the extension to live under
the package it ships with. Nothing about the API changed, so this module keeps
``import oqci_native`` working for code and documentation written against the
older layout.

New code should prefer ``import oqci``, which is the SDK surface §17 describes.
"""

from __future__ import annotations

from oqci._native import (  # noqa: F401
    OqciError,
    __version__,
    backends,
    compile_qasm3,
    decomposition_rules,
    qasm3_to_qir,
    qiskit_parameters,
    qiskit_to_qir,
)

__all__ = [
    "OqciError",
    "__version__",
    "backends",
    "compile_qasm3",
    "decomposition_rules",
    "qasm3_to_qir",
    "qiskit_parameters",
    "qiskit_to_qir",
]
