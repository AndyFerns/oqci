"""The compile benchmark: every item × target × compiler configuration.

For each combination the runner compiles (with the Stage G timing protocol
for timed configurations), computes metrics, checks correctness, writes the
compiled circuit (OpenQASM 3) and, for OQCI, its canonical QIR, and appends
one record to ``records.jsonl``. A failed correctness check stops the run.

Records follow ``benchmarks/schema/result.schema.json`` — the §20.3 fields
plus correctness, timing breakdown, seeds and the target digest.
"""

from __future__ import annotations

import json
import statistics
import time
from pathlib import Path
from typing import Any, Callable, Optional

from . import circuits, environment, equivalence, metrics, targets
from .compilers import Compiled, registry, versions

SCHEMA_VERSION = "1.0"


class CorrectnessFailure(RuntimeError):
    """A compiled output is not equivalent to its source. Stops the run."""


def _time(fn: Callable[[], Compiled], warmup: int, repetitions: int) -> tuple[Compiled, list[float]]:
    result: Optional[Compiled] = None
    for _ in range(warmup):
        result = fn()
    samples = []
    for _ in range(max(1, repetitions)):
        started = time.perf_counter_ns()
        result = fn()
        samples.append((time.perf_counter_ns() - started) / 1e6)
    assert result is not None
    return result, samples


def _summary(samples: list[float]) -> dict[str, Any]:
    ordered = sorted(samples)
    if len(ordered) >= 4:
        q = statistics.quantiles(ordered, n=4, method="inclusive")
        iqr = q[2] - q[0]
    else:
        iqr = (ordered[-1] - ordered[0]) if len(ordered) > 1 else 0.0
    return {
        "median_ms": statistics.median(ordered),
        "iqr_ms": iqr,
        "samples_ms": samples,
    }


def _qasm(circuit) -> str:
    from qiskit import qasm3

    return qasm3.dumps(circuit)


def run(config: dict, out: Path, *, progress: Callable[[str], None] = print,
        only: Optional[set[str]] = None) -> Path:
    """Run the benchmark described by ``config`` into the directory ``out``."""
    out.mkdir(parents=True, exist_ok=True)
    (out / "artifacts").mkdir(exist_ok=True)
    (out / "config.toml").write_text(config["_text"], encoding="utf-8")
    env = environment.capture()
    (out / "environment.json").write_text(json.dumps(env, indent=2), encoding="utf-8")

    compile_fns = registry()
    compiler_versions = versions()
    items = circuits.corpus(config)
    timing = config["timing"]
    check = config["correctness"]
    benchmark_id = out.name
    records_path = out / "records.jsonl"
    records_path.write_text("", encoding="utf-8")
    counts = {"pass": 0, "not-checked": 0}

    with records_path.open("a", encoding="utf-8") as sink:
        for item in items:
            if only and item.circuit_id not in only:
                continue
            for entry in config["compilers"]:
                for kind in entry["targets"]:
                    target = targets.for_item(kind, item.width, config)
                    if target is None:
                        continue
                    fn = compile_fns[entry["compiler"]]
                    warmup = int(timing["warmup"]) if entry.get("timed") else 0
                    reps = int(timing["repetitions"]) if entry.get("timed") else 1
                    compiled, samples = _time(lambda: fn(item, target, entry), warmup, reps)

                    verdict = equivalence.check(
                        item.circuit,
                        compiled,
                        operator_max_qubits=int(check["operator_max_qubits"]),
                        state_max_qubits=int(check["state_max_qubits"]),
                        tolerance=float(check["tolerance"]),
                    )
                    record_id = f"{item.circuit_id}.{target.target_id}.{entry['id']}"
                    if verdict.status == "fail":
                        raise CorrectnessFailure(f"{record_id}: {verdict}")
                    counts[verdict.status] += 1

                    artifact = Path("artifacts") / f"{record_id}.qasm"
                    (out / artifact).write_text(_qasm(compiled.circuit), encoding="utf-8")
                    paths = {"compiled_qasm": str(artifact).replace("\\", "/")}
                    qir = compiled.internal.get("qir")
                    if qir:
                        qir_path = Path("artifacts") / f"{record_id}.ll"
                        (out / qir_path).write_text(qir["text"], encoding="utf-8")
                        paths["qir"] = str(qir_path).replace("\\", "/")

                    provenance = compiled.internal.get("provenance") or {}
                    record = {
                        "schema_version": SCHEMA_VERSION,
                        "benchmark_id": benchmark_id,
                        "record_id": record_id,
                        "circuit_id": item.circuit_id,
                        "circuit_family": item.family,
                        "circuit_size": item.n,
                        "circuit_width": item.width,
                        "circuit_reps": item.reps,
                        "circuit_instance": item.instance,
                        "item_seed": str(item.seed),
                        "source_format": "qiskit.QuantumCircuit",
                        "compiler": compiled.compiler,
                        "compiler_version": compiler_versions.get(compiled.compiler),
                        "config_id": entry["id"],
                        "config": {k: v for k, v in entry.items() if k not in ("targets",)},
                        "git_commit": env["repository_commit"],
                        "frontend": "qiskit" if compiled.compiler == "oqci" else None,
                        "pass_pipeline": provenance.get("pass_pipeline"),
                        "backend": target.target_id,
                        "target_kind": target.kind,
                        "target_profile": {
                            "id": target.target_id,
                            "digest": target.digest(),
                            "source": target.source,
                            "basis": target.basis,
                            "num_qubits": target.num_qubits,
                        },
                        "cost_model": (
                            {
                                "id": provenance.get("cost_model_id"),
                                "version": provenance.get("cost_model_version"),
                                "configuration": provenance.get("cost_model_configuration"),
                                "cost": compiled.internal.get("cost"),
                            }
                            if provenance
                            else None
                        ),
                        "compilation_time": {
                            **_summary(samples),
                            "warmup": warmup,
                            "timed": bool(entry.get("timed")),
                            "clock": "time.perf_counter_ns around the compile call",
                        },
                        "oqci_timings": compiled.internal.get("timings"),
                        "oqci_passes": compiled.internal.get("passes"),
                        "metrics": metrics.metrics(compiled.circuit, target.basis),
                        "swaps_inserted": compiled.swaps,
                        "layout": {"initial": compiled.initial, "final": compiled.final},
                        "correctness": verdict.to_dict(),
                        "execution_metadata": None,
                        "result_artifact_paths": paths,
                    }
                    sink.write(json.dumps(record, sort_keys=True) + "\n")
                    sink.flush()
                    progress(
                        f"{record_id:60} {verdict.method:28} "
                        f"{record['compilation_time']['median_ms']:9.2f} ms"
                    )

    summary = {
        "benchmark_id": benchmark_id,
        "config": config["protocol"],
        "records": counts["pass"] + counts["not-checked"],
        "correctness": counts,
        "finished_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    return records_path
