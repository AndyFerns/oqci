# The Backend Contract — Lowering, Validation, Preparation and the Execution Boundary

Status: normative
Implemented by: `src/backend/`, `build.rs`, `python/oqci/backends/`
Verified by: the unit tests in `src/backend/mod.rs`, `src/backend/executable.rs`,
`src/backend/result.rs`, `src/backend/simulator.rs`, `src/backend/ibm.rs`,
the backend cases in `tests/cli.rs`, and `python/tests/test_aer.py`
Entry points: `Backend`, `BackendError`, `validated`, `all`, `by_id`,
`Executable`, `ExecutableOp`, `ExecutionSettings`, `ExecutionResult`,
`Provenance`, `SimulatorBackend`, `IbmBackend`

[`target_model.md`](target_model.md) describes what OQCI knows about a device.
[`lowering.md`](lowering.md) describes what OQCI does with that knowledge to
make a circuit legal. This document describes the last layer: what a device
must expose for OQCI to compile *for* it, what artifact that compilation
produces, and — the part that matters most — exactly where OQCI stops.

**Read this first.** No backend shipped in this build executes a circuit.
Nothing in this document should be read as a claim that OQCI can run a program
on IBM hardware, because it cannot. What OQCI produces is an artifact that has
been lowered to, and validated against, a supplied target description. Whether
that description corresponds to a real device is a separate question, and for
the only IBM-shaped target here the answer is explicitly no. The
[execution boundary](#the-execution-boundary) section says all of this again,
with reasons.

## Why the contract exists

Stage C is the locked decision behind `src/backend/`:

> **C — Define an explicit backend contract and add a dedicated IBM
> target-lowering stage. Never assume that arbitrary emitted QIR is directly
> executable on IBM hardware.**

Its §1 asks the compiler to distinguish five things that are easy to run
together and impossible to separate afterwards: a target-independent optimized
program, a backend target description, target-specific lowering, execution
submission, and execution results. §6 then names the shape that must not
appear anywhere in the generic compiler:

```text
if IBM
    ...
else if Cirq
    ...
else if CUDA-Q
    ...
```

The ban is structural here rather than a convention. Backend-specific
behaviour lives behind the [`Backend`](#what-a-backend-must-expose) trait,
backend *selection* is a registry lookup returning a `Box<dyn Backend>`
(see [`compiler.md`](compiler.md)), and the compiler core names no vendor. The
word "IBM" appears in `src/compile.rs` only in the doc comment explaining why
it does not appear anywhere else, and nowhere at all in `src/pass/`,
`src/lowering/` or `src/ir/` outside comments citing the specification.

## The four stages

Stage C §10 requires the abstraction to distinguish compilation/target
preparation, execution, and result retrieval. `Backend` splits the first of
those into three, giving four methods that fail for four different reasons:

| Stage | Method | Answers | Fails when |
|---|---|---|---|
| Target lowering | `Backend::lower` | "Can this device run this program at all, after rewriting?" | The circuit cannot be made legal — too wide, unroutable, or no rule reaches the basis. |
| Validation | `Backend::validate` | "Is this circuit legal for this device *as written*?" | Never; it returns a `LegalityReport`, not a `Result`. `validated` turns a blocking report into an error. |
| Preparation | `Backend::prepare` | "What artifact would this device be given?" | The circuit still contains something no device can run, or it fails validation. |
| Execution | `Backend::execute` | "What came back?" | Always, for every backend in this build. See [the boundary](#the-execution-boundary). |

`lower` and `validate` have default implementations that delegate to
`crate::lowering::lower` and `crate::target::check`, so a backend gets both for
free by supplying a `BasisProfile`. Neither shipped backend overrides them:
lowering is generic machinery driven by target *data*, which is the whole point
of the profile layer.

### Why preparation is separate from execution

Because it is what makes the IBM path honest.

Everything up to and including a validated executable artifact is pure Rust,
runs with no network and no credentials, and is covered by ordinary unit tests.
Only the final submission needs an account, an SDK and a queue. Fusing the two
would mean that the parts of the IBM path that *can* be verified could only be
exercised by the parts that cannot — so nothing would be verified, and a
refusal at the very end would be indistinguishable from a refusal at the
beginning.

Split, the boundary falls in exactly one place, and the error that names it
(`ExecutionNotAvailableInProcess`) is about submission only. Everything before
it succeeded, and `tests/cli.rs::prepare_writes_a_replayable_executable` shows
the artifact it produced.

## What a backend must expose

Stage C §3 enumerates what the contract must be able to provide. Every item is
reachable:

| Stage C §3 capability | Where |
|---|---|
| target identity | `Backend::id`, `Backend::description` |
| supported qubit count / physical resources | `Backend::profile` → `BasisProfile::qubit_count` |
| connectivity / topology | `Backend::profile` → `BasisProfile::topology` |
| supported operation / basis information | `BasisProfile::supported_operations` |
| operation constraints | `BasisProfile::parameter_constraint` |
| measurement constraints | `BasisProfile::measurement` → `MeasurementSupport` |
| reset constraints | `MeasurementSupport::reset` |
| parameter constraints | `ParameterConstraint { min, max }` |
| target-specific decomposition information | `BasisProfile::decomposition_rules` — **identifiers only**; see [`target_model.md`](target_model.md) |
| cost-model access | `Backend::cost_model` → `&dyn CostModel` |
| circuit validation | `Backend::validate`, and `validated` for the blocking form |
| target lowering entry point | `Backend::lower` |
| executable representation type | `Executable` |
| execution interface | `Backend::execute` — present, and it refuses |
| structured execution result | `ExecutionResult` — defined, and never produced in this build |

The last two rows are the honest ones. The interface exists and is typed; what
it returns today is a boundary, not a result.

Stage E's requirement sits in the same trait: `Backend::cost_model` is supplied
by the *target*, not chosen by the optimizer, which is Stage E §1's principle
("the compiler optimizer chooses among transformations; the target describes
what is expensive") expressed as a method rather than a promise.
`src/backend/mod.rs::every_backend_names_a_resolvable_cost_model` asserts that a
backend and its profile never disagree about which model that is — a
disagreement would make every provenance record the backend writes false.

## Every `BackendError`

`BackendError` is `#[non_exhaustive]`, so matching on it requires a wildcard and
a new variant cannot silently change a caller's behaviour.

| Variant | Fields | Raised by | Means |
|---|---|---|---|
| `Lowering` | `source: LoweringError` | `Backend::lower` | The circuit cannot be made legal for this device. Transparent, so the message a user sees is lowering's own — see [`lowering.md`](lowering.md) for the full list. |
| `NotExecutable` | `index`, `detail` | `Executable::from_lowered` | The circuit survived lowering but still holds something no execution API accepts: a symbolic parameter, or an `Opaque` gate with no known implementation. |
| `ExecutionNotAvailableInProcess` | `backend`, `reason` | `Backend::execute` | Not a failure and not a stub. The artifact is ready; it has to cross into another runtime to run. |
| `InvalidTarget` | `backend`, `detail` | `validated`, `SimulatorBackend::try_from_profile`, `IbmBackend::from_profile`, `IbmBackend::from_target_json` | Either the circuit is illegal for this target, or the target description itself is unusable. |

`ExecutionNotAvailableInProcess` being a typed variant rather than a `todo!()`,
an empty `ExecutionResult` or a silent `Ok` is deliberate: a caller can match on
it and route around it, and nothing can mistake it for a result. An empty
counts map would have been the worst of the three — it looks exactly like a
circuit that ran and produced nothing.

### `validated`, and the one violation it lets through

`validated(backend, lowered)` is the shared "check before you prepare" step,
written once so it is not a convention each backend re-implements and one of
them eventually forgets. Both shipped `prepare` implementations call it first.

It filters one violation out of the blocking set: `Violation::UnboundParameter`.
That is not a loophole, and the circuit still never reaches an artifact. An
unbound parameter *is* a genuine violation — no execution API takes a symbol —
but it is a program problem with a known remedy, not a target-compatibility one,
and refusing it here would produce

```text
circuit is not legal for `ideal-simulator@1`: [UnboundParameter { .. }]
```

which tells a user nothing they can act on. `Executable::from_lowered` refuses
the same circuit a moment later with the message that helps: *bind parameters
before preparing for execution*. `src/backend/executable.rs::
a_symbolic_parameter_is_refused_with_advice` asserts the wording, and
`python/tests/test_aer.py::test_an_unbound_parameter_is_refused_rather_than_guessed`
asserts it survives all the way out to the Python SDK.

`describe` renders a violation list as a sentence rather than a `Debug` dump,
and its `match` deliberately has no wildcard arm: a new `Violation` breaks the
build and makes someone write a sentence for it instead of letting a
`{:?}` reach a user.

## The executable representation

### Why this is not QIR

Stage C §5 is explicit that the textual QIR emitter "is useful as a
lowering/output artifact, but … does not itself guarantee IBM execution
compatibility", and it forbids documenting "QIR emitted = executable on IBM",
writing backend tests that assume it, or letting a hardware layer consume
arbitrary high-level QIR without validating the target contract. §33.12 adds
the general rule: do not claim hardware executability without target-specific
lowering and validation.

So QIR and the executable representation are different artifacts produced by
different stages, and they do not touch. `src/backend/` contains no reference to
QIR at all — not an import, not a function, not a string. `Executable` can only
be built from a `Lowered` circuit that a target has already validated;
there is no path from QIR text into a backend. QIR remains available through
`ir::emit_qir`, the CLI's `--emit qir`, and `oqci.qasm3_to_qir`, all of which
document it as a lowering artifact and nothing more.

### Why this is not OpenQASM 3 either

Handing the Python adapter OpenQASM text would have been the obvious choice.
Two things rule it out.

The first is mechanical: `qiskit.qasm3.loads` raises
`MissingOptionalLibraryError` unless the separate `qiskit_qasm3_import` package
is installed, and it is not a dependency of this project
(`python/requirements-dev.txt` pins `qiskit`, `qiskit-aer`, `maturin` and
`pytest`, and nothing else).

The second is the real reason: it would put a third-party parser on the trust
path between *the circuit OQCI verified* and *the circuit that runs*. A parser
bug could then silently change the program after verification, and the counts
would look perfectly reasonable. Everything this layer does is aimed at making
that class of failure impossible, so adding a component that reintroduces it in
exchange for convenience is not a trade worth making.

### What it is

A structured instruction list, serialized with serde, replayed by the adapter
one method call per operation.

```rust
pub struct Executable {
    pub backend_id: String,
    pub num_qubits: u32,
    pub num_clbits: u32,
    pub ops: Vec<ExecutableOp>,
    pub settings: ExecutionSettings,
    pub provenance: Provenance,
}

pub struct ExecutableOp {
    pub op: String,          // mnemonic, and the adapter's method name
    pub qubits: Vec<u32>,    // physical operands
    pub params: Vec<f64>,    // concrete radians, always
    pub clbit: Option<u32>,  // measurement destination
}
```

| Field | Why it is there |
|---|---|
| `backend_id` | An executable is not portable between targets, and the artifact says so rather than leaving it to convention. Asserted by `an_executable_names_the_backend_it_was_prepared_for`. |
| `num_qubits`, `num_clbits` | Taken from the *lowered* circuit, so they are device widths, not program widths — routing may have widened the register. |
| `ops` | The program, in order. |
| `settings` | Shots, seed and `memory`, so a run can reproduce what the provenance record says it did without the caller having to remember. |
| `provenance` | Travels *with* the artifact rather than in a log beside it, which is the only arrangement that cannot drift. |

The IR types deliberately stay serde-free — their shape is a compiler contract
governed by [`ir_spec.md`](ir_spec.md), and it should not acquire a wire format
by accident. `ExecutableOp` is that wire format, defined where it belongs.

Two helpers exist so an adapter can check before it runs rather than failing
part way through a circuit: `operations()` returns the distinct mnemonics in
sorted order, and `has_measurement()` reports whether the circuit produces
counts at all. An unmeasured circuit would otherwise return an empty result
that looks exactly like a failure —
`python/tests/test_aer.py::test_an_unmeasured_circuit_is_refused_before_it_runs`
checks the adapter refuses it instead.

### The two refusals

`Executable::from_lowered` refuses rather than translating on a best effort:

- **A symbolic parameter.** No execution API accepts a symbol, and
  substituting a value here would silently run a different circuit from the one
  the user wrote.
- **An `Opaque` gate.** It has no known matrix, so guessing would produce an
  artifact that runs and computes the wrong thing.

Both are `BackendError::NotExecutable` carrying the offending instruction index.
`an_opaque_operation_is_refused_rather_than_passed_through` lowers an opaque gate
against a profile that tolerates it, so that the refusal under test is the
executable's rather than lowering's.

Every `GateKind` except `Opaque` maps one-to-one onto a `QuantumCircuit` method
on the pinned Qiskit — and the mapping in `python/oqci/backends/aer.py` was
built by introspecting the installed package, not recalled from memory, which is
what §33.3 and §33.4 demand. A mnemonic missing from that table is therefore a
genuine gap, not a gate the adapter chose not to support, and
`UnsupportedOperation` says so rather than skipping the operation. Skipping
would run a different circuit and produce plausible counts.

## Provenance and results

### Why provenance is a type and not a log line

Stage C §9 lists what every execution result must be attributable to, and
Stage E §9 adds cost-model identity and configuration on top — without which a
comparison between two optimization experiments means nothing. `Provenance`
carries both lists, and carries them inside the artifact.

| Field | Type | Requirement it satisfies | Note |
|---|---|---|---|
| `circuit` | `String` | C §9 input circuit identifier | The source circuit's name; the CLI and SDK set it from the filename. |
| `compiler_version` | `String` | C §9, E §9 compiler version | `CARGO_PKG_VERSION`, baked in at build time. |
| `git_commit` | `String` | C §9 Git commit | From `build.rs`; `"unknown"` outside a checkout. |
| `backend_id` | `String` | C §9, E §9 backend identity | |
| `profile_id` | `String` | C §9, E §9 target profile | Qualified as `id@version` (Stage D §8), taken from the `Lowered`, not from the backend — so it names the profile the circuit was actually lowered against. |
| `cost_model_id` | `String` | E §9 cost-model identifier | |
| `cost_model_version` | `String` | E §9 cost-model version | |
| `cost_model_configuration` | `BTreeMap<String, String>` | E §6, E §9 configuration | The weights, so a scalar score is never an unexplained figure. |
| `pass_pipeline` | `Vec<String>` | C §9, E §9 optimization pipeline | The passes that **ran**, not the ones registered — see [`compiler.md`](compiler.md). |
| `lowering_steps` | `Vec<String>` | C §9 compilation metadata | The schedule that executed, in order. |
| `decomposition_rules` | `Vec<String>` | C §9 compilation metadata | Which rules fired. |
| `initial_layout` | `Vec<u32>` | C §9 compilation metadata | Logical index → physical index at entry. |
| `final_layout` | `Vec<u32>` | C §9 compilation metadata | And at exit — without it, a caller cannot tell which physical wire a measurement came from. |
| `swaps_inserted` | `usize` | C §9 compilation metadata | Routing overhead. |
| `shots` | `u32` | C §9 shot count, execution settings | |
| `seed` | `Option<u64>` | C §9 execution settings | **Recorded, never chosen.** §33.15 puts random seeds in Stage G; inventing one here would fabricate an experimental parameter. `ExecutionSettings::default().seed` is `None`, asserted by `the_default_settings_choose_no_seed`. |

The two §9 items *not* on `Provenance` — raw backend result and derived
metrics — belong to `ExecutionResult`, because they describe a run rather than
a compilation.

`provenance_for` assembles the record in one place (`src/backend/simulator.rs`)
rather than in each backend, so a backend cannot quietly omit a field and make
its results incomparable with the others'.
`provenance_records_every_field_stage_c_requires` checks the assembled record
field by field.

### The git commit

`build.rs` runs `git rev-parse HEAD` at build time and exports it as
`OQCI_GIT_COMMIT`. Reading it there rather than shelling out at runtime keeps
the compiler's execution path free of process spawning, and means a released
binary carries the commit it was built from even after the checkout is gone.
The build script re-runs when `.git/HEAD` or `.git/refs/heads` changes, so a
stale commit cannot be baked in.

Failure is not an error. A crate built from a tarball, a vendored copy or a
registry download has no git metadata, and refusing to build there would be
absurd. The commit becomes the literal string `"unknown"` — reported rather than
omitted, because a missing field invites a reader to assume it was never
recorded, while `"unknown"` says the build had none.

### Why the three durations are independent

Stage C §8 is blunt: "Queue/wait behavior must not be mistaken for compiler
execution time", and §10 repeats it. So `ExecutionResult` carries three
separate fields:

| Field | Measures |
|---|---|
| `compilation_duration_ms` | How long compilation took. |
| `submission_duration_ms` | How long the job waited before running — queue time. |
| `execution_duration_ms` | How long the circuit actually ran. |

All three are `Option<f64>`, and **none is ever derived from another**. There is
no code anywhere that computes one by subtracting two others, and there is not
going to be: the three measure unrelated things, and a backend that cannot
measure one leaves it `None` rather than reporting a plausible number. An
invented duration is exactly the fabricated measurement the benchmarking work
downstream must not inherit — a zero would be worse still, since zero claims a
measurement that was never made.
`durations_are_independent_and_absent_when_unmeasured` pins this.

On the only path that actually runs a circuit, only one of the three is ever
populated: `python/oqci/backends/aer.py` times the call to Aer and reports
`execution_duration_ms`. Nothing shipped measures compilation or submission
time, so a consumer should expect those two to be absent rather than zero.

### `ExecutionResult`

`counts` is a `BTreeMap` so the same result serializes identically every time —
results get snapshotted and compared (Stage D §8), and
`counts_serialize_in_a_stable_order` checks it. `backend_metadata` is kept raw
and unparsed on purpose: Stage C §9 requires the raw backend result to be
preserved, and normalizing it would lose whatever a particular backend reported
that OQCI does not yet model.

`observed_shots()` sums the counts rather than echoing `provenance.shots`, so a
backend that returned fewer shots than were asked for is visible instead of
assumed away (`observed_shots_come_from_the_data_not_the_request`).
`most_frequent()` breaks ties on the lexicographically smaller bitstring, making
it a function of the data rather than of map iteration order.

The type is defined, serialized and unit-tested. **Nothing in this build ever
constructs one outside its own tests** — see [what is absent](#what-is-absent).

## The shipped backends

`backend::all()` returns three; `backend::by_id` looks one up.
`every_backend_is_addressable_by_its_own_id` asserts the two agree, and
`every_backend_has_a_lowerable_target` asserts each one's rule set can actually
reach its own basis, so a malformed target fails at `cargo test` rather than on
a user's first circuit.

| id | Profile | Cost model | What it is |
|---|---|---|---|
| `simulator` | `ideal-simulator@1`, 32 qubits, all-to-all, every registered mnemonic | `uniform` | An unconstrained simulator. Lowering is effectively a no-op, which makes it the control in any comparison. |
| `simulator-nisq` | `linear-nisq@1`, 5 qubits, linear chain, `{rz, sx, x, cx, measure}`, no reset, no mid-circuit measurement | `nisq-weighted` | A simulator constrained like a small NISQ device. It exercises routing and decomposition while staying runnable, so the same circuit can be run on it and on `simulator` and the results compared. |
| `ibm-illustrative` | `ibm-illustrative@1`, 5 qubits, ring with two one-way couplings, `{rz, sx, x, cx, measure}` | `nisq-weighted` | **Synthetic. It describes no real device.** |

### `ibm-illustrative` is not a device

Stated plainly, because the name is the only thing standing between it and a
misreading: `ibm-illustrative` is invented. The basis is the *shape* IBM
superconducting hardware tends to have — which is what motivated adding `sx` to
the gate set (see
[`architecture_decision_sx_basis_gate.md`](architecture_decision_sx_basis_gate.md))
— and the topology is a five-qubit ring with two links declared one-way,
chosen so that orientation repair is genuinely exercised rather than left
untested. The qubit count, the connectivity and the directions are
illustrative. There is no calibration data, no error rate and no duration on
it, because inventing those would be the fabricated experimental data §33.15
rules out.

The disclaimer is not only in prose. The profile carries the capability string
`synthetic-not-a-real-device`, so it survives serialization into any result that
cites the profile, and the backend's own description reads "synthetic
IBM-shaped target; describes no real device and cannot execute".
`the_illustrative_target_says_in_its_own_metadata_that_it_is_synthetic` asserts
both.

Real target data is meant to come from outside. §9.2 requires the selected IBM
backend to be determined by configuration or runtime availability, and forbids
hard-coding one physical device name into the compiler core — so the
constructor that matters is `IbmBackend::from_target_json`, which takes a
serialized `BasisProfile` (the shape a retrieved target would be converted
into). `a_target_can_be_built_from_a_serialized_description` exercises that
path and checks a one-way link survives the round trip;
`malformed_target_data_is_refused_rather_than_partially_applied` checks a bad
description is refused whole rather than applied in part. `illustrative()`
exists only so the path can be exercised without a network.

Stage C §8 lists target retrieval *and* target profile construction among the
IBM adapter's responsibilities. Retrieval needs the SDK; construction from
retrieved data does not, and is implemented — which is what puts the boundary in
exactly one place.

## The execution boundary

This is the section that matters.

**No backend in this build executes a circuit.** All three return
`BackendError::ExecutionNotAvailableInProcess` from `Backend::execute`. This is
asserted rather than assumed: `no_shipped_backend_claims_to_execute_in_process`
walks `backend::all()`, lowers and prepares a Bell circuit for each, calls
`execute`, and requires the error. If a backend ever gains in-process execution,
that test is where it gets noticed.

### For `IbmBackend`: three reasons

None of them is that it was forgotten.

1. **`qiskit-ibm-runtime` is not available.** It is not a dependency of this
   project and is not installed in its environment, so the SDK cannot be
   checked. §33.4 forbids implementing a vendor-specific API from memory when
   the current SDK documentation could be consulted instead — and writing a
   submission path from recollection is precisely that.
2. **There are no credentials.** Submission code could not be run even once, so
   it could not be tested even once. Untested code on the path between a
   verified circuit and real hardware is worse than an explicit boundary,
   because it would look like a working feature until the moment it mattered.
3. **§33.12 forbids the claim.** Do not claim hardware executability without
   target-specific lowering and validation. This module provides both; it does
   not thereby acquire the right to claim the rest. Lowering and validating
   against a target description is a checkable statement about that
   description. It is not a statement about a machine.

**No claim of IBM hardware executability is made.** What is claimed is narrower
and verifiable: an `Executable` produced for an IBM backend has been lowered to,
and validated against, the target description it was given. If that description
is wrong, so is the conclusion — which is why `from_target_json` exists, and why
`illustrative()` says in its own name that it is not real. The error text itself
carries the disclaimer, and
`execution_is_an_explicit_boundary_not_a_silent_success` asserts the phrase "no
claim is made" is in it, so the boundary cannot be reworded into something that
implies hardware readiness without a test failing.

### For `SimulatorBackend`: one reason

The project's explicit non-goals (§32) rule out "a custom quantum simulator
replacing established simulator frameworks". Writing one in this crate to
satisfy `Backend::execute` would trade a boundary for a scope violation.
Execution belongs to Qiskit Aer, through `python/oqci/backends/aer.py`, and the
error says so by name — `execution_names_where_it_actually_happens` asserts the
message mentions Aer, so a user who hits the boundary is told where to go next.

The test-only state-vector simulator under `tests/support/` is not a
counterexample. It exists to verify that rewrites preserve semantics, never
leaves `cargo test`, and is a dev-dependency precisely so it cannot become a
product feature by accident.

### Where execution actually happens

`python/oqci/backends/aer.py` is the one place a prepared artifact runs. It:

- rebuilds the executable as a `QuantumCircuit` (`to_qiskit`), one method call
  per operation, against a mnemonic→method table verified by introspection;
- refuses an operation it cannot replay, and refuses an executable with no
  measurement, rather than running something different from what OQCI verified;
- defaults `shots` and `seed` to whatever the executable was prepared with, so
  a run reproduces what the provenance says it did — and, when a caller
  overrides them, updates the returned provenance to match rather than
  continuing to claim the original values;
- times only the Aer call, and reports it as `execution_duration_ms`, keeping
  Stage C §8's separation;
- accepts a caller's `NoiseModel` and **never constructs one**. Noise policy
  belongs to the benchmarking protocol, which is not locked, and a built-in
  model with plausible-looking parameters would be fabricated experimental data
  wearing a library's name.

`python/tests/test_aer.py` is the only place in this project where OQCI is
checked against something else *running its output*; every other test checks
OQCI against OQCI. What it actually verifies, end to end:

| Test | What it establishes |
|---|---|
| `test_a_bell_pair_runs_on_an_unconstrained_simulator` | The whole path works at all, on a target that barely rewrites anything. |
| `test_a_bell_pair_survives_lowering_to_a_restricted_basis` | `h → rz/sx` is correct, not merely legal. A mis-signed Euler decomposition shifts the distribution or puts population on `01`/`10`. |
| `test_a_distant_bell_pair_survives_routing` | The test the lowering layer exists to pass: `q0` and `q2` are two hops apart on a line, so a SWAP is required, and a SWAP on the wrong pair (or a mis-reported final layout) changes *which* bits are correlated. |
| `test_a_ghz_state_survives_lowering` | Three-way correlation survives the same treatment. |
| `test_the_constrained_and_unconstrained_paths_agree` | Two very different devices, one program, the same distribution — evidence about the lowering, since that is the only thing that differs. |
| `test_a_dense_layout_reaches_the_same_distribution` | Layout changes the SWAP count, never the answer. |
| `test_the_ibm_shaped_target_compiles_and_the_result_is_still_correct` | Orientation repair on a directed-coupling target preserves semantics. **Run on Aer**, not on hardware — and the test's own docstring says so. |
| `test_a_seeded_run_is_reproducible` | The same seed gives the same counts. |
| `test_results_carry_the_provenance_of_what_produced_them` | The provenance survives the Rust→Python boundary intact, and Aer's own metadata is kept verbatim. |
| `test_the_executable_only_uses_operations_the_target_supports` | Nothing outside the declared basis reaches the artifact. |
| `test_an_unbound_parameter_is_refused_rather_than_guessed` | The refusal reaches the SDK with actionable wording; bound, the same program runs. |
| `test_noise_models_are_accepted_but_never_invented` | A caller's model is passed through; none is supplied by default. |
| `test_target_independent_compilation_produces_no_executable` | No backend means no `executable` and no `lowering` key at all — not an empty one. |

Distributions are checked to a 4% tolerance at 8000 shots, roughly seven sigma
on a 50/50 split: loose enough never to flake, tight enough that a wrong
lowering cannot slip through. The reasoning is that a wrong lowering does not
shift a distribution slightly — it produces a different one.

## Stage C exit criteria

Stage C §10 lists seven. Assessed honestly:

| # | Criterion | Status | Evidence, or why not |
|---|---|---|---|
| 1 | A backend contract exists and is documented. | **Met** | The `Backend` trait in `src/backend/mod.rs` covers every capability Stage C §3 enumerates (table [above](#what-a-backend-must-expose)); this document is the documentation. |
| 2 | Simulator backends use the contract. | **Partially met** | `SimulatorBackend` implements every method of the contract, and the artifact Aer runs is exactly the one `prepare` produced (`python/tests/test_aer.py`). But execution does not go *through* `Backend::execute`: that method refuses, and a caller must cross into Python deliberately. The compilation half of the contract is fully used; the execution half is a boundary the caller steps over. |
| 3 | Target-independent compilation does not contain IBM-specific branching. | **Met** | No vendor name appears as control flow anywhere in `src/ir/`, `src/pass/`, `src/lowering/`, `src/analysis/` or `src/compile.rs` — only in comments citing this specification. Backend selection is `backend::by_id`, a registry lookup returning `Box<dyn Backend>`; adding a device touches no compiler file. |
| 4 | IBM target lowering is a distinct, testable component. | **Met, with a caveat worth stating** | `src/backend/ibm.rs` is a separate module with its own tests, including `a_one_way_link_forces_orientation_repair` and `a_target_whose_rules_cannot_reach_its_basis_is_refused_at_construction`. The caveat: what is IBM-specific is the *profile data*, and the only IBM profile shipped is synthetic. The component is distinct and tested; the target it is tested against is invented. |
| 5 | IBM target validity is checked before submission. | **Met in substance; partly vacuous** | `IbmBackend::prepare` calls `validated` before `Executable::from_lowered`, so no artifact exists for a circuit the target rejects — `preparing_an_illegal_circuit_is_refused` and `preparing_refuses_a_circuit_lowered_for_a_different_device` both exercise a circuit lowered for one device and offered to another. The vacuity: there is no submission step for the check to precede. What the criterion can mean here, it means. |
| 6 | QIR generation and hardware execution are explicitly separated. | **Met** | `src/backend/` contains no reference to QIR of any kind. `Executable` is constructible only from a validated `Lowered`; there is no path from QIR text into a backend. The SDK's `qasm3_to_qir` docstring states in as many words that QIR is not an execution guarantee. |
| 7 | Backend results preserve sufficient provenance for later benchmarking. | **Partially met** | `Provenance` carries all of Stage C §9's compilation-side list plus Stage E §9's cost-model identity and configuration, and it travels inside the artifact and out through Aer's result (`test_results_carry_the_provenance_of_what_produced_them`). What is missing: §9's "raw backend result" and "derived metrics" live on `ExecutionResult`, which **nothing in this build ever constructs** outside its own tests; the executing path returns a Python `AerResult` instead, and nothing persists either into a benchmark record. The provenance a result needs is computed and carried. The result type it was designed to sit inside is not yet produced. |

Summary: four met, one met with a caveat, two partially met. None of the seven
is unmet, and no claim above depends on execution that does not happen.

## How this is verified

| Claim | Test |
|---|---|
| Every backend is reachable by its own id. | `src/backend/mod.rs::every_backend_is_addressable_by_its_own_id` |
| A backend and its profile agree about the cost model. | `every_backend_names_a_resolvable_cost_model` |
| Every backend's rules can reach its own basis. | `every_backend_has_a_lowerable_target` |
| No backend executes in-process. | `no_shipped_backend_claims_to_execute_in_process` |
| A circuit lowered elsewhere is refused. | `preparing_an_illegal_circuit_is_refused`, `preparing_refuses_a_circuit_lowered_for_a_different_device` |
| Symbolic parameters and opaque gates are refused. | `a_symbolic_parameter_is_refused_with_advice`, `an_opaque_operation_is_refused_rather_than_passed_through` |
| The artifact round-trips through JSON unchanged. | `an_executable_round_trips_through_json`, `a_result_round_trips_through_json` |
| Provenance carries every required field. | `provenance_records_every_field_stage_c_requires` |
| Durations are independent and absent when unmeasured. | `durations_are_independent_and_absent_when_unmeasured` |
| The IBM boundary does not imply hardware readiness. | `execution_is_an_explicit_boundary_not_a_silent_success` |
| The illustrative target declares itself synthetic. | `the_illustrative_target_says_in_its_own_metadata_that_it_is_synthetic` |
| The CLI listing does not imply circuits run here. | `tests/cli.rs::backends_lists_what_can_be_compiled_for` |
| A prepared artifact is replayable and stays inside the basis. | `tests/cli.rs::prepare_writes_a_replayable_executable` |
| The written artifact carries provenance. | `tests/cli.rs::prepare_records_the_provenance_of_what_it_built` |
| The compiled result is physically correct. | the whole of `python/tests/test_aer.py` |

## What is absent

Per `final-deliverables-spec.md`'s Critical Rule — a feature is not implemented
merely because a module, interface, type or document names it — the following
are **absent**, not partly there:

- **Live IBM submission.** There is no code that authenticates, discovers a
  backend, retrieves a real target, submits a job or polls one. Stage C §8's
  adapter responsibilities are implemented only as far as *target profile
  construction* (`from_target_json`); authentication, backend discovery, target
  retrieval, submission and result retrieval are all missing, for the three
  reasons [above](#for-ibmbackend-three-reasons).
- **Result retrieval, on any backend.** `ExecutionResult` is defined,
  serialized and unit-tested, and **nothing constructs one** outside
  `src/backend/result.rs`'s own tests. The Rust side has no code path that
  produces a result object, because no Rust code path executes.
- **A Rust-side path back from Aer.** `python/oqci/backends/aer.py` returns an
  `AerResult` dataclass; nothing converts it into an `ExecutionResult`, and the
  two shapes are not interchangeable as they stand. `AerResult` has no
  `compilation_duration_ms` or `submission_duration_ms`, and serde requires
  both to be present even though they are `Option` (neither carries
  `#[serde(default)]`); `AerResult.backend_metadata` also holds non-string
  values such as Aer's `time_taken`, where `ExecutionResult::backend_metadata`
  is a `BTreeMap<String, String>`. So the two types agree in spirit and not yet
  in schema, and `a_result_round_trips_through_json` demonstrates Rust→Rust
  only.
- **Cirq and CUDA-Q adapters.** Stage C §7 and §15.2 name both. Neither exists
  in any form — no backend, no profile, no adapter, no import. The
  representation is deliberately SDK-neutral (a structured operation list, not
  any one vendor's format), so nothing *blocks* one; that is not the same as
  one existing.
- **Calibration and error metadata on profiles.** Stage C §4 lists
  "backend error/cost information" among what IBM lowering may need, and
  Stage D §2 lists per-operation costs and optional error/noise metadata.
  `BasisProfile` has no field for either, which is why `Cost::estimated_duration`
  and `Cost::estimated_error` are always `None`, why routing minimizes hop count
  rather than error, and why `ibm-illustrative` carries no device figures. This
  is blocked on the profile growing fields, not on the backend layer.
- **A backend registry a plugin could extend.** §18.3 asks that a new backend
  supply a profile, lowering, an execution interface and a cost model without
  modifying the compiler core. The *trait* boundary is real and sufficient for
  that, but `backend::all()` is a hard-coded `vec![…]` of three constructors,
  and `cost::resolve` is a hard-coded match of two ids. Adding a backend today
  means editing `src/backend/mod.rs`. Nothing dynamic, and no `libloading`.
- **Any noise-model policy.** Aer accepts one from the caller and OQCI supplies
  none, by design (§33.15). There is no default, no preset, and no
  device-derived model.
- **An `oqci run` command.** §19 sketches one. The CLI stops at `oqci prepare`,
  which writes the artifact; running it is the Python adapter's job. See
  [`cli.md`](cli.md).
- **A `--target`/cost report for `ibm-illustrative`.** `oqci lower` resolves the
  target report through `target::builtin::by_id`, and the illustrative IBM
  profile is not in `builtin::all()`. Lowering and preparation work normally for
  that backend; the target/cost section is simply omitted from the report.
