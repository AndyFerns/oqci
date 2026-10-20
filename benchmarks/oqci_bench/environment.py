"""What a result must be traceable to (§22): code, versions, machine.

Captured once per run and written next to the records, so a result can be
reconstructed from a repository commit and the exact dependency versions.
Nothing secret is recorded: no environment variable other than the
thread-count ones that affect timing.
"""

from __future__ import annotations

import os
import platform
import subprocess
import sys
from importlib import metadata
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[2]

_PACKAGES = (
    "oqci",
    "qiskit",
    "qiskit-aer",
    "qiskit-ibm-runtime",
    "pytket",
    "pytket-qiskit",
    "pyqir",
    "numpy",
    "scipy",
    "matplotlib",
    "cudaq",
)

_THREAD_VARIABLES = (
    "RAYON_NUM_THREADS",
    "OMP_NUM_THREADS",
    "QISKIT_PARALLEL",
    "QISKIT_NUM_PROCS",
    "MKL_NUM_THREADS",
    "OPENBLAS_NUM_THREADS",
)


def _run(command: list[str]) -> str:
    try:
        return subprocess.run(
            command, cwd=REPO, capture_output=True, text=True, timeout=30, check=False
        ).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return ""


def git_commit() -> str:
    return _run(["git", "rev-parse", "HEAD"]) or "unknown"


def git_dirty() -> bool:
    return bool(_run(["git", "status", "--porcelain", "--untracked-files=no"]))


def capture() -> dict[str, Any]:
    versions = {}
    for package in _PACKAGES:
        try:
            versions[package] = metadata.version(package)
        except metadata.PackageNotFoundError:
            versions[package] = None
    oqci_identity: dict[str, Any] = {}
    try:
        import oqci

        oqci_identity["version"] = oqci.__version__
        probe = oqci.compile(
            "qubit[1] q; bit[1] c; h q[0]; c[0] = measure q[0];", backend="simulator"
        )
        oqci_identity["compiled_from_commit"] = probe["executable"]["provenance"]["git_commit"]
    except Exception as error:  # noqa: BLE001
        oqci_identity["error"] = str(error)
    return {
        "repository_commit": git_commit(),
        "repository_dirty": git_dirty(),
        "oqci": oqci_identity,
        "python": sys.version,
        "platform": platform.platform(),
        "machine": platform.machine(),
        "processor": platform.processor(),
        "cpu_count": os.cpu_count(),
        "rustc": _run(["rustc", "-V"]),
        "packages": versions,
        "thread_environment": {k: os.environ.get(k) for k in _THREAD_VARIABLES},
    }
