"""``python -m oqci_bench`` — run from the ``benchmarks/`` directory.

    run          the compile benchmark (records.jsonl, artifacts/)
    determinism  repeated fresh-process compilation (determinism.json)
    interop      QIR acceptance and execution agreement (interop.json)
    mutants      mutation evidence (mutants.json)
    coverage     the checker coverage matrix (coverage.json)
    report       tables and figures from a run directory
    all          run + determinism + interop + coverage + mutants + report
    hardware     snapshot | compile | submit | retrieve (IBM, operator-run)
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

from . import config as config_module


def _out(args) -> Path:
    if args.out:
        return Path(args.out)
    from . import environment

    stamp = time.strftime("%Y%m%d")
    return Path("results") / f"{stamp}-{environment.git_commit()[:8]}"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="oqci_bench", description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    def common(p, needs_out=True):
        p.add_argument("--config", default="configs/stage_g.toml")
        if needs_out:
            p.add_argument("--out", help="run directory (default results/<date>-<commit>)")

    for name in ("run", "determinism", "interop", "mutants", "coverage", "all"):
        p = sub.add_parser(name)
        common(p)
        if name == "run":
            p.add_argument("--only", help="comma-separated circuit ids")
    p = sub.add_parser("report")
    p.add_argument("run_dir")
    p.add_argument("--config", default=None, help="defaults to the run's own config.toml")

    hw = sub.add_parser("hardware")
    hw_sub = hw.add_subparsers(dest="step", required=True)
    for step in ("snapshot", "compile", "submit", "retrieve"):
        p = hw_sub.add_parser(step)
        common(p, needs_out=False)
        p.add_argument("--out", required=True, help="hardware run directory")
        p.add_argument("--fake", help="a qiskit_ibm_runtime.fake_provider backend, for an offline run")
        if step == "snapshot":
            p.add_argument("--backend", help="a backend name; default: least busy CZ-native")

    p = sub.add_parser("_digests")  # internal: one determinism repetition
    p.add_argument("--config", required=True)
    p.add_argument("--compiler", required=True)
    p.add_argument("--targets", required=True)

    args = parser.parse_args(argv)

    if args.command == "_digests":
        from . import determinism

        print(json.dumps(determinism.digests(args.config, args.compiler, args.targets.split(","))))
        return 0

    if args.command == "report":
        run_dir = Path(args.run_dir)
        cfg = config_module.load(args.config or (run_dir / "config.toml"))
        from . import report

        path = report.build(run_dir, cfg)
        print(f"report: {path / 'README.md'}")
        return 0

    cfg = config_module.load(args.config)

    if args.command == "hardware":
        from . import hardware

        out = Path(args.out)
        if args.step == "snapshot":
            result = hardware.snapshot(cfg, out, backend_name=args.backend, fake=args.fake)
            print(f"snapshot of {result['backend_name']} ({result['num_qubits']} qubits) in {out}")
        elif args.step == "compile":
            print(hardware.compile_subset(cfg, out))
        elif args.step == "submit":
            record = hardware.submit(cfg, out, fake=args.fake)
            print(f"submitted job {record['job_id']} to {record['backend_name']} "
                  f"({len(record['pubs'])} circuits x {record['shots']} shots)")
        else:
            result = hardware.retrieve(out)
            print(f"retrieved {len(result['counts'])} results from {result['backend_name']}")
        return 0

    out = _out(args)
    out.mkdir(parents=True, exist_ok=True)
    started = time.time()
    if args.command in ("run", "all"):
        from . import runner

        only = set(args.only.split(",")) if getattr(args, "only", None) else None
        runner.run(cfg, out, only=only)
    if args.command in ("determinism", "all"):
        from . import determinism

        determinism.run(cfg, out)
    if args.command in ("interop", "all"):
        from . import interop

        interop.run(cfg, out)
    if args.command in ("coverage", "all"):
        from . import coverage

        coverage.run(out)
    if args.command in ("mutants", "all"):
        from . import mutants

        mutants.run(cfg, out)
    if args.command == "all":
        from . import report

        if not (out / "config.toml").exists():
            (out / "config.toml").write_text(cfg["_text"], encoding="utf-8")
        report.build(out, cfg)
    print(f"done in {time.time() - started:.0f}s: {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
