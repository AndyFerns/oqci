"""Run an OQCI executable on IBM Quantum hardware (CZ-native, Heron-family).

This closes the last step of the IBM path. The Rust compiler lowers,
validates and prepares; this adapter submits, retrieves and reports. It
follows the layout in ``docs/adapters.md``, with two extra steps that real
hardware needs:

1. **The target comes from the device.** :func:`target_from_backend` turns a
   backend's live ``Target`` into an OQCI basis profile. Compile for that
   profile with ``oqci.compile(..., target=profile)``, so the executable is
   lowered against the device's actual coupling map and basis rather than an
   illustration of one.
2. **Nothing is transpiled.** IBM Runtime only accepts circuits already in a
   device's instruction set architecture (ISA). OQCI's lowering produces
   exactly that, so the executable is submitted *verbatim*, with twirling
   and dynamical decoupling disabled. :func:`check_isa`
   checks every instruction and its physical qubits against the backend's
   ``Target`` and refuses on any mismatch; it never rewrites. The circuit that
   runs is the one OQCI verified.

Only CZ-native backends are supported (IBM's Heron family). A backend whose
two-qubit gate is ``ecr`` (Eagle) is refused by :func:`target_from_backend`:
ECR is not in OQCI's gate set.

**Credentials** come only from the caller's own saved account or
environment (``QiskitRuntimeService()`` with no arguments reads them). This
module never takes a token, and nothing it writes contains one
(``final-deliverables-spec.md`` §29).

**Verified against** ``qiskit-ibm-runtime`` 0.50.0 (``QiskitRuntimeService``,
the client-side ``executor_sampler.Sampler(mode=...).run(pubs, shots=...)``
that replaces the deprecated ``SamplerV2``, ``RuntimeJobV2.metrics()``,
``service.job(job_id)``) and Qiskit 2.5 (``Target.instruction_supported``).
Gate twirling, measurement twirling and dynamical decoupling are switched
off explicitly — their defaults are left to the service otherwise — so the
device executes OQCI's circuit and nothing inserted around it.
The offline tests run the whole path through IBM's local testing mode on a
Heron fake backend (``FakeFez``), which executes on Aer.

Counts use the shared convention (one character per classical bit, clbit 0
rightmost), which is also Qiskit's, so they are returned unconverted. Queue
time and execution time are reported separately and never derived from each
other (Stage C §8); compilation time is not this module's to report.
"""

from __future__ import annotations

import datetime as _dt
import time
from dataclasses import dataclass
from typing import Any, Mapping, Optional

from ._common import RunResult, UnsupportedOperation

#: The OQCI operations a CZ-native IBM device can run, by Target name.
_SUPPORTED = ("cz", "rz", "sx", "x", "measure", "reset")

#: The cost model a device profile names. The same one the synthetic
#: IBM-shaped targets use; it weighs counts, not calibration data.
_COST_MODEL = "nisq-weighted"


class IsaViolation(UnsupportedOperation):
    """The executable is not in the backend's instruction set architecture.

    Raised instead of transpiling: rewriting the circuit here would run
    something other than what OQCI verified.
    """


def target_from_backend(backend: Any) -> dict[str, Any]:
    """Build an OQCI basis profile from an IBM backend's ``Target``.

    The profile describes what the device reports *now*: its qubit count, its
    CZ couplings (both directions, as the target declares them), and whether
    it supports reset and mid-circuit measurement. Qubits missing any of
    ``rz``, ``sx``, ``x`` or ``measure`` are left with no couplings, so
    routing never moves a qubit through them; :func:`check_isa` still refuses
    an executable that places work on one.

    The returned dict is what ``oqci.compile(..., target=...)`` and
    ``oqci lower --target-file`` take. It records the backend's name and
    version in ``capabilities`` so the provenance of anything compiled for it
    names the device snapshot it was compiled against.

    :raises UnsupportedOperation: if the backend has no ``cz`` (for example an
        ECR-native Eagle device).
    """
    target = backend.target
    names = set(target.operation_names)
    if "cz" not in names:
        two_qubit = sorted(n for n in names if n in ("ecr", "cx", "rzz"))
        raise UnsupportedOperation(
            f"backend `{backend.name}` is not CZ-native (two-qubit operations: "
            f"{two_qubit or 'none'}); OQCI supports CZ-native IBM devices only"
        )

    def has(name: str, qubit: int) -> bool:
        return name in names and target.instruction_supported(name, (qubit,))

    num_qubits = int(target.num_qubits)
    usable = {
        q for q in range(num_qubits) if all(has(n, q) for n in ("rz", "sx", "x", "measure"))
    }
    edges = sorted(
        (int(a), int(b))
        for (a, b) in (target.qargs_for_operation_name("cz") or ())
        if int(a) in usable and int(b) in usable
    )

    from .. import basis_rule_sets

    version = getattr(backend, "backend_version", None) or "unknown"
    return {
        "id": str(backend.name),
        "version": str(version),
        "backend_id": "ibm",
        "topology": {"qubit_count": num_qubits, "edges": edges},
        "supported_operations": sorted(
            op for op in _SUPPORTED if op in names
        ),
        "parameter_constraints": {},
        "decomposition_rules": basis_rule_sets()["cz"],
        "measurement": {
            "measurement": "measure" in names,
            # Heron targets advertise dynamic-circuit support through these;
            # OQCI compiles no classical control, so it only needs to know a
            # measured qubit may be reused.
            "mid_circuit_measurement": "measure" in names and "if_else" in names,
            "reset": "reset" in names,
        },
        "capabilities": sorted(
            {
                "cz-native",
                f"device:{backend.name}",
                f"device-version:{version}",
                f"usable-qubits:{len(usable)}/{num_qubits}",
            }
        ),
        "cost_model_id": _COST_MODEL,
    }


def to_isa_circuit(executable: Mapping[str, Any], backend: Any):
    """The executable as a Qiskit circuit over the backend's physical qubits.

    The executable's qubit indices are already physical (lowering placed
    them). The circuit is padded to the device width so index ``i`` *is*
    physical qubit ``i`` when the backend reads it.
    """
    from qiskit import QuantumCircuit

    from .aer import to_qiskit

    narrow = to_qiskit(executable)
    width = int(backend.target.num_qubits)
    if narrow.num_qubits > width:
        raise IsaViolation(
            f"the executable uses {narrow.num_qubits} qubits but `{backend.name}` has {width}"
        )
    wide = QuantumCircuit(width, narrow.num_clbits)
    wide.compose(narrow, qubits=range(narrow.num_qubits), clbits=range(narrow.num_clbits),
                 inplace=True)
    return wide


def check_isa(circuit: Any, backend: Any) -> None:
    """Refuse a circuit with any instruction the backend does not support on
    those physical qubits. Checks; never rewrites.

    :raises IsaViolation: naming every offending instruction.
    """
    target = backend.target
    problems = []
    for index, instruction in enumerate(circuit.data):
        name = instruction.operation.name
        qargs = tuple(circuit.find_bit(q).index for q in instruction.qubits)
        if name == "barrier":
            continue
        if not target.instruction_supported(name, qargs):
            problems.append(f"#{index} {name}{list(qargs)}")
    if problems:
        shown = ", ".join(problems[:10])
        more = f" (and {len(problems) - 10} more)" if len(problems) > 10 else ""
        raise IsaViolation(
            f"not in `{backend.name}`'s instruction set: {shown}{more}. Compile for "
            "target_from_backend(backend) so lowering targets this device."
        )


@dataclass
class IbmSubmission:
    """A submitted job: enough to retrieve it later, from another process."""

    job_id: str
    backend_name: str
    shots: int
    #: When the job was handed to IBM Runtime (UTC, ISO 8601).
    submitted_at: str
    #: The executable's provenance, updated to the shots actually requested.
    provenance: dict[str, Any]
    #: The primitive options that were set, so the record says how it ran.
    options: dict[str, Any]

    def to_dict(self) -> dict[str, Any]:
        return {
            "job_id": self.job_id,
            "backend_name": self.backend_name,
            "shots": self.shots,
            "submitted_at": self.submitted_at,
            "provenance": self.provenance,
            "options": self.options,
        }


@dataclass
class IbmResult(RunResult):
    """What IBM Runtime returned.

    ``backend_metadata`` carries the job id, backend name and the job's own
    metrics (timestamps rendered as ISO 8601 text). ``queue_duration_ms`` is
    the time between creation and running as IBM reports it;
    ``execution_duration_ms`` the time between running and finishing. Either
    is ``None`` when the job did not report the timestamps it needs. Local
    testing mode reports all three timestamps as the same instant, so both
    durations are zero there — what the runtime said, not a measurement of
    hardware.
    """

    queue_duration_ms: Optional[float] = None


def submit(
    executable: Mapping[str, Any],
    backend: Any,
    *,
    shots: Optional[int] = None,
) -> tuple[Any, IbmSubmission]:
    """Submit one executable to ``backend`` with ``SamplerV2``.

    ``backend`` is an ``IBMBackend`` from ``QiskitRuntimeService`` — or, for
    an offline run, a fake backend, which IBM Runtime executes locally on Aer.
    Returns the job and a serializable :class:`IbmSubmission`.

    :raises IsaViolation: if the executable is not valid for this backend as
        written. Nothing is submitted in that case.
    """
    from qiskit_ibm_runtime.executor_sampler import Sampler

    settings = executable.get("settings") or {}
    shots = int(settings.get("shots", 1024)) if shots is None else int(shots)
    if not any(op["op"] == "measure" for op in executable["ops"]):
        raise UnsupportedOperation(
            "this executable has no measurement, so it would produce no counts"
        )
    circuit = to_isa_circuit(executable, backend)
    check_isa(circuit, backend)

    sampler = Sampler(mode=backend)
    # Execute exactly the circuit OQCI produced: no Pauli frames inserted
    # around gates or measurements, no idle-time pulse sequences.
    options = {
        "twirling.enable_gates": False,
        "twirling.enable_measure": False,
        "dynamical_decoupling.enable": False,
    }
    sampler.options.twirling.enable_gates = False
    sampler.options.twirling.enable_measure = False
    sampler.options.dynamical_decoupling.enable = False
    job = sampler.run([circuit], shots=shots)
    provenance = dict(executable.get("provenance") or {})
    provenance["shots"] = shots
    # Hardware has no seed; record that none was used rather than echo one.
    provenance["seed"] = None
    submission = IbmSubmission(
        job_id=str(job.job_id()),
        backend_name=str(backend.name),
        shots=shots,
        submitted_at=_dt.datetime.now(_dt.timezone.utc).isoformat(),
        provenance=provenance,
        options=options,
    )
    return job, submission


def _jsonable(value: Any) -> Any:
    """``value`` with every datetime as ISO text and anything else exotic as
    ``str``, so job metadata survives JSON serialization intact."""
    if isinstance(value, Mapping):
        return {str(k): _jsonable(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [_jsonable(v) for v in value]
    if isinstance(value, (_dt.datetime, _dt.date)):
        return value.isoformat()
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    return str(value)


def _parse_time(value: Any) -> Optional[_dt.datetime]:
    if not value:
        return None
    try:
        return _dt.datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    except ValueError:
        return None


def _durations(metrics: Mapping[str, Any]) -> tuple[Optional[float], Optional[float]]:
    """Queue and execution time in ms, from a job's reported timestamps."""
    stamps = metrics.get("timestamps") or {}
    created = _parse_time(stamps.get("created"))
    running = _parse_time(stamps.get("running"))
    finished = _parse_time(stamps.get("finished"))
    queue = (running - created).total_seconds() * 1000.0 if created and running else None
    execution = (finished - running).total_seconds() * 1000.0 if running and finished else None
    return queue, execution


def collect(job: Any, submission: IbmSubmission, *, timeout: Optional[float] = None) -> IbmResult:
    """Wait for a job and turn its result into an :class:`IbmResult`."""
    started = time.perf_counter()
    primitive = job.result(timeout=timeout) if timeout is not None else job.result()
    waited_ms = (time.perf_counter() - started) * 1000.0

    data = primitive[0].data
    registers = [name for name in dir(data) if not name.startswith("_")]
    # `to_qiskit` builds a single classical register, `c`.
    bits = getattr(data, "c", None)
    if bits is None:
        raise UnsupportedOperation(
            f"the job returned no classical register `c` (found: {registers})"
        )
    counts = {str(k): int(v) for k, v in bits.get_counts().items()}

    try:
        metrics = _jsonable(dict(job.metrics()))
    except Exception:  # noqa: BLE001 — a job type without a metrics endpoint
        metrics = {}
    queue_ms, execution_ms = _durations(metrics)

    metadata: dict[str, Any] = {
        "job_id": submission.job_id,
        "backend_name": submission.backend_name,
        "submitted_at": submission.submitted_at,
        "options": submission.options,
        "metrics": metrics,
        "result_metadata": _jsonable(dict(getattr(primitive, "metadata", {}) or {})),
    }
    if not metrics:
        # No server-side timing (local testing mode): record the client's
        # wait, labeled as such, and claim no queue or execution split.
        metadata["client_wait_ms"] = waited_ms

    return IbmResult(
        counts=counts,
        memory=None,
        backend_metadata=metadata,
        execution_duration_ms=execution_ms,
        provenance=dict(submission.provenance),
        queue_duration_ms=queue_ms,
    )


def retrieve(submission: Mapping[str, Any], *, service: Any = None,
             timeout: Optional[float] = None) -> IbmResult:
    """Retrieve a job submitted earlier, possibly from another process.

    ``submission`` is :meth:`IbmSubmission.to_dict`'s output. ``service``
    defaults to ``QiskitRuntimeService()`` — the caller's saved account.
    """
    if service is None:
        from qiskit_ibm_runtime import QiskitRuntimeService

        service = QiskitRuntimeService()
    job = service.job(str(submission["job_id"]))
    record = IbmSubmission(
        job_id=str(submission["job_id"]),
        backend_name=str(submission["backend_name"]),
        shots=int(submission["shots"]),
        submitted_at=str(submission["submitted_at"]),
        provenance=dict(submission.get("provenance") or {}),
        options=dict(submission.get("options") or {}),
    )
    return collect(job, record, timeout=timeout)


def run(
    executable: Mapping[str, Any],
    *,
    backend: Any,
    shots: Optional[int] = None,
    seed: Optional[int] = None,
    timeout: Optional[float] = None,
) -> IbmResult:
    """Submit and wait: :func:`submit` then :func:`collect`.

    ``seed`` exists for the shared adapter signature and must be ``None``:
    hardware cannot be seeded, and accepting one silently would record a
    reproducibility the run does not have.
    """
    if seed is not None:
        raise UnsupportedOperation("IBM hardware cannot be seeded; pass seed=None")
    job, submission = submit(executable, backend, shots=shots)
    return collect(job, submission, timeout=timeout)


def least_busy_cz_backend(service: Any, *, min_num_qubits: int = 1) -> Any:
    """The least busy operational, CZ-native backend the account can use.

    The default selection rule of the Stage G hardware protocol; a caller
    may always name a backend instead.
    """
    return service.least_busy(
        min_num_qubits=min_num_qubits,
        filters=lambda b: (
            not b.configuration().simulator
            and b.status().operational
            and "cz" in b.target.operation_names
        ),
    )
