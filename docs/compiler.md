# The Compiler Orchestrator — One Path from Source to Artifacts

Status: normative
Implemented by: `src/compile.rs`
Verified by: the unit tests in `src/compile.rs`, the pipeline tests in
`src/cli/pipeline.rs`, the backend cases in `tests/cli.rs`, and
`python/tests/test_aer.py`
Entry points: `compile`, `compile_named`, `CompilerConfig`, `Frontend`, `Stop`,
`CompilationArtifacts`, `CompileError`, `available_backends`

Every other document in this set describes one layer: the IR, a frontend, the
pass manager, the target model, lowering, the backend contract. This one
describes the layer that puts them in order and hands the result to whoever
asked — the CLI, the Python SDK, or a Rust caller. There is exactly one such
path, and this is it.

## What §6 requires

`final-deliverables-spec.md` §6 asks for an orchestration layer that can
perform

```text
Frontend → QC-IR → QCO-IR → pass pipeline → target lowering → backend preparation
```

and then lists nine things it must do. Each maps onto a specific part of
`compile_named`:

| §6 requirement | Where it happens |
|---|---|
| accept an explicit configuration | `CompilerConfig`, passed by reference; nothing is read from the environment, a global, or a config file. |
| select frontend | `match config.frontend` — one arm today, and the `match` is what makes adding a second a local change. |
| validate | The frontend validates its own source; `CircuitBuilder::build` enforces the IR invariants; `target::check` validates against the device. Three checks at three layers, none of which the orchestrator re-implements. |
| construct IR | `frontend::parse_openqasm3_named`, which returns a built `Circuit`. |
| run passes in explicit order | `PassManager::default_pipeline().run(...)` — registered order, filtered by `PassSelection`. |
| record pass execution metadata | `result.records` → `CompilationArtifacts::pass_records`, one `PassRecord` per registered pass including the ones that were skipped. |
| select backend | `backend::by_id`, a registry lookup. |
| apply target-aware lowering | `backend.lower(&optimized, &config.lowering)`. |
| return structured artifacts/results | `CompilationArtifacts`. |

§6 closes with "the orchestrator must not hard-code IBM-specific logic", which
is the subject of [backend selection](#backend-selection-without-vendor-branching).

### The order the code actually runs in

Worth stating plainly, because it differs from the order §6 lists the
requirements in:

```text
frontend  →  parameter binding  →  analyze source  →  backend selection
          →  pass pipeline      →  analyze optimized
          →  [stop?]  →  target lowering  →  cost evaluation  →  provenance
          →  [stop?]  →  execution preparation
```

Two deviations, both deliberate:

- **Binding happens before optimization**, not after. See
  [below](#why-parameter-binding-happens-before-optimization).
- **The backend is selected before the passes run**, though §6 lists selection
  after them. Nothing is lowered at that point; the backend is resolved early so
  its profile and cost model can be put into the `PassContext` the pipeline
  receives. Stage E exit criterion 3 asks that optimization be *able* to consult
  backend-defined costs, and this is the channel. None of the four shipped
  generic passes reads it — Stage E §8 is explicit that target awareness must
  not make every pass backend-specific — so the channel being open and unused is
  the intended state. Resolving the backend early also means an unknown backend
  id fails before any work is done rather than after.

## `CompilerConfig`

```rust
pub struct CompilerConfig {
    pub frontend: Frontend,
    pub bindings: HashMap<String, f64>,
    pub passes: PassSelection,
    pub backend: Option<String>,
    pub lowering: LoweringConfig,
    pub settings: ExecutionSettings,
    pub stop: Stop,
}
```

| Field | Default | Meaning |
|---|---|---|
| `frontend` | `Frontend::OpenQasm3` | Which frontend reads the source. The enum is `#[non_exhaustive]` and has one variant. |
| `bindings` | empty | Values for symbolic parameters, applied **before** optimization. OQCI never invents one: an unbound parameter is refused, not guessed. |
| `passes` | `PassSelection::All` | Which optimization passes to run. `Only(..)` and `AllExcept(..)` are the two shapes an ablation study takes. |
| `backend` | `None` | Which backend to compile for. `None` means target-independent compilation, which stops after optimization **whatever `stop` says** — there is nothing to lower to. |
| `lowering` | `LayoutChoice::Trivial`, routing on, decomposition on | How to lower. Documented in [`lowering.md`](lowering.md). |
| `settings` | 1024 shots, no seed, no per-shot memory | Carried into the provenance record and into the executable. The seed default is `None` because choosing one would be picking an experimental parameter §33.15 assigns to Stage G. |
| `stop` | `Stop::Prepared` | Where to stop. |

### `Stop`

```rust
pub enum Stop { Optimized, Lowered, Prepared }
```

| Variant | Runs | `lowered` | `executable` | `cost` / `provenance` | Typical caller |
|---|---|---|---|---|---|
| `Optimized` | frontend, binding, passes | `None` | `None` | `None` | `oqci optimize`, `oqci compile`, `oqci analyze` |
| `Lowered` | …and target lowering | `Some` | `None` | `Some` | `oqci lower` |
| `Prepared` (default) | …and execution preparation | `Some` | `Some` | `Some` | `oqci prepare`, `oqci.compile(backend=…)` |

`stopping_early_stops_exactly_where_asked` runs the same source at all three and
asserts the exact shape of each result.

One consequence worth knowing: with a backend selected and `Stop::Optimized`,
`backend_id` is populated while `lowered`, `cost` and `provenance` are not. The
backend was resolved (and its profile reached the passes); it was simply never
asked to lower anything.

### Target-independent compilation is a complete result

With no backend, `compile` returns after optimization with `lowered`,
`executable`, `cost` and `provenance` all `None` — and that is a finished run,
not a degraded one.

The distinction matters because the alternative designs are both worse. If a
missing backend were an *error*, `oqci optimize` could not exist without
inventing a target the user did not ask for, and every optimization measurement
would silently be a measurement against some particular device. If a default
backend were substituted, the artifacts would carry a `profile_id` nobody chose,
and the provenance would be a fabrication. So the orchestrator returns early
with a plain comment saying why:

```rust
let Some(backend) = backend else {
    // Target-independent compilation is a complete result, not a failure
    // to reach a target. There is nothing to lower *to*.
    return Ok(artifacts);
};
```

`target_independent_compilation_is_a_complete_result` asserts the shape, and
`python/tests/test_aer.py::test_target_independent_compilation_produces_no_executable`
asserts the SDK says the same thing at its own surface: `backend` is `None`, and
the `executable` and `lowering` keys are *absent* rather than empty.

## Why parameter binding happens before optimization

Because rotation merging only fires on concrete angles.

`Param` has two variants, `Concrete(Angle)` and `Symbol(String)`. There is no
variant meaning "the sum of two symbols", and inventing a small symbolic algebra
is out of scope (§32; see also [`ir_spec.md`](ir_spec.md) on the parameter
model). So a pass that would combine `rz(a); rz(b)` into `rz(a+b)` cannot act on
two symbols — it has nothing to write in the result.

If binding ran *after* the pipeline, every rewrite that depends on a number
would be silently lost: the circuit would compile, produce correct results, and
be larger than it needed to be, with nothing in the output indicating why. That
is the worst kind of bug, because it looks like the optimizer simply not finding
much.

`parameters_are_bound_before_optimization_so_the_passes_can_fire` pins the
behaviour with a two-rotation program, compiled twice:

| Run | Source | Result |
|---|---|---|
| unbound | `rz(theta) q[0]; rz(theta) q[0];` | 2 instructions — two symbolic rotations cannot be merged |
| bound (`theta = 0.5`) | the same source, with `bindings` | 1 instruction — bound first, the two merge |

(The test's own comment notes that the frontend's subset does not accept
`rz(-theta)`, a compound expression over a parameter, so the pair is
`theta; theta`, which merges rather than cancels. The point is the same.)

The ordering has a second effect that is easy to miss: `source_metrics` are
computed from the **bound** circuit, so the before/after comparison a user sees
compares like with like. Binding is not an optimization and should not show up
as one.

## Backend selection without vendor branching

Stage C §6 forbids `if IBM … else if Cirq … else if CUDA-Q …` branching through
the compiler, and §6 of the deliverables spec repeats it for the orchestrator
specifically. The implementation makes the ban structural rather than a rule
someone has to remember:

```rust
let backend = match &config.backend {
    None => None,
    Some(id) => Some(crate::backend::by_id(id).ok_or_else(|| {
        CompileError::UnknownBackend { requested: id.clone(), available: /* … */ }
    })?),
};
```

`by_id` returns `Option<Box<dyn Backend>>`. Everything afterwards is a trait
method call — `backend.lower(..)`, `backend.cost_model()`, `backend.profile()`,
`backend.prepare(..)` — so the orchestrator never learns which device it has.
Adding a backend means adding a constructor to `backend::all()`; it does not
touch `src/compile.rs` at all. The word "IBM" appears in that file only in the
doc comment explaining why it appears nowhere else.

An unknown id is refused with the alternatives listed, because a typo is the
overwhelmingly common cause and a bare "unknown backend" makes the user go
looking for the list themselves:

```text
unknown backend `quantum-supercomputer`; available: simulator, simulator-nisq, ibm-illustrative
```

`an_unknown_backend_lists_what_is_available` and
`tests/cli.rs::an_unknown_backend_is_rejected_with_the_alternatives` cover both
surfaces. `every_available_backend_can_compile_a_bell_circuit` walks
`available_backends()` and requires each to produce a legal circuit, so a
backend cannot be registered and quietly be unusable.

## `CompilationArtifacts`

```rust
pub struct CompilationArtifacts {
    pub source_circuit: Circuit,
    pub optimized: Circuit,
    pub pass_records: Vec<PassRecord>,
    pub source_metrics: ResourceReport,
    pub optimized_metrics: ResourceReport,
    pub lowered: Option<Lowered>,
    pub executable: Option<Executable>,
    pub cost: Option<Cost>,
    pub backend_id: Option<String>,
    pub provenance: Option<Provenance>,
}
```

| Field | Present when | Why it is kept |
|---|---|---|
| `source_circuit` | always | The circuit as the frontend read it, **after** binding. Without it there is no "before" to compare against, and the provenance record's `circuit` name comes from here. |
| `optimized` | always | The pass pipeline's output. |
| `pass_records` | always | One record per *registered* pass, including disabled ones, each with `enabled`, `changed` and notes. A pass that was skipped is visible as skipped rather than absent — which is the entire point in an ablation. |
| `source_metrics` | always | §13 requires before/after analysis. |
| `optimized_metrics` | always | The "after" half. |
| `lowered` | a backend was selected and `stop >= Lowered` | Layouts, SWAP count, the rules that fired, the schedule, and the legality report. |
| `executable` | `stop == Prepared` | The artifact an execution adapter replays. See [`backend_contract.md`](backend_contract.md). |
| `cost` | as `lowered` | What the **target** thinks the lowered circuit costs, from the model the profile names — not a figure the orchestrator invented (Stage E §1). Components are kept alongside the scalar (Stage E §4, §7). |
| `backend_id` | a backend was selected | Even when nothing was lowered. |
| `provenance` | as `lowered` | Everything needed to cite a result. |

### Why every stage's output is kept, not just the last

§13 requires before/after analysis, and a result that reported only the final
circuit could not answer "what did optimization actually do" — the question the
tool exists to answer. Keeping only the end state would also make the pass table
unverifiable: a reader could see that `gate-cancellation` reported `changed:
true` but could not check it against anything.

There is a second reason, specific to lowering. Lowering is *not* semantics-
preserving in the way a pass is: its output is a permutation of the input over a
possibly wider register (see [`lowering.md`](lowering.md)). So `optimized` and
`lowered.circuit` are not two views of the same thing, and collapsing them would
lose the only circuit a user can meaningfully diff against their source.

`final_circuit()` returns the furthest stage reached — `lowered.circuit` when
there is one, `optimized` otherwise — so a caller that just wants "the answer"
does not have to reimplement that rule.
`the_final_circuit_is_the_furthest_stage_reached` checks both branches.

### Every `CompileError`

`#[non_exhaustive]`, and four of the five variants are `#[error(transparent)]`,
so the message a user sees is written by the layer that actually refused.

| Variant | Wraps | Raised when |
|---|---|---|
| `Frontend` | `FrontendError` | The source could not be read. |
| `Ir` | `IrError` | Parameter binding failed, or an analysis that builds a QCO-IR graph did. |
| `Pass` | `PassError` | A pass failed. |
| `Backend` | `BackendError` | Lowering, validation or preparation refused. |
| `UnknownBackend` | — | The configuration named a backend that does not exist. The only variant with a message of its own, because it is the only failure the orchestrator itself detects. |

The pipeline stops at the first failure and returns nothing. There is
deliberately no partially-compiled artifact: a caller cannot tell from one how
far it is safe to trust it, and an artifact that is trustworthy only up to an
unmarked point is worse than no artifact.

`compilation_is_deterministic` compiles the same source twice with the same
config and requires the optimized circuit, the lowered circuit and the
executable to be equal each time — the precondition for every claim the
provenance record makes.

## Three consumers, one entry point

§19 states the rule for the CLI: it "must not duplicate compiler logic that
belongs in the Rust library — it should invoke the same public compiler APIs."
The reason is not tidiness. Two implementations of the pipeline would eventually
disagree, and the one a user could see would be the wrong one. So all three
consumers call `compile_named` and do nothing but present what comes back.

| Consumer | Path | Adds |
|---|---|---|
| CLI | `src/cli/pipeline.rs` → `compile::compile_named` | Rendering (text and JSON), file I/O, `watch`. |
| Python SDK | `oqci.compile` → `oqci_native.compile_qasm3` → `compile::compile_named` | A dict view, and the Aer execution adapter. |
| Rust library | `oqci::compile::compile` / `compile_named` | Nothing. |

### The CLI

`src/cli/pipeline.rs` is the only module under `src/cli/` that touches the
compiler, and it reaches it through two shapes:

- **`orchestrate`** — used by `run_compile` and `run_optimize`. It builds a
  `CompilerConfig` with no backend and `Stop::Optimized`, because neither
  command has a target. `run_compile` additionally passes an *empty*
  `PassSelection::Only([])`, which is how `oqci compile` shows the pipeline
  stages without optimizing anything.
- **`run_lower`** — used by `oqci lower` and `oqci prepare`. It takes a
  `LowerRequest` and builds a config with the backend, the lowering options, the
  execution settings, and `Stop::Prepared` or `Stop::Lowered` depending on
  whether the caller asked for an artifact.

`LowerRequest` is a struct rather than nine positional arguments for a concrete
reason stated at its definition: `want_diff` and `prepare` would otherwise sit
next to each other as two bare `bool`s — a call site that is easy to get
backwards and impossible to read.

Everything after the call is presentation. `cli::snapshot` turns a `Lowered` into
a `LoweringView` and an `Executable` into an `ExecutableView`; no gate is
interpreted, no metric recomputed, no circuit rewritten. The numbers a user sees
are the numbers the compiler computed, and
`tests/cli.rs::lower_output_is_stable_across_runs` checks they do not move
between runs. See [`cli.md`](cli.md) for the command surface.

### The Python SDK

`python/oqci/__init__.py` is the §17 layer: circuit import, compiler invocation,
configuration, backend selection, analysis and result access, over the stable
contracts the `oqci._native` extension exposes. Its rule is the CLI's rule, for
the CLI's reason — **no compilation decision is made in Python**.

`oqci.compile(source, backend=…, bindings=…, name=…, shots=…, seed=…,
layout=…)` builds a `CompilerConfig` on the Rust side and calls `compile_named`.
The result is a plain dict:

| Key | Present when | From |
|---|---|---|
| `backend` | always (may be `None`) | `artifacts.backend_id` |
| `source_metrics`, `optimized_metrics` | always | the two `ResourceReport`s |
| `passes` | always | `pass_records`, as `{id, enabled, changed, notes}` |
| `lowering` | a backend was selected | profile, layouts, SWAP count, orientations repaired, rules applied, legality |
| `cost` | as `lowering` | the target's `Cost`, serialized whole |
| `executable` | preparation ran | the `Executable`, serialized whole |

Execution is deliberately not part of this. `oqci.backends.aer.run` takes the
`executable` dict and runs it; the Rust `Backend::execute` refuses by design.
[`backend_contract.md`](backend_contract.md#the-execution-boundary) explains
why, and `python/tests/test_aer.py` is the test file that closes the loop by
checking OQCI's output against Aer actually running it.

`oqci.available_backends()` mirrors `compile::available_backends()`, and its
docstring says plainly that none of them executes in the compiler process.
`oqci.qasm3_to_qir` and `oqci.qiskit_to_qir` remain available and both document
QIR as a lowering artifact rather than an execution guarantee (Stage C §5).

## What is absent

Per `final-deliverables-spec.md`'s Critical Rule — a feature is not implemented
merely because a module, type, parameter or document names it — the following
are **absent**:

- **Any frontend but OpenQASM 3.** `Frontend` has one variant. A Qiskit adapter
  exists (`frontend::qiskit::translate`, §5.3) and is reachable from Python
  through `qiskit_to_qir`, but it does **not** go through the orchestrator:
  `compile` and `compile_named` take `&str` source text, so a
  `QuantumCircuit` cannot be handed to the pipeline at all. A Qiskit circuit can
  therefore reach QIR but cannot reach a backend, a `Lowered`, an `Executable`
  or a provenance record. §5.4 (Cirq) and §5.5 (CUDA-Q) do not exist in any
  form.
- **Optimization configuration from Python.** §17 lists it among what the SDK
  should expose. `compile_qasm3` has no `passes` or `disable` parameter, so
  `CompilerConfig::passes` is always `PassSelection::All` on that path. The
  ablation shape the CLI supports (`--passes`, `--disable`) has no Python
  equivalent; `PassSelection` is reachable only from Rust.
- **Circuit *construction* from Python.** §17 says "circuit
  construction/import". Import exists (OpenQASM 3 text, and Qiskit for the QIR
  path); there is no builder API — nothing in the SDK creates a circuit
  programmatically.
- **The circuits themselves in the Python dict.** `artifacts_to_dict` exposes
  metrics, pass records, lowering summary, cost and the executable. Neither
  `source_circuit` nor `optimized` is serialized, so a Python caller can see
  *that* a pass changed something and by how much, but cannot see the
  instruction list it produced. The CLI's `--emit` does expose it.
- **A shared schema between the SDK dict and the CLI's `--json`.** These are two
  different shapes over the same artifacts: the CLI emits a `PipelineReport`
  (`source_path`, `frontend`, `circuit_name`, `unbound_parameters`, `stages`,
  `passes`, `diff`, `target`, `lowering`, `executable`), while the SDK emits the
  table above. `executable` and `cost` are serialized from the same Rust types
  in both and so cannot drift; `lowering` is assembled separately on each side
  and already differs (the CLI's carries `backend`, `steps` and `violations`;
  the SDK's does not).
- **Pass ordering as configuration.** `PassManager::default_pipeline()` is fixed.
  `PassSelection` can disable a pass or restrict to a subset, but nothing
  reorders the pipeline, inserts a pass, or registers one from outside the crate
  (§18.2).
- **An execution step in the orchestrator.** `compile` stops at
  `Stop::Prepared`. There is no `Stop::Executed` and no `ExecutionResult` on
  `CompilationArtifacts`, because no backend executes — see
  [`backend_contract.md`](backend_contract.md#the-execution-boundary). Adding
  one would mean the orchestrator returning an artifact it cannot fill.
- **Compilation timing.** `ExecutionResult::compilation_duration_ms` exists as a
  field, and the orchestrator does not measure it. Nothing in `compile_named`
  reads a clock. The field stays `None` rather than acquiring a number nobody
  measured.
- **Incremental or cached compilation.** Every call runs the whole pipeline from
  source text. There is no artifact cache, and `Lowered` is not serializable, so
  a lowered circuit cannot be persisted and re-offered to a backend later —
  which is the case `Backend::validate`'s "a `Lowered` that arrived from
  somewhere else" doc comment anticipates but nothing yet produces.
