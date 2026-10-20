"""Loading a protocol config (``benchmarks/configs/*.toml``)."""

from __future__ import annotations

import tomllib
from pathlib import Path
from typing import Any

BENCH = Path(__file__).resolve().parents[1]


def load(path: str | Path) -> dict[str, Any]:
    path = Path(path)
    if not path.is_absolute() and not path.exists():
        path = BENCH / path
    with path.open("rb") as handle:
        config = tomllib.load(handle)
    config["_path"] = str(path.resolve())
    config["_text"] = path.read_text(encoding="utf-8")
    _validate(config)
    return config


def _validate(config: dict[str, Any]) -> None:
    for key in ("seeds", "families", "targets", "compilers", "timing", "correctness"):
        if key not in config:
            raise ValueError(f"config is missing [{key}]")
    seen = set()
    for compiler in config["compilers"]:
        if compiler["id"] in seen:
            raise ValueError(f"duplicate compiler id {compiler['id']!r}")
        seen.add(compiler["id"])
        if compiler["compiler"] not in ("oqci", "qiskit", "tket"):
            raise ValueError(f"unknown compiler {compiler['compiler']!r}")
        for kind in compiler["targets"]:
            if kind not in config["targets"]["kinds"]:
                raise ValueError(f"{compiler['id']}: unknown target kind {kind!r}")


def compiler(config: dict[str, Any], config_id: str) -> dict[str, Any]:
    for entry in config["compilers"]:
        if entry["id"] == config_id:
            return entry
    raise KeyError(config_id)
