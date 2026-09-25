from __future__ import annotations

import math
import types
from typing import Any

from .ir import Circuit, Operation


class CudaQBackend:
    """
    OQCI Unified IR -> CUDA-Q backend adapter.

    It generates a CUDA-Q Python kernel from the OQCI IR and executes it
    through cudaq.sample(). The generated source can also be inspected,
    which is useful for compiler debugging and demonstrations.
    """

    def __init__(self, target: str = "qpp-cpu"):
        self.target = target

    def _import_cudaq(self):
        try:
            import cudaq
        except ImportError as exc:
            raise RuntimeError(
                "CUDA-Q is not installed. Install it with: python -m pip install cudaq"
            ) from exc
        return cudaq

    def set_target(self):
        cudaq = self._import_cudaq()
        cudaq.set_target(self.target)

    def generate_source(self, circuit: Circuit, kernel_name: str = "oqci_kernel") -> str:
        circuit.validate()
        lines = [
            "import cudaq",
            "",
            f"@cudaq.kernel",
            f"def {kernel_name}():",
            f"    q = cudaq.qvector({circuit.num_qubits})",
        ]

        for op in circuit.operations:
            lines.extend(self._emit_operation(op))

        if circuit.measurements is not None:
            if not circuit.measurements:
                lines.append("    mz(q)")
            else:
                for q in circuit.measurements:
                    lines.append(f"    mz(q[{q}])")
        else:
            lines.append("    mz(q)")

        return "\n".join(lines) + "\n"

    @staticmethod
    def _emit_operation(op: Operation) -> list[str]:
        name = op.name.lower()
        q = op.qubits
        if name in {"h", "x", "y", "z", "s", "sdg", "t", "tdg"}:
            return [f"    {name}(q[{q[0]}])"]
        if name == "cx":
            return [f"    x.ctrl(q[{q[0]}], q[{q[1]}])"]
        if name == "cz":
            return [f"    z.ctrl(q[{q[0]}], q[{q[1]}])"]
        if name == "swap":
            return [f"    swap(q[{q[0]}], q[{q[1]}])"]
        if name in {"rx", "ry", "rz", "r1"}:
            return [f"    {name}({op.params[0]!r}, q[{q[0]}])"]
        if name == "measure":
            return [f"    mz(q[{q[0]}])"]
        raise ValueError(f"Unsupported operation: {op.name}")

    def compile(self, circuit: Circuit, kernel_name: str = "oqci_kernel"):
        """
        Compile an OQCI Circuit into a live CUDA-Q kernel object.
        """
        source = self.generate_source(circuit, kernel_name)
        cudaq = self._import_cudaq()

        namespace = {"cudaq": cudaq}
        exec(compile(source, "<oqci-cudaq-generated>", "exec"), namespace, namespace)
        return namespace[kernel_name]

    def run(self, circuit: Circuit, shots: int = 1000) -> dict[str, Any]:
        kernel = self.compile(circuit)
        self.set_target()
        cudaq = self._import_cudaq()
        result = cudaq.sample(kernel, shots_count=shots)

        # CUDA-Q Result objects are iterable/mapping-like in current releases,
        # but the exact display API can evolve. Convert conservatively.
        counts: dict[str, int] = {}
        try:
            for key, value in result.items():
                counts[str(key)] = int(value)
        except AttributeError:
            try:
                for key in result:
                    counts[str(key)] = int(result[key])
            except Exception:
                counts = {"result": str(result)}

        return {
            "backend": self.target,
            "shots": shots,
            "counts": counts,
            "kernel": "oqci_kernel",
        }

    def export_source(self, circuit: Circuit, path: str) -> None:
        with open(path, "w", encoding="utf-8") as f:
            f.write(self.generate_source(circuit))
