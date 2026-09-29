"""Validate OQCI's emitted QIR with PyQIR, an external QIR tool.

Usage::

    OQCI_QIR_CORPUS_DIR=qir-corpus cargo test --test qir_conformance
    python scripts/validate_qir.py qir-corpus/opaque

Every ``*.ll`` file in the directory is parsed and verified by PyQIR
(``pyqir.Module.from_ir`` and ``Module.verify``), and the QIR metadata PyQIR
exposes is read back: the entry point, its ``required_num_qubits`` /
``required_num_results`` attributes, and the ``qir_major_version`` and
``dynamic_*_management`` module flags.

What this proves: the module is well-formed LLVM IR as PyQIR's LLVM reads it,
and its QIR metadata is readable and has the values OQCI intends. It does
**not** prove QIR Base Profile conformance; OQCI's own structural checks
(``tests/qir_conformance.rs``) cover that, and they are not independent.

Pinned to PyQIR 0.12.5 (``python/requirements-qir.txt``). It is run on both
the ``opaque`` and the ``typed`` directory: PyQIR 0.12.5 builds on LLVM 18-20,
whose IR parser accepts typed-pointer syntax as well as ``ptr``.
``llvm-as`` runs on the same files in CI (see ``.github/workflows/ci.yml``).
"""

from __future__ import annotations

import sys
from importlib.metadata import version as installed_version
from pathlib import Path

import pyqir

EXPECTED_PYQIR = "0.12.5"


def check(path: Path) -> list[str]:
    """Every problem PyQIR finds with one module; empty if it is clean."""
    problems: list[str] = []
    try:
        module = pyqir.Module.from_ir(pyqir.Context(), path.read_text(encoding="utf-8"), path.stem)
    except Exception as error:  # PyQIR raises a plain error with LLVM's message.
        return [f"does not parse: {error}"]

    failure = module.verify()
    if failure is not None:
        problems.append(f"does not verify: {failure}")

    if pyqir.qir_major_version(module) != 1:
        problems.append(f"qir_major_version is {pyqir.qir_major_version(module)}, expected 1")
    if pyqir.qir_minor_version(module) != 0:
        problems.append(f"qir_minor_version is {pyqir.qir_minor_version(module)}, expected 0")
    if pyqir.dynamic_qubit_management(module) is not False:
        problems.append("dynamic_qubit_management is not false")
    if pyqir.dynamic_result_management(module) is not False:
        problems.append("dynamic_result_management is not false")

    entry_points = [f for f in module.functions if pyqir.is_entry_point(f)]
    if len(entry_points) != 1:
        problems.append(f"expected one entry point, found {len(entry_points)}")
    else:
        entry = entry_points[0]
        if pyqir.required_num_qubits(entry) is None:
            problems.append("entry point has no readable required_num_qubits")
        if pyqir.required_num_results(entry) is None:
            problems.append("entry point has no readable required_num_results")
    return problems


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__)
        return 2
    version = installed_version("pyqir")
    if version != EXPECTED_PYQIR:
        print(f"expected PyQIR {EXPECTED_PYQIR}, found {version}")
        return 2

    files = sorted(Path(argv[1]).glob("*.ll"))
    if not files:
        print(f"no .ll files in {argv[1]}")
        return 2

    failures = 0
    for path in files:
        problems = check(path)
        status = "ok" if not problems else "FAIL"
        print(f"{status:4}  {path.name}")
        for problem in problems:
            print(f"        {problem}")
        failures += bool(problems)

    print(f"\nPyQIR {version}: {len(files) - failures}/{len(files)} module(s) accepted")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
