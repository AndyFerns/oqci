"""What every execution adapter shares.

An execution adapter takes the ``executable`` dict a backend-targeted
``oqci.compile`` returns and runs it on one runtime. Each adapter module
(``aer``, ``cudaq``, …) exposes the same shape:

- ``run(executable, *, shots=None, seed=None, …)`` returning a
  :class:`RunResult` subclass;
- a pure, runtime-free translation function (``to_qiskit``,
  ``to_cudaq_source``) so the artifact that will run can be inspected without
  running it;
- :class:`UnsupportedOperation` for anything it cannot replay faithfully.

Counts use one key convention across adapters — Qiskit's: the key has one
character per classical bit and **clbit 0 is the rightmost character**. An
adapter whose runtime reports bits differently converts before returning, so
results from two runtimes can be compared key by key.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Optional


class UnsupportedOperation(RuntimeError):
    """An executable asked for something an adapter cannot replay faithfully.

    Raised rather than skipped. Dropping or approximating an operation would
    run a different circuit from the one OQCI verified, and the counts would
    look perfectly reasonable.
    """


@dataclass
class RunResult:
    """What a runtime returned, with the provenance of what produced it."""

    #: Measured bitstrings and how often each occurred, keyed by the shared
    #: convention in this module's docstring (clbit 0 rightmost).
    counts: dict[str, int]
    #: Per-shot outcomes, when requested and supported.
    memory: Optional[list[str]] = None
    #: The runtime's own metadata, verbatim.
    backend_metadata: dict[str, Any] = field(default_factory=dict)
    #: Wall-clock time inside ``run``, in milliseconds.
    #:
    #: Kept separate from any compilation timing. Stage C §8 forbids
    #: conflating the two, and this number covers execution only — the
    #: compiler had already finished before the executable reached here.
    execution_duration_ms: Optional[float] = None
    #: Everything needed to cite this result, carried from the executable.
    provenance: dict[str, Any] = field(default_factory=dict)

    @property
    def observed_shots(self) -> int:
        """Shots actually observed, summed from the counts.

        Derived from the data rather than echoed from the request, so a run
        that returned fewer shots than were asked for is visible.
        """
        return sum(self.counts.values())

    def probabilities(self) -> dict[str, float]:
        """The counts as frequencies."""
        total = self.observed_shots
        if total == 0:
            return {}
        return {bits: count / total for bits, count in self.counts.items()}
