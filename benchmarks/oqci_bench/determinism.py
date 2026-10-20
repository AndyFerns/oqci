"""The determinism predicate (Stage G §3.2, ``docs/determinism.md``).

A compilation is deterministic if its semantic outputs — optimized and
lowered circuits, executable, layouts, cost, QIR text — are byte-identical
across repeated runs of the same build. Measurements (durations) are removed
first, by the same rule as ``oqci::cli::snapshot::strip_measurements``.

Each repetition runs in a **fresh process** that compiles the whole corpus
and prints one SHA-256 per item: Rust seeds its hash maps per process, so an
output that leaked hash-iteration order would pass an in-process comparison
and fail this one. Pass/fail is reported per item, for this platform.
"""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

from . import circuits, config as config_module, targets


def strip_measurements(value: Any) -> Any:
    """Drop every ``timings`` key and every ``*duration_us``/``*duration_ms``
    key, at any depth. Mirrors the Rust definition exactly."""
    if isinstance(value, dict):
        return {
            k: strip_measurements(v)
            for k, v in value.items()
            if k != "timings" and not k.endswith("duration_us") and not k.endswith("duration_ms")
        }
    if isinstance(value, list):
        return [strip_measurements(v) for v in value]
    return value


def digests(config_path: str, compiler_id: str, kinds: list[str]) -> dict[str, str]:
    """Compile the corpus once in this process; one digest per item."""
    import oqci

    config = config_module.load(config_path)
    entry = config_module.compiler(config, compiler_id)
    out: dict[str, str] = {}
    for item in circuits.corpus(config):
        for kind in kinds:
            target = targets.for_item(kind, item.width, config)
            if target is None:
                continue
            artifacts = oqci.compile(
                item.circuit,
                target=target.oqci_profile(),
                name=item.circuit_id,
                layout=entry.get("layout", "dense"),
                passes=entry.get("passes"),
                disable=entry.get("disable"),
            )
            semantic = json.dumps(strip_measurements(artifacts), sort_keys=True)
            out[f"{item.circuit_id}.{target.target_id}"] = hashlib.sha256(semantic.encode()).hexdigest()
    return out


def run(config: dict, out: Path, progress=print) -> dict[str, Any]:
    spec = config["determinism"]
    reps = int(spec["repetitions"])
    kinds = list(spec["targets"])
    runs: list[dict[str, str]] = []
    for rep in range(reps):
        command = [
            sys.executable,
            "-m",
            "oqci_bench",
            "_digests",
            "--config",
            config["_path"],
            "--compiler",
            spec["compiler"],
            "--targets",
            ",".join(kinds),
        ]
        completed = subprocess.run(
            command, cwd=Path(__file__).resolve().parents[1], capture_output=True, text=True,
            check=False,
        )
        if completed.returncode != 0:
            raise RuntimeError(f"repetition {rep} failed:\n{completed.stderr}")
        runs.append(json.loads(completed.stdout))
        progress(f"determinism: repetition {rep + 1}/{reps} ({len(runs[-1])} items)")

    keys = sorted(runs[0])
    per_item = {}
    for key in keys:
        values = {r.get(key) for r in runs}
        per_item[key] = {"pass": len(values) == 1 and None not in values, "distinct": len(values)}
    import platform

    result = {
        "predicate": "byte-identical semantic outputs after strip_measurements",
        "platform": platform.platform(),
        "repetitions": reps,
        "fresh_process": True,
        "compiler": spec["compiler"],
        "targets": kinds,
        "items": len(keys),
        "passed": sum(1 for v in per_item.values() if v["pass"]),
        "per_item": per_item,
    }
    (out / "determinism.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    return result
