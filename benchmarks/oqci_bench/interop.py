"""Interoperability (Stage G §3.3): acceptance and execution agreement.

**Acceptance.** The canonical QIR of every corpus item — lowered for the
linear target (labeled ``custom``) and for the ``qir-base`` backend (labeled
``base_profile``), in both pointer styles — is fed to each named consumer at
its pinned version: PyQIR (parse, verify, read the QIR metadata) and
``llvm-as`` 18 and 14 (assemble; 14 only for typed pointers). A consumer
that is not installed is reported ``not run`` — never as a pass. What
acceptance proves: well-formed LLVM IR with readable QIR metadata. Not
Base Profile conformance, and not execution.

**Execution agreement.** Each executing consumer (Aer, CUDA-Q) runs the
OQCI executable for the linear target, and its counts are tested against the
*exact* distribution of the source circuit with a chi-square goodness-of-fit
test (Stage G: reject at p < alpha; bins with expected count below
``min_expected`` pooled). TVD is reported alongside.
"""

from __future__ import annotations

import json
import re
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Any, Optional

from . import circuits, stats, targets


def _emit(item, target, pointers: str) -> dict[str, Optional[str]]:
    import oqci

    custom = oqci.compile(item.circuit, target=target.oqci_profile(), qir_pointers=pointers)
    base = oqci.compile(item.circuit, backend="qir-base", qir_pointers=pointers)
    return {
        "custom": custom["qir"]["text"] if custom["qir"] else None,
        "base_profile": base["qir"]["text"] if base["qir"] else None,
    }


def _pyqir(text: str) -> Optional[str]:
    """None if PyQIR accepts the module, else what it objected to."""
    import pyqir

    try:
        module = pyqir.Module.from_ir(pyqir.Context(), text, "m")
    except Exception as error:  # noqa: BLE001
        return f"does not parse: {error}"
    failure = module.verify()
    if failure is not None:
        return f"does not verify: {failure}"
    if pyqir.qir_major_version(module) != 1 or pyqir.qir_minor_version(module) != 0:
        return "QIR version metadata is not 1.0"
    entries = [f for f in module.functions if pyqir.is_entry_point(f)]
    if len(entries) != 1:
        return f"{len(entries)} entry points"
    return None


def _llvm_as(major: int) -> Optional[str]:
    """Path of an llvm-as of exactly this major version, if installed."""
    for name in (f"llvm-as-{major}", "llvm-as"):
        path = shutil.which(name)
        if not path:
            continue
        try:
            version = subprocess.run([path, "--version"], capture_output=True, text=True,
                                     timeout=30).stdout
        except (OSError, subprocess.SubprocessError):
            continue
        match = re.search(r"version (\d+)\.", version)
        if match and int(match.group(1)) == major:
            return path
    return None


def _assemble(tool: str, text: str) -> Optional[str]:
    with tempfile.TemporaryDirectory() as tmp:
        source = Path(tmp) / "m.ll"
        source.write_text(text, encoding="utf-8")
        completed = subprocess.run(
            [tool, str(source), "-o", str(Path(tmp) / "m.bc")], capture_output=True, text=True,
            timeout=120,
        )
        return None if completed.returncode == 0 else completed.stderr.strip()[:500]


def acceptance(config: dict, out: Path, progress=print) -> dict[str, Any]:
    from importlib import metadata

    qir_dir = out / "interop" / "qir"
    try:
        pyqir_version = metadata.version("pyqir")
    except metadata.PackageNotFoundError:
        pyqir_version = None
    consumers = {
        "pyqir-0.12.5": pyqir_version == "0.12.5",
        "llvm-as-18": _llvm_as(18),
        "llvm-as-14": _llvm_as(14),
    }
    rows = []
    for item in circuits.corpus(config):
        target = targets.for_item("linear", item.width, config)
        for pointers in ("opaque", "typed"):
            for label, text in _emit(item, target, pointers).items():
                if text is None:
                    continue
                name = f"{item.circuit_id}.{label}.{pointers}.ll"
                (qir_dir / pointers).mkdir(parents=True, exist_ok=True)
                (qir_dir / pointers / name).write_text(text, encoding="utf-8")
                row = {"module": name, "label": label, "pointers": pointers, "results": {}}
                for consumer in config["interop"]["consumers"]:
                    available = consumers.get(consumer)
                    if consumer == "llvm-as-14" and pointers == "opaque":
                        row["results"][consumer] = "not applicable (LLVM 14 predates opaque pointers)"
                    elif not available:
                        row["results"][consumer] = "not run"
                    elif consumer.startswith("pyqir"):
                        problem = _pyqir(text)
                        row["results"][consumer] = "accepted" if problem is None else f"rejected: {problem}"
                    else:
                        problem = _assemble(str(available), text)
                        row["results"][consumer] = "accepted" if problem is None else f"rejected: {problem}"
                rows.append(row)
    summary = {}
    for consumer in config["interop"]["consumers"]:
        values = [r["results"][consumer] for r in rows]
        summary[consumer] = {
            "accepted": sum(v == "accepted" for v in values),
            "rejected": sum(v.startswith("rejected") for v in values),
            "not_run": sum(v == "not run" for v in values),
            "not_applicable": sum(v.startswith("not applicable") for v in values),
        }
    progress(f"interop acceptance: {summary}")
    return {"consumers": {k: (str(v) if v else None) for k, v in consumers.items()},
            "summary": summary, "modules": rows}


def agreement(config: dict, out: Path, progress=print) -> dict[str, Any]:
    import oqci

    spec = config["interop"]["agreement"]
    rows = []
    available = {}
    try:
        import qiskit_aer  # noqa: F401

        available["aer"] = True
    except ImportError:
        available["aer"] = False
    try:
        import cudaq  # noqa: F401

        available["cudaq"] = True
    except ImportError:
        available["cudaq"] = False

    for item in circuits.corpus(config):
        if item.width > int(spec["max_qubits"]):
            continue
        target = targets.for_item("linear", item.width, config)
        artifacts = oqci.compile(
            item.circuit, target=target.oqci_profile(), shots=int(spec["shots"]),
            seed=item.seed % (2**63),
        )
        exact = stats.exact_distribution(item.circuit)
        row = {"circuit_id": item.circuit_id, "results": {}}
        for consumer in spec["consumers"]:
            if not available.get(consumer):
                row["results"][consumer] = {"status": "not run"}
                continue
            if consumer == "aer":
                from oqci.backends import aer

                counts = aer.run(artifacts["executable"]).counts
            else:
                from oqci.backends import cudaq as cq

                counts = cq.run(artifacts["executable"]).counts
            fit = stats.chi_square_gof(counts, exact, min_expected=float(spec["min_expected"]))
            row["results"][consumer] = {
                "status": "agrees" if fit["p_value"] >= float(spec["alpha"]) else "disagrees",
                "tvd": stats.total_variation(stats.normalize(counts), exact),
                **fit,
            }
        rows.append(row)
    summary = {}
    for consumer in spec["consumers"]:
        statuses = [r["results"][consumer]["status"] for r in rows]
        summary[consumer] = {s: statuses.count(s) for s in sorted(set(statuses))}
    progress(f"interop agreement: {summary}")
    return {"alpha": spec["alpha"], "shots": spec["shots"], "summary": summary, "circuits": rows}


def run(config: dict, out: Path, progress=print) -> dict[str, Any]:
    result = {
        "acceptance": acceptance(config, out, progress),
        "agreement": agreement(config, out, progress),
    }
    (out / "interop.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    return result
