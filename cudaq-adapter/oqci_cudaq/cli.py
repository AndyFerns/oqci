from __future__ import annotations

import argparse
import json

from .backend import CudaQBackend
from .frontend import CudaQFrontend
from .ir import Circuit


def main():
    parser = argparse.ArgumentParser(description="OQCI CUDA-Q adapters")
    sub = parser.add_subparsers(dest="command", required=True)

    p_front = sub.add_parser("frontend", help="CUDA-Q Python -> OQCI JSON")
    p_front.add_argument("source")
    p_front.add_argument("-o", "--output", required=True)

    p_back = sub.add_parser("backend", help="OQCI JSON -> CUDA-Q source")
    p_back.add_argument("ir")
    p_back.add_argument("-o", "--output", required=True)
    p_back.add_argument("--target", default="qpp-cpu")

    p_run = sub.add_parser("run", help="OQCI JSON -> CUDA-Q execution")
    p_run.add_argument("ir")
    p_run.add_argument("--target", default="qpp-cpu")
    p_run.add_argument("--shots", type=int, default=1000)

    args = parser.parse_args()

    if args.command == "frontend":
        circuit = CudaQFrontend().from_file(args.source)
        with open(args.output, "w", encoding="utf-8") as f:
            json.dump(circuit.to_dict(), f, indent=2)
        print(f"Wrote OQCI IR to {args.output}")

    elif args.command == "backend":
        with open(args.ir, encoding="utf-8") as f:
            circuit = Circuit.from_dict(json.load(f))
        CudaQBackend(args.target).export_source(circuit, args.output)
        print(f"Wrote CUDA-Q source to {args.output}")

    elif args.command == "run":
        with open(args.ir, encoding="utf-8") as f:
            circuit = Circuit.from_dict(json.load(f))
        result = CudaQBackend(args.target).run(circuit, args.shots)
        print(json.dumps(result, indent=2))
