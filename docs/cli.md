# The `oqci` CLI

Status: normative
Implemented by: `src/cli/`

A way to see what the compiler actually produced at every stage, for a real
program — and, in `watch` mode, to keep seeing it as you edit.

## The one rule

`final-deliverables-spec.md` §19: the CLI "must not duplicate compiler logic
that belongs in the Rust library — it should invoke the same public compiler
APIs."

That is structural here, not aspirational. `src/cli/pipeline.rs` is the only
module that calls the compiler, and it does nothing but delegate:

| What you see | Who computed it |
|---|---|
| parsed circuit | `frontend::parse_openqasm3_named` |
| bound parameters | `ir::bind_parameters` |
| dependency graph | `ir::qc_to_qco` |
| QIR text | `ir::emit_qir` |
| pass table | `pass::PassManager` |
| every metric | `analysis::analyze` |
| the diff | `analysis::diff_circuits` |

No gate is interpreted, no metric recomputed, no circuit rewritten in the CLI.
Every number printed came from the same function the pass manager uses for its
own bookkeeping, so the tool's view of a circuit and the compiler's cannot
drift apart.

## Commands

```text
oqci compile  <input.qasm> [--emit STAGES] [--bind NAME=VALUE] [--json]
oqci optimize <input.qasm> [--emit STAGES] [--passes IDS] [--disable IDS] [--diff] [--bind …] [--json]
oqci analyze  <input.qasm> [--optimized] [--bind …] [--json]
oqci watch    <input.qasm> [--mode compile|optimize] [...same flags] [--json]
oqci passes
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

### passes

Lists the default pipeline in execution order — the ids `--passes` and
`--disable` accept.

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

Notes on the schema:

- `stage` is `"qc-ir"`, `"qco-ir"`, `"qir"`, or `"optimized"` (the
  post-pass circuit in an `optimize` run, where `"qc-ir"` is the input).
- Absent sections are **omitted**, not null.
- A gate parameter is tagged: `{"kind":"concrete","radians":0.5}` or
  `{"kind":"symbol","name":"theta"}`, so a consumer can distinguish a bound
  angle from a free one without parsing display text.
- `unavailable` on a stage explains why it produced nothing.

The IR types themselves stay serde-free: these are view types built in
`src/cli/snapshot.rs`. The IR's shape is a compiler contract governed by
[`ir_spec.md`](ir_spec.md) and should not acquire a wire format by accident.

## Exit codes

`0` on success; `1` with a diagnostic on stderr otherwise. Unbound parameters
are not a failure.
