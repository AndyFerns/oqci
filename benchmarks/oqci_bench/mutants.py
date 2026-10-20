"""Mutation evidence (Stage G §3.1).

For each mutant in ``benchmarks/mutants/mutants.toml``: create a throwaway
git worktree of HEAD, apply the mutant's exact text replacement, build, run
every checker, record which caught it, and remove the worktree. The working
tree is never touched.

A checker "catches" a mutant when it exits nonzero (for the in-compiler
verifier: when any probe compilation reports a semantic mismatch). A mutant
that does not build is a bug in the mutant, reported as such — it is never
counted as caught. A mutant no checker catches is reported as surviving.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import tomllib
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[2]


def _git(*args: str, cwd: Path = REPO) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, check=False)


def _apply(worktree: Path, mutant: dict) -> None:
    path = worktree / mutant["file"]
    text = path.read_text(encoding="utf-8")
    find = mutant["find"].lstrip("\n")
    count = text.count(find)
    if count != 1:
        raise RuntimeError(f"mutant {mutant['id']}: `find` occurs {count} times in {mutant['file']}")
    path.write_text(text.replace(find, mutant["replace"].lstrip("\n")), encoding="utf-8")


def _run(command: list[str], cwd: Path, env: dict) -> subprocess.CompletedProcess:
    return subprocess.run(command, cwd=cwd, env=env, capture_output=True, text=True, check=False)


def _verifier(worktree: Path, probes: Path, env: dict, binary: Path) -> dict[str, Any]:
    caught_by = []
    for probe in sorted(probes.glob("*.qasm")):
        for backend in ("simulator", "simulator-nisq"):
            completed = _run(
                [str(binary), "lower", str(probe), "--backend", backend, "--verify"], worktree, env
            )
            if "semantic verification failed" in completed.stderr:
                caught_by.append(f"{probe.name}@{backend}")
    return {"caught": bool(caught_by), "evidence": caught_by}


def run(config: dict, out: Path, progress=print) -> dict[str, Any]:
    manifest_path = REPO / "benchmarks" / config["mutants"]["manifest"]
    manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    head = _git("rev-parse", "HEAD").stdout.strip()
    target_dir = REPO / "target" / "mutants"
    env = dict(os.environ, CARGO_TARGET_DIR=str(target_dir), CARGO_TERM_COLOR="never")
    exe = "oqci.exe" if os.name == "nt" else "oqci"
    binary = target_dir / "debug" / exe

    results = []
    for mutant in [None] + manifest["mutant"]:
        label = mutant["id"] if mutant else "(unmutated baseline)"
        tmp = Path(tempfile.mkdtemp(prefix="oqci-mutant-"))
        worktree = tmp / "wt"
        added = _git("worktree", "add", "--detach", str(worktree), head)
        if added.returncode != 0:
            raise RuntimeError(f"git worktree add failed: {added.stderr}")
        try:
            if mutant:
                _apply(worktree, mutant)
            build = _run(["cargo", "build", "-q", "--tests", "--bins"], worktree, env)
            if build.returncode != 0:
                results.append({"id": label, "builds": False, "error": build.stderr[-2000:]})
                progress(f"mutants: {label}: DOES NOT BUILD")
                continue
            row: dict[str, Any] = {
                "id": label,
                "description": mutant["description"] if mutant else "the compiler as committed",
                "file": mutant["file"] if mutant else None,
                "builds": True,
                "checkers": {},
            }
            for checker in manifest["checker"]:
                if "probes" in checker:
                    outcome = _verifier(worktree, worktree / checker["probes"], env, binary)
                else:
                    completed = _run(checker["command"], worktree, env)
                    outcome = {"caught": completed.returncode != 0}
                row["checkers"][checker["id"]] = {"kind": checker["kind"], **outcome}
            row["caught_by"] = [k for k, v in row["checkers"].items() if v["caught"]]
            results.append(row)
            progress(f"mutants: {label}: caught by {row['caught_by'] or 'nothing'}")
        finally:
            _git("worktree", "remove", "--force", str(worktree))
            shutil.rmtree(tmp, ignore_errors=True)

    baseline = results[0]
    clean_baseline = baseline.get("builds") and not baseline.get("caught_by")
    mutants = results[1:]
    summary = {
        "commit": head,
        "baseline_clean": bool(clean_baseline),
        "mutants": len(mutants),
        "caught": sum(1 for m in mutants if m.get("caught_by")),
        "survived": [m["id"] for m in mutants if m.get("builds") and not m.get("caught_by")],
        "broken": [m["id"] for m in mutants if not m.get("builds")],
        "checkers": [{"id": c["id"], "kind": c["kind"]} for c in manifest["checker"]],
    }
    result = {"summary": summary, "results": results}
    (out / "mutants.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    if not clean_baseline:
        raise RuntimeError("the unmutated baseline fails a checker; mutation evidence is void")
    return result
