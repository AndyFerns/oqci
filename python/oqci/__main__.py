"""``python -m oqci run`` — compile a program and run it, in one command.

The CLI form of §19's ``oqci run``. The Rust ``oqci`` binary stops at
``prepare``, because execution lives outside the compiler crate; this command
compiles through the same library (``oqci.compile``) and hands the executable
to an execution adapter::

    python -m oqci run bell.qasm --backend simulator-nisq --runtime aer --shots 2000
    python -m oqci run kernel.py --runtime cudaq
    python -m oqci run bell.qasm --runtime ibm --ibm-backend ibm_fez
    python -m oqci run bell.qasm --runtime ibm --fake FakeFez     # offline

For ``--runtime ibm`` the compile target defaults to the device's own
profile (``oqci.backends.ibm.target_from_backend``), so the executable is
lowered for the device that runs it. ``--ibm-backend`` uses the caller's
saved IBM Quantum account; ``--fake`` uses a fake backend from
``qiskit_ibm_runtime.fake_provider`` through IBM's local testing mode.

The result is printed as OQCI's structured ``ExecutionResult`` (counts,
durations, provenance) with ``--json``, or as a short summary otherwise.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Optional, Sequence

import oqci


def _parse(argv: Optional[Sequence[str]]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(prog="python -m oqci", description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    run = sub.add_parser("run", help="compile a program and execute it")
    run.add_argument("input", type=Path, help="OpenQASM 3 source, or a CUDA-Q kernel (.py)")
    run.add_argument("--frontend", choices=list(oqci.FRONTENDS),
                     help="default: from the file extension")
    device = run.add_mutually_exclusive_group()
    device.add_argument("--backend", help="a registered backend id (see oqci.available_backends())")
    device.add_argument("--target-file", type=Path, help="a serialized target profile to compile for")
    run.add_argument("--target-kind", help="backend kind for --target-file (ibm, simulator)")
    run.add_argument("--runtime", choices=["aer", "cudaq", "ibm"], default="aer")
    run.add_argument("--ibm-backend", help="IBM backend name (with --runtime ibm; saved account)")
    run.add_argument("--fake", help="fake IBM backend for an offline run (with --runtime ibm)")
    run.add_argument("--shots", type=int, default=1024)
    run.add_argument("--seed", type=int, help="simulator seed (not allowed for IBM hardware)")
    run.add_argument("--layout", choices=["trivial", "dense"], default="dense")
    run.add_argument("--bind", action="append", default=[], metavar="NAME=VALUE")
    run.add_argument("--verify", action="store_true", help="semantic verification during compile")
    run.add_argument("--json", action="store_true", help="print the full ExecutionResult as JSON")
    return parser.parse_args(argv)


def _bindings(raw: list[str]) -> dict[str, float]:
    out = {}
    for entry in raw:
        name, sep, value = entry.partition("=")
        if not sep:
            raise SystemExit(f"--bind expects NAME=VALUE, got {entry!r}")
        out[name.strip()] = float(value)
    return out


def _ibm_backend(args: argparse.Namespace) -> Any:
    if args.fake:
        from qiskit_ibm_runtime import fake_provider

        return getattr(fake_provider, args.fake)()
    from qiskit_ibm_runtime import QiskitRuntimeService

    from .backends import ibm

    service = QiskitRuntimeService()
    return service.backend(args.ibm_backend) if args.ibm_backend else ibm.least_busy_cz_backend(service)


def run(args: argparse.Namespace) -> dict[str, Any]:
    source = args.input.read_text(encoding="utf-8")
    frontend = args.frontend or ("cudaq" if args.input.suffix.lower() == ".py" else "openqasm3")
    options: dict[str, Any] = dict(
        frontend=frontend, name=args.input.stem, shots=args.shots, layout=args.layout,
        bindings=_bindings(args.bind) or None, verify=args.verify,
    )
    if args.seed is not None and args.runtime != "ibm":
        options["seed"] = args.seed

    backend_object = None
    if args.runtime == "ibm":
        if args.seed is not None:
            raise SystemExit("IBM hardware cannot be seeded; drop --seed")
        backend_object = _ibm_backend(args)
        if args.backend is None and args.target_file is None:
            from .backends import ibm

            options["target"] = ibm.target_from_backend(backend_object)
    if args.backend:
        options["backend"] = args.backend
    elif args.target_file:
        options["target"] = args.target_file.read_text(encoding="utf-8")
        options["target_kind"] = args.target_kind
    elif "target" not in options:
        options["backend"] = "simulator"

    artifacts = oqci.compile(source, **options)
    executable = artifacts["executable"]
    if args.runtime == "aer":
        from .backends import aer

        result = aer.run(executable)
    elif args.runtime == "cudaq":
        from .backends import cudaq

        result = cudaq.run(executable)
    else:
        from .backends import ibm

        result = ibm.run(executable, backend=backend_object)
    return oqci.backends.to_execution_result(result, artifacts)


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = _parse(argv)
    try:
        outcome = run(args)
    except oqci.OqciError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    if args.json:
        print(json.dumps(outcome, indent=2, sort_keys=True))
        return 0
    provenance = outcome["provenance"]
    print(f"{args.input.name}: {sum(outcome['counts'].values())} shots on "
          f"{provenance['backend_id']} ({provenance['profile_id']}) via {args.runtime}")
    for bits, count in sorted(outcome["counts"].items(), key=lambda kv: -kv[1])[:16]:
        print(f"  {bits}  {count}")
    for label, key in (("compile", "compilation_duration_ms"), ("queue", "submission_duration_ms"),
                       ("execute", "execution_duration_ms")):
        if outcome.get(key) is not None:
            print(f"  {label:8} {outcome[key]:.2f} ms")
    return 0


if __name__ == "__main__":
    sys.exit(main())
