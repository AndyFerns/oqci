from __future__ import annotations

import ast
import math
import re
from pathlib import Path

from .ir import Circuit, Operation


class CudaQFrontend:
    """
    CUDA-Q -> OQCI Unified IR adapter.

    The frontend intentionally parses a small, explicit CUDA-Q Python subset.
    This makes the compiler boundary deterministic and testable instead of
    depending on CUDA-Q's internal Python implementation details.

    Supported:
      cudaq.qvector(N)
      h(q[i]), x(q[i]), y(q[i]), z(q[i]), s(q[i]), sdg(q[i]), t(q[i]), tdg(q[i])
      cx(q[i], q[j]), cz(q[i], q[j]), swap(q[i], q[j])
      rx(theta, q[i]), ry(theta, q[i]), rz(theta, q[i]), r1(theta, q[i])
      mz(q) / mz(q[i])

    Parameters may be numeric literals or simple arithmetic expressions
    involving math.pi.
    """

    GATE_ALIASES = {
        "h": "h", "x": "x", "y": "y", "z": "z",
        "s": "s", "sdg": "sdg", "t": "t", "tdg": "tdg",
        "cx": "cx", "cnot": "cx", "cz": "cz", "swap": "swap",
        "rx": "rx", "ry": "ry", "rz": "rz", "r1": "r1",
    }

    def from_file(self, path: str | Path) -> Circuit:
        return self.from_source(Path(path).read_text(encoding="utf-8"))

    def from_source(self, source: str) -> Circuit:
        tree = ast.parse(source)
        num_qubits = self._find_qvector_size(tree)
        if num_qubits is None:
            raise ValueError("Could not find cudaq.qvector(N) in the CUDA-Q source.")

        ops: list[Operation] = []
        measurements: list[int] | None = None

        for node in ast.walk(tree):
            if not isinstance(node, ast.Call):
                continue

            name = self._call_name(node.func)
            if name is None:
                continue

            if name == "mz":
                measurements = self._parse_measurement(node)
                continue

            gate = self._gate_from_call(node)
            if gate is None:
                continue

            args = list(node.args)
            if gate in {"rx", "ry", "rz", "r1"}:
                if len(args) != 2:
                    raise ValueError(f"{name} expects angle and qubit.")
                angle = self._eval_number(args[0])
                qubits = [self._parse_qubit(args[1])]
                ops.append(Operation(gate, qubits, [angle]))
            else:
                qubits = [self._parse_qubit(x) for x in args]
                ops.append(Operation(gate, qubits))

        circuit = Circuit(
            num_qubits=num_qubits,
            operations=ops,
            measurements=measurements,
            metadata={"source": "cuda-q-python", "adapter": "oqci-cudaq-frontend"},
        )
        circuit.validate()
        return circuit

    @staticmethod
    def _call_name(func: ast.AST) -> str | None:
        if isinstance(func, ast.Name):
            return func.id
        if isinstance(func, ast.Attribute):
            return func.attr
        return None

    @classmethod
    def _gate_from_call(cls, node: ast.Call) -> str | None:
        # CUDA-Q commonly expresses CNOT as x.ctrl(control, target).
        if isinstance(node.func, ast.Attribute) and node.func.attr == "ctrl":
            base = node.func.value
            if isinstance(base, ast.Name) and base.id.lower() == "x":
                return "cx"
            if isinstance(base, ast.Name) and base.id.lower() == "z":
                return "cz"
            return None

        name = cls._call_name(node.func)
        return cls.GATE_ALIASES.get(name.lower()) if name else None

    @classmethod
    def _find_qvector_size(cls, tree: ast.AST) -> int | None:
        for node in ast.walk(tree):
            if isinstance(node, ast.Call):
                name = cls._call_name(node.func)
                if name == "qvector" and len(node.args) == 1:
                    value = cls._eval_number(node.args[0])
                    if int(value) != value or value < 1:
                        raise ValueError("qvector size must be a positive integer.")
                    return int(value)
        return None

    @staticmethod
    def _parse_qubit(node: ast.AST) -> int:
        # q[0]
        if isinstance(node, ast.Subscript):
            index = node.slice
            if isinstance(index, ast.Constant) and isinstance(index.value, int):
                return int(index.value)
            # Python 3.11 compatibility for a simple literal index.
            if isinstance(index, ast.Index):  # pragma: no cover
                return int(index.value.value)
        raise ValueError("Expected a simple CUDA-Q qubit expression such as q[0].")

    @classmethod
    def _parse_measurement(cls, node: ast.Call) -> list[int]:
        if len(node.args) != 1:
            raise ValueError("mz expects q or q[i].")
        arg = node.args[0]
        if isinstance(arg, ast.Subscript):
            return [cls._parse_qubit(arg)]
        # mz(q) means all allocated qubits.
        return []

    @staticmethod
    def _eval_number(node: ast.AST) -> float:
        if isinstance(node, ast.Constant) and isinstance(node.value, (int, float)):
            return float(node.value)

        if isinstance(node, ast.Attribute) and isinstance(node.value, ast.Name):
            if node.value.id == "math" and node.attr == "pi":
                return math.pi

        if isinstance(node, ast.UnaryOp) and isinstance(node.op, (ast.USub, ast.UAdd)):
            value = CudaQFrontend._eval_number(node.operand)
            return -value if isinstance(node.op, ast.USub) else value

        if isinstance(node, ast.BinOp):
            left = CudaQFrontend._eval_number(node.left)
            right = CudaQFrontend._eval_number(node.right)
            if isinstance(node.op, ast.Add):
                return left + right
            if isinstance(node.op, ast.Sub):
                return left - right
            if isinstance(node.op, ast.Mult):
                return left * right
            if isinstance(node.op, ast.Div):
                return left / right

        raise ValueError("Only numeric literals and simple math.pi expressions are supported.")
