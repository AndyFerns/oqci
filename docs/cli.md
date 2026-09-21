# The `oqci` CLI

Status: normative
Implemented by: `src/cli/`

A way to see what the compiler actually produced at every stage, for a real
program — and, in `watch` mode, to keep seeing it as you edit.

## The one rule

`final-deliverables-spec.md` §19: the CLI "must not duplicate compiler logic
that belongs in the Rust library — it should invoke the same public compiler
APIs."

That is structural here, not aspirational — and it got stronger in `0.3.0`.
The CLI used to call the frontend, the pass manager and the emitters itself,
in the right order. It now calls `crate::compile`, the orchestrator, and does
nothing but render what comes back:

| What you see | Who computed it |
|---|---|
| parsed and bound circuit | `compile::compile_named` → `frontend`, `ir::bind_parameters` |
| pass table | `compile::compile_named` → `pass::PassManager` |
| every metric | `analysis::analyze` |
| the diff | `analysis::diff_circuits` |
| dependency graph | `ir::qc_to_qco` |
| QIR text | `ir::emit_qir` |
| layout, SWAP count, rules applied | `lowering::lower` |
| legality and cost | `target::check`, the backend's own `CostModel` |
| the executable | `backend::Executable::from_lowered` |

The difference matters because the CLI is no longer the only caller. The
Python SDK and library callers take the same path, so there is exactly one
implementation of "what compiling means" and no way for the tool's view of a
circuit to drift from the compiler's — see [`compiler.md`](compiler.md).

One visible consequence: the CLI no longer imports the frontend at all. When
that refactor landed, the compiler stopped building until the now-unused
`parse_openqasm3_named` import was removed, which is about as direct a proof
of the property as one gets.

No gate is interpreted, no metric recomputed, no circuit rewritten in the CLI.

## Commands

```text
oqci compile  <input.qasm> [--emit STAGES] [--bind NAME=VALUE] [--target ID] [--json]
oqci optimize <input.qasm> [--emit STAGES] [--passes IDS] [--disable IDS] [--diff] [--bind …] [--target ID] [--json]
oqci analyze  <input.qasm> [--optimized] [--bind …] [--target ID] [--json]
oqci lower    <input.qasm> --backend ID [--layout trivial|dense] [--no-route]
                                          [--no-decompose] [--emit STAGES] [--diff]
                                          [--bind …] [--json]
oqci prepare  <input.qasm> --backend ID [--layout …] [--shots N] [--seed N]
                                          [-o FILE] [--bind …]
oqci watch    <input.qasm> [--mode compile|optimize] [...same flags] [--json]
oqci passes
oqci targets
oqci backends
```

Input is OpenQASM 3 (see [`openqasm_frontend.md`](openqasm_frontend.md)). The
Qiskit adapter needs a live Python interpreter and stays reachable through
`oqci_native` from Python — see [`qiskit_adapter.md`](qiskit_adapter.md).

`--emit` takes any comma-separated subset of `qc-ir`, `qco-ir`, `qir`; the
default is all three. The circuit is named after the file stem, so the QIR
entry point matches the file you are looking at.

### compile

```console
$ oqci compile examples/bell.qasm
== examples/bell.qasm (bell via openqasm3)

-- qc-ir -- 2 qubit(s), 2 clbit(s), 4 op(s), depth 3
    0  h %q0
    1  cx %q0, %q1
    2  measure %q0 -> %c0
    3  measure %q1 -> %c1

-- qco-ir --
  4 op node(s), depth 3, max parallel width 2
  layer 0: 0
  layer 1: 1
  layer 2: 2, 3

-- qir --
  ; QIR module for circuit `bell`
  …
```

Layers group operations that are **provably independent** — no path in the
dependency graph connects them. A `control barriers:` section appears when a
measurement or reset constrains a later operation.

### optimize

```console
$ oqci optimize examples/ghz3.qasm --diff --emit qc-ir
-- passes --
  [  --] canonicalize       9 -> 9 ops, depth 4 -> 4
  [  ok] gate-cancellation  9 -> 7 ops, depth 4 -> 4
           cancelled 1 inverse pair(s)
  [  --] rotation-merge     7 -> 7 ops, depth 4 -> 4
  …

-- diff --
    h %q0
  - x %q2
    h %q1
  - x %q2
    cx %q0, %q1
```

Status markers: `ok` changed the circuit, `--` ran and changed nothing,
`skip` was disabled.

Note what the diff shows: the two `x %q2` gates cancelled *despite* the
`h %q1` between them, because that gate is on an independent wire. See
[`pass_manager.md`](pass_manager.md#adjacency-what-adjacent-means).

**Ablation.** `--disable gate-cancellation` runs everything else and still
reports the skipped pass, which is the shape an ablation study needs.
`--passes` and `--disable` are mutually exclusive, and an unknown id is
rejected with the list of valid ones rather than silently running a different
pipeline than you asked for.

### analyze

```console
$ oqci analyze examples/bell.qasm
  qubits              2
  operations          4
  one-qubit gates     1
  multi-qubit gates   1
  depth               3
  by operation:
    cx         1
    h          1
    measure    2
```

`--optimized` measures the circuit after the pass pipeline instead of the
parsed one.

### watch

```console
$ oqci watch examples/ghz3.qasm --mode optimize --diff
watching examples/ghz3.qasm — press Ctrl+C to stop
```

Re-runs on every save and re-renders. Two details make it usable:

- It watches the **directory**, not the file, because many editors save by
  writing a temporary file and renaming it over the original — which replaces
  the inode and would leave a file-level watch deaf after the first save.
- A parse error prints the diagnostic and **keeps watching**. A file being
  edited is malformed most of the time; exiting exactly when you are
  mid-edit would defeat the purpose.

### lower

Compiles a program *onto a device* and shows the whole schedule. This is the
command that exercises every stage the compiler has.

```bash
oqci lower examples/ghz3.qasm --backend simulator-nisq
```

```text
-- lowering -- backend `simulator-nisq`, target linear-nisq@1
  layout   %q0->#q0, %q1->#q1, %q2->#q2 -> %q0->#q0, %q1->#q1, %q2->#q2
  routing  0 swap(s) inserted, 0 orientation(s) repaired
  rules    h-to-rz-sx

  steps:
    arity-reduction           7 op(s)  0 wide gate(s) reduced
    layout                    7 op(s)  trivial layout: %q0->#q0, %q1->#q1, %q2->#q2
    routing                   7 op(s)  0 swap(s) inserted
    basis-decomposition      11 op(s)  2 operation(s) rewritten
    orientation-repair       11 op(s)  0 reversed operation(s) repaired
    single-qubit-cleanup     11 op(s)  0 operation(s) rewritten
    verify                   11 op(s)  legal

  legal for this target
```

Both layouts are printed because routing moves qubits: the initial one says
where each logical qubit started, the final one where it ended up, and without
the second you cannot tell which physical wire a measurement result came from.

`--backend` is required and is not the same flag as `--target`. `--target`
*checks* a circuit against a profile and reports; `--backend` selects a device
and actually compiles for it. See `oqci backends` for the list.

`--layout` picks the initial placement. `trivial` puts logical *n* on physical
*n*; `dense` seats interacting qubits near each other. Layout choice can only
change how many SWAPs routing needs — it cannot make a circuit incorrect, so
this is a cost knob, not a correctness one.

`--no-route` and `--no-decompose` are **inspection aids, not compilation
modes**. They let you see the circuit at an intermediate point, and the result
is usually illegal. The report says so rather than pretending otherwise:

```bash
oqci lower far.qasm --backend simulator-nisq --no-route
```

```text
  routing  0 swap(s) inserted, 0 orientation(s) repaired
  ...
  NOT legal: 1 violation(s)
    [3] no coupling #q0 -> #q2 on this device
```

(The index is `3` rather than `1` because decomposition still ran: the `h`
became three gates before the `cx` was reached.)

Lowering can also *refuse*, and every refusal names what it could not fix —
a circuit wider than the device, two qubits in different connected components,
an operation with no decomposition rule, a symbolic parameter a rule would
have had to transform. See [`lowering.md`](lowering.md) for the full table.

### prepare

Lowers a program and writes the executable a backend would run.

```bash
oqci prepare examples/bell.qasm --backend simulator-nisq --shots 512 --seed 7
```

```json
{
  "backend_id": "simulator-nisq",
  "num_qubits": 2,
  "num_clbits": 2,
  "ops": [
    { "op": "rz", "qubits": [0], "params": [1.5707963267948966], "clbit": null },
    { "op": "sx", "qubits": [0], "params": [], "clbit": null },
    …
  ],
  "settings": { "shots": 512, "seed": 7, "memory": false },
  "provenance": { "backend_id": "simulator-nisq", "profile_id": "linear-nisq@1", … }
}
```

`-o FILE` writes it to a file instead of stdout. The artifact is what
`oqci.backends.aer.run` consumes — see [`../python/README.md`](../python/README.md).

Two things about this output are deliberate. It is **not QIR**: Stage C §5
forbids treating emitted QIR as a guarantee that anything will run, so the
executable representation is a separate artifact from a separate stage. And it
names the backend it was prepared for, because an executable is not portable
between devices and the artifact should say so rather than leaving it to
convention.

`--seed` has no default. Choosing one would be picking an experimental
parameter that the benchmarking protocol owns (§33.15); it is recorded when
you supply it and absent when you do not.

`prepare` refuses a circuit with an unbound parameter, with advice rather than
a guess:

```text
error: instruction 0 is not executable: parameter `theta` is still symbolic;
       bind parameters before preparing for execution
```

### backends

```bash
oqci backends
```

```text
backends:
  simulator            unconstrained simulator: all-to-all connectivity, full gate set
  simulator-nisq       simulator constrained to a linear NISQ basis and topology
  ibm-illustrative     synthetic IBM-shaped target; describes no real device and cannot execute

No backend executes in this process.
`oqci prepare` writes the artifact; an execution adapter runs it.
```

That last line is the important one. The compiler prepares executables; it
does not run them. Qiskit Aer runs a prepared circuit through the Python
adapter, and live hardware submission is not implemented —
[`backend_contract.md`](backend_contract.md) explains why in full, including
why `ibm-illustrative` is synthetic and what is *not* being claimed about it.

### passes

Lists the default pipeline in execution order — the ids `--passes` and
`--disable` accept.

### targets

Lists the built-in target profiles — the ids `--target` accepts. Both are
synthetic; neither describes real hardware. See
[`target_model.md`](target_model.md).

### `--target ID`

Checks the circuit against a backend profile and costs it. On `optimize` the
**optimized** circuit is checked, since that is what would actually be
submitted.

```console
$ oqci compile examples/bell.qasm --target linear-nisq --emit qc-ir
-- target -- linear-nisq@1 on generic-nisq (5 qubits, 8 directed coupling(s))
  legality: 1 violation(s) — will not run as written
    [0] `h` is not in the target's basis set
  cost (nisq-weighted@1):
    operations        4
    …
    non-native ops    1
    scalar score      22
      from: depth_weight=1, gate_count_weight=1, non_native_weight=5, swap_weight=30, two_qubit_weight=10
```

Three things about that output are deliberate:

- **Every violation is listed, not just the first** — someone fixing a
  circuit wants the whole list.
- **The scalar never appears without its weights.** A cost number with
  undisclosed coefficients is not evidence (Stage E §6), so the
  configuration that produced it is printed alongside, and carried in
  `--json`.
- **"Will not run as written"** is the precise claim. There is no layout step
  yet, so logical qubit *n* is checked against physical qubit *n*. A
  connectivity violation means this circuit needs routing, not that the
  target can never run it.

An unknown id is rejected with the list of valid ones rather than silently
checking against nothing.

## Parameterized circuits

A circuit with unbound parameters is a normal state, not an error. `compile`
reports the free parameters and says why QIR is unavailable, rather than
failing:

```console
$ oqci compile examples/parameterized.qasm
   unbound parameter(s): phi, theta
…
-- qir --
  unavailable: circuit has unbound parameter(s): phi, theta; supply values with --bind NAME=VALUE
```

`--bind theta=1.5708 --bind phi=0.7854` supplies values through
`ir::bind_parameters`, after which QIR emits normally. This keeps `watch`
useful while an ansatz is still being written.

## JSON output

`--json` prints one `PipelineReport`. This is the schema a dashboard
consumes — everything the terminal shows is in it:

```json
{
  "source_path": "examples/bell.qasm",
  "frontend": "openqasm3",
  "circuit_name": "bell",
  "unbound_parameters": [],
  "stages": [
    {
      "stage": "qc-ir",
      "metrics": { "num_qubits": 2, "op_count": 4, "depth": 3, "gate_counts": {…} },
      "instructions": [
        { "index": 0, "op": "gate", "gate": "h", "qubits": [0], "text": "h %q0" }
      ]
    },
    { "stage": "qco-ir", "graph": { "depth": 3, "layers": [[0],[1],[2,3]], "edges": [...] } },
    { "stage": "qir", "qir": "; QIR module…" }
  ],
  "passes": [ { "id": "gate-cancellation", "changed": true, "before": {…}, "after": {…} } ],
  "diff": [ { "marker": "-", "text": "x %q2" } ]
}
```

`oqci lower --json` adds two more fields:

```json
{
  "lowering": {
    "backend": "simulator-nisq",
    "profile": "linear-nisq@1",
    "initial_layout": [0, 1, 2],
    "final_layout": [1, 0, 2],
    "swaps_inserted": 1,
    "orientations_repaired": 0,
    "rules_applied": ["h-to-rz-sx", "swap-to-cx"],
    "steps": [ { "id": "routing", "op_count": 9, "detail": "1 swap(s) inserted" } ],
    "legal": true,
    "violations": []
  },
  "executable": {
    "backend_id": "simulator-nisq",
    "operations": ["cx", "measure", "rz", "sx"],
    "has_measurement": true,
    "executable": { "ops": [...], "provenance": {...} }
  }
}
```

The nested `executable.executable` is the artifact itself — byte-identical to
what `oqci prepare` writes — with the fields around it a summary for a reader
who does not want to scan the operation list.

The layouts are arrays indexed by logical qubit: `"final_layout": [1, 0, 2]`
means logical `q0` ended on physical `#q1`.

Notes on the schema:

- `stage` is `"qc-ir"`, `"qco-ir"`, `"qir"`, or `"optimized"` (the
  post-pass circuit in an `optimize` run, where `"qc-ir"` is the input).
- Absent sections are **omitted**, not null.
- A gate parameter is tagged: `{"kind":"concrete","radians":0.5}` or
  `{"kind":"symbol","name":"theta"}`, so a consumer can distinguish a bound
  angle from a free one without parsing display text.
- `unavailable` on a stage explains why it produced nothing.
- `target` is present only when `--target` was given. It carries the profile's
  `id@version`, the legality verdict with every `violations` entry tagged by
  `kind`, the full `cost` breakdown, and `cost_model_configuration` — the
  weights behind `cost.scalar_score`.

The IR types themselves stay serde-free: these are view types built in
`src/cli/snapshot.rs`. The IR's shape is a compiler contract governed by
[`ir_spec.md`](ir_spec.md) and should not acquire a wire format by accident.

## Exit codes

`0` on success; `1` with a diagnostic on stderr otherwise. Unbound parameters
are not a failure.
