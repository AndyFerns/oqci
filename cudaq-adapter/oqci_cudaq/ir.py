from __future__ import annotations
from dataclasses import dataclass, field
from typing import Any


@dataclass
class Operation:
    """A minimal vendor-neutral OQCI operation."""

    name: str
    qubits: list[int] = field(default_factory=list)
    params: list[float] = field(default_factory=list)
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "qubits": self.qubits,
            "params": self.params,
            "metadata": self.metadata,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "Operation":
        return cls(
            name=str(data["name"]),
            qubits=[int(x) for x in data.get("qubits", [])],
            params=[float(x) for x in data.get("params", [])],
            metadata=dict(data.get("metadata", {})),
        )


@dataclass
class Circuit:
    """OQCI's compact common representation used by this adapter."""

    num_qubits: int
    operations: list[Operation] = field(default_factory=list)
    measurements: list[int] | None = None
    metadata: dict[str, Any] = field(default_factory=dict)

    def validate(self) -> None:
        if self.num_qubits < 1:
            raise ValueError("num_qubits must be >= 1")

        for op in self.operations:
            for q in op.qubits:
                if q < 0 or q >= self.num_qubits:
                    raise ValueError(
                        f"Operation {op.name!r} references invalid qubit {q}"
                    )

        allowed = {
            "h", "x", "y", "z", "s", "sdg", "t", "tdg",
            "cx", "cz", "swap", "rx", "ry", "rz", "r1",
            "measure",
        }
        for op in self.operations:
            if op.name.lower() not in allowed:
                raise ValueError(f"Unsupported OQCI operation: {op.name}")

            if op.name.lower() in {"rx", "ry", "rz", "r1"} and len(op.params) != 1:
                raise ValueError(f"{op.name} requires exactly one parameter")

            if op.name.lower() in {"cx", "cz", "swap"} and len(op.qubits) != 2:
                raise ValueError(f"{op.name} requires exactly two qubits")

        if self.measurements is not None:
            for q in self.measurements:
                if q < 0 or q >= self.num_qubits:
                    raise ValueError(f"Invalid measurement qubit {q}")

    def to_dict(self) -> dict[str, Any]:
        self.validate()
        return {
            "num_qubits": self.num_qubits,
            "operations": [op.to_dict() for op in self.operations],
            "measurements": self.measurements,
            "metadata": self.metadata,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "Circuit":
        circuit = cls(
            num_qubits=int(data["num_qubits"]),
            operations=[Operation.from_dict(x) for x in data.get("operations", [])],
            measurements=data.get("measurements"),
            metadata=dict(data.get("metadata", {})),
        )
        circuit.validate()
        return circuit
