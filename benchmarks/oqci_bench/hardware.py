"""The IBM hardware experiment (Stage G §5, secondary) — run by its operator.

Five steps, each a separate command so they can run hours apart and on
different machines; each reads the previous step's files from one
directory::

    python -m oqci_bench hardware snapshot --out DIR [--backend NAME]
    python -m oqci_bench hardware compile  --out DIR
    python -m oqci_bench hardware submit   --out DIR
    python -m oqci_bench hardware retrieve --out DIR
    python -m oqci_bench report DIR

``snapshot`` records the device's target at that moment (the default
selection is the least busy operational CZ-native backend the account can
use; ``--backend`` names one). ``compile`` builds the Stage G hardware subset
with OQCI and with Qiskit O3, both against that snapshot's coupling map and
basis and nothing else. ``submit`` checks every circuit against the device's
instruction set — refusing, never transpiling — and submits all of them as
one Sampler job with twirling and dynamical decoupling disabled. ``retrieve``
fetches the counts.

Credentials: only the operator's own saved account (``QiskitRuntimeService()``
with no arguments). Nothing written here contains a token.

``--fake NAME`` runs every step against a fake backend through IBM's local
testing mode — the Stage G noisy-simulation data, and an offline dry run of
the whole flow.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Optional

from . import circuits, stats
from .compilers import Compiled, qiskit_compiler
from .targets import Target


def _service():
    from qiskit_ibm_runtime import QiskitRuntimeService

    return QiskitRuntimeService()


def _backend(name: Optional[str], fake: Optional[str]):
    if fake:
        from qiskit_ibm_runtime import fake_provider

        return getattr(fake_provider, fake)()
    from oqci.backends import ibm

    service = _service()
    return service.backend(name) if name else ibm.least_busy_cz_backend(service)


def snapshot(config: dict, out: Path, *, backend_name: Optional[str], fake: Optional[str]) -> dict:
    from oqci.backends import ibm

    out.mkdir(parents=True, exist_ok=True)
    backend = _backend(backend_name, fake)
    profile = ibm.target_from_backend(backend)
    record = {
        "backend_name": backend.name,
        "backend_version": getattr(backend, "backend_version", None),
        "num_qubits": backend.num_qubits,
        "fake": fake,
        "selection": "named" if (backend_name or fake) else config["hardware"]["selection"],
        "profile": profile,
    }
    (out / "snapshot.json").write_text(json.dumps(record, indent=2), encoding="utf-8")
    return record


def _target_from_snapshot(snapshot: dict) -> Target:
    profile = snapshot["profile"]
    edges = tuple(tuple(e) for e in profile["topology"]["edges"])
    return Target(
        target_id=profile["id"],
        kind="hardware",
        num_qubits=int(profile["topology"]["qubit_count"]),
        edges=edges,
        two_qubit="cz",
        source=f"snapshot of {snapshot['backend_name']}@{snapshot['backend_version']}",
    )


def compile_subset(config: dict, out: Path) -> dict:
    from qiskit import qpy

    snap = json.loads((out / "snapshot.json").read_text(encoding="utf-8"))
    target = _target_from_snapshot(snap)
    profile = snap["profile"]
    items = circuits.hardware_subset(config)
    compiled_dir = out / "compiled"
    compiled_dir.mkdir(exist_ok=True)
    manifest = []
    for item in items:
        for config_id in config["hardware"]["compilers"]:
            entry = next(c for c in config["compilers"] if c["id"] == config_id)
            if entry["compiler"] == "oqci":
                import oqci
                from oqci.backends.aer import to_qiskit

                artifacts = oqci.compile(
                    item.circuit, target=profile, name=item.circuit_id,
                    layout=entry.get("layout", "dense"), verify=True,
                    verify_max_qubits=max(8, item.width + 4),
                    shots=int(config["hardware"]["shots"]),
                )
                circuit = to_qiskit(artifacts["executable"])
                prov = artifacts["executable"]["provenance"]
                compiled = Compiled("oqci", config_id, circuit, prov["initial_layout"], prov["final_layout"])
                detail = {"executable": artifacts["executable"], "verification": artifacts["verification"]}
            elif entry["compiler"] == "qiskit":
                compiled = qiskit_compiler.compile_item(item, target, entry)
                detail = {}
            else:
                raise ValueError(f"{config_id}: hardware runs support oqci and qiskit configurations")
            name = f"{item.circuit_id}.{config_id}"
            with (compiled_dir / f"{name}.qpy").open("wb") as handle:
                qpy.dump(compiled.circuit, handle)
            manifest.append({
                "name": name,
                "circuit_id": item.circuit_id,
                "config_id": config_id,
                "initial": compiled.initial,
                "final": compiled.final,
                "exact_distribution": stats.exact_distribution(item.circuit),
                **detail,
            })
    (out / "compiled.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return {"circuits": len(manifest)}


def submit(config: dict, out: Path, *, fake: Optional[str]) -> dict:
    from qiskit import qpy

    from oqci.backends import ibm

    snap = json.loads((out / "snapshot.json").read_text(encoding="utf-8"))
    backend = _backend(snap["backend_name"] if not fake else None, fake)
    if backend.name != snap["backend_name"]:
        raise RuntimeError(f"snapshot is of {snap['backend_name']}, backend is {backend.name}")
    manifest = json.loads((out / "compiled.json").read_text(encoding="utf-8"))
    pubs = []
    for entry in manifest:
        with (out / "compiled" / f"{entry['name']}.qpy").open("rb") as handle:
            circuit = qpy.load(handle)[0]
        if entry["config_id"].startswith("oqci"):
            circuit = ibm.to_isa_circuit(entry["executable"], backend)
        ibm.check_isa(circuit, backend)
        pubs.append(circuit)
    shots = int(config["hardware"]["shots"])
    sampler, options = ibm.configured_sampler(backend)
    job = sampler.run(pubs, shots=shots)
    record = {
        "job_id": str(job.job_id()),
        "backend_name": backend.name,
        "shots": shots,
        "options": options,
        "pubs": [e["name"] for e in manifest],
        "fake": fake,
    }
    (out / "submission.json").write_text(json.dumps(record, indent=2), encoding="utf-8")
    if fake:
        # Local jobs cannot be retrieved by id from another process.
        _collect(out, job, record)
    return record


def _collect(out: Path, job: Any, submission: dict) -> dict:
    from oqci.backends.ibm import _durations, _jsonable

    primitive = job.result()
    counts = {}
    for name, result in zip(submission["pubs"], primitive):
        counts[name] = {str(k): int(v) for k, v in result.data.c.get_counts().items()}
    try:
        metrics = _jsonable(dict(job.metrics()))
    except Exception:  # noqa: BLE001
        metrics = {}
    queue_ms, execution_ms = _durations(metrics)
    record = {
        "job_id": submission["job_id"],
        "backend_name": submission["backend_name"],
        "fake": submission.get("fake"),
        "metrics": metrics,
        "queue_duration_ms": queue_ms,
        "execution_duration_ms": execution_ms,
        "counts": counts,
    }
    (out / "results.json").write_text(json.dumps(record, indent=2), encoding="utf-8")
    return record


def retrieve(out: Path) -> dict:
    submission = json.loads((out / "submission.json").read_text(encoding="utf-8"))
    if submission.get("fake"):
        return json.loads((out / "results.json").read_text(encoding="utf-8"))
    job = _service().job(submission["job_id"])
    return _collect(out, job, submission)


def analyze(out: Path) -> list[dict]:
    """Hellinger fidelity of each circuit's counts to its exact distribution."""
    manifest = {e["name"]: e for e in json.loads((out / "compiled.json").read_text(encoding="utf-8"))}
    results = json.loads((out / "results.json").read_text(encoding="utf-8"))
    rows = []
    for name, counts in results["counts"].items():
        entry = manifest[name]
        observed = stats.normalize(counts)
        rows.append({
            "name": name,
            "circuit_id": entry["circuit_id"],
            "config_id": entry["config_id"],
            "backend": results["backend_name"],
            "fake": results.get("fake"),
            "shots": sum(counts.values()),
            "hellinger_fidelity": stats.hellinger_fidelity(observed, entry["exact_distribution"]),
            "tvd": stats.total_variation(observed, entry["exact_distribution"]),
        })
    return rows
