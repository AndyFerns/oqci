# Running on IBM Quantum hardware

OQCI compiles for, submits to and retrieves from IBM Quantum devices. The
supported devices are the CZ-native Heron family, through
[`oqci.backends.ibm`](../python/oqci/backends/ibm.py).

This page covers:

- how the path works, and what it does and does not claim;
- the one-off run of a single program;
- the Stage G hardware experiment.

## How it works

1. **The target comes from the device.**
   `ibm.target_from_backend(backend)` reads the backend's live `Target`. It
   records the qubit count, the CZ couplings, and support for reset and
   mid-circuit measurement, and builds an OQCI basis profile with the
   `{rz, sx, x, cz}` rule set. Compiling for that profile lowers against the
   device's real coupling map.
   - Qubits missing any basis operation get no couplings, so routing never
     uses them.
   - ECR-native (Eagle) devices are refused by name. ECR is not in OQCI's
     gate set.
2. **Nothing is transpiled.** IBM Runtime accepts only circuits already in a
   device's instruction set (ISA). `ibm.check_isa` checks every instruction
   and physical qubit against the backend's `Target`, and refuses rather than
   rewrites. The executable is submitted verbatim.
3. **Nothing is inserted around it.** Submission uses the client-side
   `executor_sampler.Sampler`, which replaces the deprecated `SamplerV2` in
   `qiskit-ibm-runtime` 0.50. Gate twirling, measurement twirling and
   dynamical decoupling are switched off explicitly, and the submission
   record says so.
4. **Results come back structured.**
   - Counts use the shared key convention (clbit 0 rightmost).
   - Queue time and execution time come from the job's own timestamps, kept
     separately (Stage C §8).
   - `oqci.backends.to_execution_result` converts the result into OQCI's
     `ExecutionResult`.

**Credentials** are only ever your own saved account. Save it once, yourself:

```python
from qiskit_ibm_runtime import QiskitRuntimeService
QiskitRuntimeService.save_account(token="...", channel="ibm_quantum_platform")
```

No OQCI code takes a token, and nothing OQCI writes contains one.

**Verified against** `qiskit-ibm-runtime` 0.50.0 (the `ibm` extra pins
`>=0.50.0,<0.51`) and Qiskit 2.5. Everything except the network call is tested
offline, through IBM's local testing mode on `FakeFez`, a Heron snapshot
(`python/tests/test_backend_ibm.py`).

**What is claimed:** the executable was lowered to, and validated against,
the device's own reported target at the time of the snapshot, and it ran
unmodified.

**What is not claimed:** that its counts are noise-free. Hardware has noise,
and the experiment below measures how much.

## One program

```bash
python -m oqci run examples/bell.qasm --runtime ibm --ibm-backend ibm_fez --shots 4000
python -m oqci run examples/bell.qasm --runtime ibm --shots 4000            # least busy CZ-native device
python -m oqci run examples/bell.qasm --runtime ibm --fake FakeFez          # offline rehearsal
```

## The Stage G hardware experiment

The protocol is locked in
[`stage-g-benchmarking-protocol.md`](core_architecture/stage-g-benchmarking-protocol.md)
§5. It runs seven circuits:

- Bell(2), GHZ(3), GHZ(5);
- Grover(3);
- VQE(4, one layer);
- random(4, instance 0);
- QFT(4) followed by its inverse on `|0101⟩`.

Each circuit is compiled by OQCI and by Qiskit O3, both against the same
device snapshot's coupling map and basis. That makes 14 circuits in one job,
at 4096 shots each, well inside a 10-QPU-minute budget. The reported metric is
the Hellinger fidelity of the counts to the exact distribution, with no
pass/fail threshold.

Each step is a separate command, so they can run hours apart. From
`benchmarks/`:

```bash
python -m oqci_bench hardware snapshot --out results/hw-ibm         # add --backend ibm_fez to pick one
python -m oqci_bench hardware compile  --out results/hw-ibm
python -m oqci_bench hardware submit   --out results/hw-ibm         # prints the job id
python -m oqci_bench hardware retrieve --out results/hw-ibm         # once the job has finished
```

What each step produces:

- **`snapshot`** records the device target (`snapshot.json`).
- **`compile`** writes the 14 compiled circuits. OQCI's are semantically
  verified during compilation.
- **`submit`** checks every circuit against the device's ISA before
  submitting anything.
- **`retrieve`** writes `results.json`.

To get the report, copy or move `results/hw-ibm` under the reference run's
`hardware/` directory and rerun `python -m oqci_bench report <run>`. It adds a
hardware table of Hellinger fidelity and total-variation distance per circuit
and compiler.

To rehearse the whole flow offline, add `--fake FakeFez` to every step. This
is also Stage G's secondary noisy-simulation data, since the fake backend
carries the device's noise model.
