"""Validating records against ``benchmarks/schema/result.schema.json``.

Implements exactly the JSON-Schema keywords that schema uses — ``type``
(including unions), ``required``, ``enum`` and nested ``properties`` — so
validation needs no extra dependency. A keyword outside that subset is an
error, so the schema cannot grow a rule this checker silently ignores.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

SCHEMA = Path(__file__).resolve().parents[1] / "schema" / "result.schema.json"
_SUPPORTED = {"$schema", "$id", "title", "description", "type", "required", "properties", "enum"}
_TYPES = {
    "object": dict,
    "array": list,
    "string": str,
    "integer": int,
    "number": (int, float),
    "boolean": bool,
    "null": type(None),
}


def _check(value: Any, schema: dict, path: str, errors: list[str]) -> None:
    unknown = set(schema) - _SUPPORTED
    if unknown:
        raise ValueError(f"schema at {path} uses unsupported keywords {sorted(unknown)}")
    if "type" in schema:
        types = schema["type"] if isinstance(schema["type"], list) else [schema["type"]]
        ok = any(
            isinstance(value, _TYPES[t]) and not (t in ("integer", "number") and isinstance(value, bool))
            for t in types
        )
        if not ok:
            errors.append(f"{path}: expected {types}, got {type(value).__name__}")
            return
    if "enum" in schema and value not in schema["enum"]:
        errors.append(f"{path}: {value!r} not in {schema['enum']}")
    if isinstance(value, dict):
        for key in schema.get("required", []):
            if key not in value:
                errors.append(f"{path}: missing required key {key!r}")
        for key, sub in schema.get("properties", {}).items():
            if key in value:
                _check(value[key], sub, f"{path}.{key}", errors)


def validate(record: dict, schema: dict | None = None) -> list[str]:
    """Every way ``record`` violates the schema; empty if it conforms."""
    schema = schema or json.loads(SCHEMA.read_text(encoding="utf-8"))
    errors: list[str] = []
    _check(record, schema, "$", errors)
    return errors
