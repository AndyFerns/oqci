"""The checker coverage matrix (Stage G §3.1), validated against the code."""

from __future__ import annotations

import json
import re
import tomllib
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[2]
MANIFEST = REPO / "benchmarks" / "coverage" / "checkers.toml"
KINDS = ("structural", "semantic-test", "semantic-in-compiler", "external")


class MissingChecker(RuntimeError):
    pass


def _exists(ref: str) -> bool:
    path, _, function = ref.partition("::")
    file = REPO / path
    if not file.is_file():
        return False
    text = file.read_text(encoding="utf-8")
    pattern = rf"(?m)^\s*(?:pub(?:\([a-z]+\))?\s+)?(?:async\s+)?(?:fn|def)\s+{re.escape(function)}\b"
    return re.search(pattern, text) is not None


def build() -> dict[str, Any]:
    manifest = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))
    missing = []
    rows = []
    for transformation in manifest["transformation"]:
        cells = {kind: [] for kind in KINDS}
        for checker in transformation["checkers"]:
            if checker["kind"] not in KINDS:
                raise ValueError(f"unknown checker kind {checker['kind']!r}")
            if not _exists(checker["ref"]):
                missing.append(checker["ref"])
            cells[checker["kind"]].append(checker["ref"])
        strongest = next(
            (k for k in ("semantic-in-compiler", "semantic-test", "structural", "external") if cells[k]),
            "none",
        )
        rows.append({
            "transformation": transformation["id"],
            "stage": transformation["stage"],
            "note": transformation.get("note", ""),
            "checkers": cells,
            "strongest": strongest,
        })
    if missing:
        raise MissingChecker("the coverage matrix cites checks that do not exist: " + ", ".join(missing))
    return {"kinds": list(KINDS), "rows": rows}


def run(out: Path, progress=print) -> dict[str, Any]:
    matrix = build()
    (out / "coverage.json").write_text(json.dumps(matrix, indent=2), encoding="utf-8")
    progress(f"coverage: {len(matrix['rows'])} transformations, every cited check exists")
    return matrix
