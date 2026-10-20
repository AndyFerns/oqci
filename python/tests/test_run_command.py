"""``python -m oqci run``: compile and execute in one command."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

pytest.importorskip("qiskit_aer")

ROOT = Path(__file__).resolve().parents[2]
BELL = ROOT / "examples" / "bell.qasm"


def run(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run([sys.executable, "-m", "oqci", "run", *args], capture_output=True,
                          text=True, check=False)


def test_run_on_aer_returns_an_execution_result():
    done = run(str(BELL), "--backend", "simulator-nisq", "--shots", "500", "--seed", "3", "--json")
    assert done.returncode == 0, done.stderr
    result = json.loads(done.stdout)
    assert sum(result["counts"].values()) == 500
    assert set(result["counts"]) <= {"00", "11"}
    assert result["provenance"]["backend_id"] == "simulator-nisq"
    assert result["provenance"]["seed"] == 3
    assert result["compilation_duration_ms"] is not None
    assert result["submission_duration_ms"] is None


def test_run_with_a_target_file_and_verification():
    fixture = ROOT / "tests" / "fixtures" / "heron_style_synthetic.json"
    done = run(str(BELL), "--target-file", str(fixture), "--shots", "200", "--verify")
    assert done.returncode == 0, done.stderr
    assert "heron-style-synthetic" in done.stdout


def test_run_on_a_fake_ibm_backend_offline():
    pytest.importorskip("qiskit_ibm_runtime")
    done = run(str(BELL), "--runtime", "ibm", "--fake", "FakeFez", "--shots", "300", "--json")
    assert done.returncode == 0, done.stderr
    result = json.loads(done.stdout)
    assert sum(result["counts"].values()) == 300
    assert result["provenance"]["backend_id"] == "fake_fez"


def test_hardware_cannot_be_seeded():
    done = run(str(BELL), "--runtime", "ibm", "--fake", "FakeFez", "--seed", "1")
    assert done.returncode != 0
    assert "cannot be seeded" in done.stderr
