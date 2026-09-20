# Target Model — Basis Profiles, Topology, Legality and Cost

Status: normative
Implemented by: `src/target/`
Entry points: `BasisProfile`, `BasisProfileBuilder`, `Topology`, `check`,
`CostModel`, `WeightedCostModel`, `builtin`

This document defines what OQCI knows about a backend. Everything before this
layer — the IR, the frontends, the optimization passes — is deliberately
target-independent. Everything after it needs to know which device is being
compiled for, and this is where that knowledge lives.

## Why this is its own layer

Three locked decisions put it here:

| Decision | Requirement | Consequence for this layer |
|---|---|---|
| Stage D §1 | "The abstract OQCI gate vocabulary must not be rewritten every time a hardware backend changes." | A backend supplies a `BasisProfile` describing its operations, connectivity and constraints; the abstract gate set is untouched. |
| Stage C §6 | The generic compiler must not contain `if IBM … else if Cirq …` branching through optimization logic. | Backend-specific facts are data behind one interface, not control flow scattered through `src/pass/`. |
| Stage E §1 | "The compiler optimizer chooses among transformations; the target describes what is expensive." | `CostModel` lives in `src/target/`, not in `src/pass/`. |

Stage D §4 is equally explicit that abstract IR, basis profile, target
lowering and routing/mapping "must not be conflated". This module implements
only the first two of those four responsibilities: it **describes** targets, it
does not **apply** them. That is why `check` reports problems rather than
quietly repairing them, and why nothing here rewrites a circuit.

## Module layout

```text
src/target/
    profile.rs    BasisProfile, BasisProfileBuilder, ParameterConstraint,
                  MeasurementSupport, TargetError
    topology.rs   PhysicalQubit, Topology
    legality.rs   check, LegalityReport, Violation
    cost.rs       Cost, CostModel, WeightedCostModel
    builtin.rs    ideal_simulator, linear_nisq, all, by_id
```

## Basis profiles

`final-deliverables-spec.md` §11 enumerates the conceptual fields a first-class
target profile must be able to express, and notes that "the final Rust schema
may differ in exact naming, but all required semantics must be representable".
`BasisProfile` covers the list:

| §11 field | Representation |
|---|---|
| `profile_id`, `profile_version` | `BasisProfile::id`, `BasisProfile::version` |
| `backend_id` | `BasisProfile::backend_id` |
| `physical_qubit_count` | `BasisProfile::qubit_count`, derived from the topology |
| `topology` | `BasisProfile::topology` |
| `supported_operations` | `BasisProfile::supports_operation`, `BasisProfile::supported_operations` |
| `parameter_constraints` | `BasisProfile::parameter_constraint` |
| `decomposition_rules` | `BasisProfile::decomposition_rules` — identifiers only, see below |
| `measurement_constraints`, `reset_constraints` | `MeasurementSupport` via `BasisProfile::measurement` |
| `capabilities` | `BasisProfile::capabilities` |
| `cost_model_reference` | `BasisProfile::cost_model_id` |

Fields are private. A profile is obtained only from
`BasisProfileBuilder::build`, so a profile in hand is always one that passed
validation — the same relationship `Circuit` has with `CircuitBuilder`.

### Construction errors

`build` rejects a malformed profile rather than letting it surface later as a
confusing legality failure against a circuit that was actually fine.

| `TargetError` | Raised for |
|---|---|
| `MissingField { field }` | An empty or whitespace-only `id`, `version`, `backend_id` or `cost_model_id`. All four are required. |
| `EdgeOutOfRange` | A coupling referencing a qubit outside `0..qubit_count`. |
| `ConstraintForUnsupportedOperation` | A parameter constraint naming a mnemonic the profile does not list as supported — almost always a typo, and ignoring it would make the profile quietly weaker than its author believed. |
| `InvalidParameterRange` | Reversed (`min > max`) or non-finite bounds. |

### Operations are named by mnemonic, not by `GateKind`

A profile declares `"rz"`, not `GateKind::Rz(Param)`. "Supported" is a
statement about a gate *kind*, independent of the angle any particular instance
carries, and `GateKind::mnemonic` already yields exactly that — so the target
layer needs no parallel gate vocabulary of its own. A string also lets a
backend name an operation OQCI has no registered variant for; an `Opaque` gate
reports its own name through the same accessor.

### Parameter constraints

`ParameterConstraint { min, max }` is an **inclusive** bound in radians.
Stage D §2 lists parameter domains among what a profile must describe: hardware
implementing `rz` by frame change may accept any angle, while a
pulse-calibrated rotation may not. `admits` tests membership; bounds must be
finite and ordered.

### Measurement and reset

`MeasurementSupport` carries three independent booleans — `measurement`,
`mid_circuit_measurement`, `reset` — because Stage D §2 lists measurement and
reset constraints separately from the gate set. A device may measure only at
the end of a circuit, or may not implement reset at all. The default is
`unrestricted()`: all three true.

### Profiles are snapshots

Stage D §8 requires every profile used in an experiment to carry a stable
identifier and version, and requires benchmark results to record which profile
produced them. `BasisProfile` is therefore `Serialize` and backed by ordered
collections (`BTreeSet`, `BTreeMap`), so two profiles built by different code
paths serialize byte-identically. `qualified_id()` produces `"<id>@<version>"`,
the form that belongs in result provenance.

## Topology and directionality

Stage D §6 requires the profile to expose topology so mapping and routing can
consume it. Stage D §7 fixes the shape that takes:

> If a backend treats a two-qubit interaction as directed, the target model
> must represent that explicitly. Do not assume that an undirected edge means
> both ordered interactions are equally native.

**Edges here are directed.** An `(a, b)` edge means a two-qubit operation with
`a` as control and `b` as target is native; it says nothing about `(b, a)`.
`Topology::supports(a, b)` and `supports(b, a)` are independent questions.

The asymmetry is not hypothetical: on superconducting hardware a reversed CNOT
costs surrounding basis changes. Encoding that as a missing edge means routing
learns it from the topology rather than from a comment.

The convenience constructors do not weaken this. `add_undirected(a, b)`
inserts **two explicit directed edges** rather than introducing an "undirected"
notion the rest of the model would have to interpret; `linear` and `all_to_all`
declare both directions of every link the same way.

| Constructor / method | Behaviour |
|---|---|
| `disconnected(n)` | `n` qubits, no couplings. |
| `all_to_all(n)` | Every ordered pair of distinct qubits. No self-coupling. |
| `linear(n)` | `0 ↔ 1 ↔ … ↔ n-1`, each link declared in both directions. |
| `add_directed(c, t)` | One direction only; the reverse stays absent. |
| `add_undirected(a, b)` | Both directions, as two directed edges. |
| `neighbors(q)` | Qubits reachable from `q` **as a control**, ascending — outgoing edges only. |
| `is_symmetric()` | Whether every declared coupling has its reverse declared. Reported, never assumed. |

`edge_count()` counts **directed** couplings. A symmetric line of `n` qubits
therefore has `2(n-1)` edges, not `n-1`; `all_to_all(3)` has 6, not 3.

Edges are stored in a `BTreeSet`, so insertion order and duplicate inserts do
not affect equality or serialization — a requirement of the snapshotting rule
above.

`PhysicalQubit` is a distinct type from the IR's `QubitId`. Conflating "qubit 3
in the program" with "qubit 3 on the chip" is precisely the bug layout and
routing exist to prevent, and Stage D §6 lists logical and physical qubit ids
as distinct required concepts.

## Legality checking

`check(&Circuit, &BasisProfile) -> LegalityReport` is Stage D exit criterion 4:
"Unsupported operations are detected before execution." It is the point at
which a circuit stops being target-independent and has to answer for itself
against a specific device.

### It reports, and it never repairs

`check` rewrites nothing and returns **every** violation it finds, in program
order, not the first. Making a circuit legal is target lowering — mapping,
routing, decomposition — which Stage D §4 keeps separate by design. A
validation function that silently changed the program it was asked to inspect
would conflate the two. Reporting the whole list is the matching ergonomic
choice: someone fixing a circuit by hand, or a future routing pass sizing up
how much work a target needs, wants all of it at once.

`LegalityReport::is_legal()` is true exactly when `violations` is empty;
`violation_count()` gives the length.

### Violations

| Variant | Meaning |
|---|---|
| `UnsupportedOperation { index, mnemonic }` | The gate's mnemonic is not in the profile's basis set. |
| `QubitOutOfRange { index, qubit, qubit_count }` | The circuit uses a qubit index the device does not have. |
| `ConnectivityViolation { index, control, target }` | A two-qubit operation spans a coupling the device does not provide **in that operand order**. |
| `ParameterOutOfRange { index, mnemonic, value, min, max }` | A concrete parameter falls outside the operation's declared domain. |
| `UnboundParameter { index, mnemonic, symbol }` | The parameter is still symbolic, so its domain cannot be checked. Assuming it fits would be exactly the silent guess this layer exists to prevent. Bind parameters before checking against a constrained profile. |
| `MeasurementUnsupported { index }` | The device cannot measure. |
| `MidCircuitMeasurementUnsupported { index, qubit }` | The device measures only terminally, and this measured qubit is operated on again later. |
| `ResetUnsupported { index }` | The device cannot reset. |

Two details of the reporting are deliberate:

- Connectivity is **not** reported for an operand already flagged as out of
  range. One problem gets one message; adding "…and they aren't coupled" on top
  would be noise.
- `UnboundParameter` is raised only for operations that actually carry a
  declared parameter constraint. An unconstrained operation's symbolic
  parameter is not a target-legality question.

Measurement and reset legality is decided by `MeasurementSupport`, not by
whether `"measure"` or `"reset"` appears in `supported_operations`.

### Logical qubits are read as physical ones

There is no layout step yet, so a circuit's `QubitId(n)` is checked against
`PhysicalQubit(n)` — the identity layout. That is the honest reading of an
unmapped circuit, and it is why a `ConnectivityViolation` here means **"this
circuit will not run as written"**, not "this circuit can never run on this
device". Routing is what closes that gap, and it does not exist yet.

`Violation` serializes with an internally tagged `kind` field. The IR types are
deliberately serde-free — their shape is a compiler contract governed by
[`ir_spec.md`](ir_spec.md) — so the wire form of a `QubitId` is produced here
rather than by deriving `Serialize` on the IR type.

## The cost model

Stage E §2: the relative importance of two-qubit burden, depth, routing
overhead, duration and error rates varies by backend, so a universal weighted
score baked into the optimizer would be a hidden assumption wearing the costume
of a measurement.

### `Cost`

Every field is a raw, independently meaningful metric except `scalar_score`,
which is derived and optional. §12.2 requires the first six to be retained at
minimum.

| Field | Meaning |
|---|---|
| `total_gate_count` | All operations, including measurement and reset. |
| `one_qubit_count` | Unitary gates on exactly one qubit. |
| `two_qubit_count` | Unitary gates on two **or more** qubits — `ccx` counts here. |
| `depth` | ASAP scheduling depth. |
| `swap_count` | `Swap` operations present in the circuit. |
| `native_gate_count` | Operations in the target's basis set. |
| `non_native_gate_count` | Operations outside it, which would need decomposition before execution. |
| `estimated_duration` | `Option<f64>` — timing, when the backend can supply it. |
| `estimated_error` | `Option<f64>` — error contribution, when the backend can supply it. |
| `scalar_score` | `Option<f64>` — derived, meaningful only alongside the `configuration()` that produced it. |

`swap_count` counts only swaps the program itself contained, because routing
does not exist yet. It is a real field rather than a placeholder because
Stage E §4 requires routing overhead to be a retained component, and a routed
circuit will populate it without this type changing.

`estimated_duration` and `estimated_error` are `None` when no such data exists,
**not** fabricated zeros. A zero would read as "this circuit takes no time" or
"this circuit is error-free"; `None` reads as "this backend supplied no timing
or error data", which is the truth.

### Components are never discarded

Stage E §7 forbids a benchmark reporting only `OQCI cost = 123.4`, because that
hides why the optimizer decided what it decided. `Cost` therefore exposes the
scalar **alongside** the components, never instead of them, and the scalar is
an `Option` so a model that declines to derive one is representable.

### The `CostModel` trait

```rust
pub trait CostModel: Send + Sync {
    fn id(&self) -> &str;
    fn version(&self) -> &str;
    fn configuration(&self) -> BTreeMap<String, String>;
    fn evaluate(&self, circuit: &Circuit, profile: &BasisProfile) -> Result<Cost, IrError>;
    fn compare(&self, a: &Cost, b: &Cost) -> Ordering;
}
```

`id`, `version` and `configuration` are part of the contract rather than an
implementation detail because Stage E §9 and §12.4 require a compiled result to
be attributable to the cost model and configuration that guided it.
`evaluate` covers §12.1's "candidate evaluation" and "structured cost
breakdown"; `compare` covers "ordering/comparison", cheapest first.

`evaluate` derives its components from `analysis::analyze` — the single place
any metric is computed, and the same function `oqci analyze` calls — so the
compiler's account of a circuit and the tool's cannot disagree. Native versus
non-native is then split by consulting the profile's basis set, with
measurement and reset contributing the mnemonics `"measure"` and `"reset"`.

### Weights are configuration, not literals

Stage E §6 rejects a scalar such as `0.6·depth + 0.3·CX + 0.1·gates` when the
coefficients are arbitrary: it "has no inherent scientific legitimacy unless
the experiment establishes why those coefficients are appropriate". §12.3 adds:
"Do not bury arbitrary weights in optimization code." §33.13 restates it as a
rule for implementers.

`WeightedCostModel` satisfies this by making the weights **public, explicit
fields** that are reported through `configuration()`, so any result derived
from a scalar carries the weights that produced it.

| Weight | Applies to | `new()` default |
|---|---|---|
| `two_qubit_weight` | `two_qubit_count` | 10.0 |
| `depth_weight` | `depth` | 1.0 |
| `gate_count_weight` | `total_gate_count` | 1.0 |
| `swap_weight` | `swap_count` | 30.0 |
| `non_native_weight` | `non_native_gate_count` | 5.0 |

The score is the plain linear combination of those five terms.

These defaults encode nothing beyond an ordering preference the components make
visible anyway — two-qubit operations above depth, depth above raw gate count.
They are a starting point for an experiment to replace, **not** a claim that
these coefficients are correct for any real device. Stage E §5 explicitly
declines to fix numeric weights in architecture, which is why they live in a
constructor an experiment can override field by field rather than in the
architecture documents.

## Built-in profiles

Stage D exit criterion 2 asks for at least one backend profile implemented end
to end. `builtin` provides two, and **both are explicitly synthetic. Neither
describes real hardware.**

`final-deliverables-spec.md` §9.2 forbids hard-coding one physical IBM device
name into the compiler core, and §33's anti-hallucination rules forbid
implementing vendor specifics from memory rather than from checked
documentation. So these profiles describe *shapes* of target rather than
pretending to calibration data they do not have. A real device profile is data
that belongs to a backend adapter, loaded from that backend, not a literal
compiled into the compiler.

### `ideal_simulator()`

| Property | Value |
|---|---|
| id / version / backend | `ideal-simulator` / `1` / `simulator` |
| Qubits | 32, `all_to_all` |
| Basis | every registered `GateKind` mnemonic, plus `measure` and `reset` |
| Measurement | `unrestricted()` |
| Capabilities | `all-to-all-connectivity`, `unrestricted-parameters` |
| Cost model reference | `uniform` |

The reference point: any legal circuit stays legal here, which makes it useful
for confirming that a legality failure elsewhere is really about the *target*
and not about the circuit. A module test asserts that every registered
`GateKind` is present, so a future gate variant cannot be added without this
profile learning about it.

### `linear_nisq(qubit_count)`

| Property | Value |
|---|---|
| id / version / backend | `linear-nisq` / `1` / `generic-nisq` |
| Qubits | caller-supplied, `linear` |
| Basis | `rz`, `sx`, `x`, `cx`, `measure` |
| Measurement | measurement yes; mid-circuit no; reset no |
| Decomposition rule ids | `h-to-rz-sx`, `u-to-rz-sx` |
| Capabilities | `linear-connectivity` |
| Cost model reference | `nisq-weighted` |

The basis is the *shape* an IBM-style superconducting device tends to have —
which is what motivated adding `sx` to the gate set, see
[`architecture_decision_sx_basis_gate.md`](architecture_decision_sx_basis_gate.md)
— but the qubit count, connectivity and constraints are chosen for
illustration, not taken from any real backend. Its couplings are symmetric
because `Topology::linear` declares both directions explicitly; this profile
therefore makes no claim about directional hardware. A genuinely directed
device would declare one direction only.

`all()` returns both, with `linear-nisq` instantiated at width 5 so the set is
enumerable; `by_id` looks one up by `BasisProfile::id`.

## Not implemented

Per `final-deliverables-spec.md`'s Critical Rule — a feature is not implemented
merely because a directory, interface, diagram or README names it — the
following are **absent**:

| Spec | Status |
|---|---|
| §8.6 Qubit mapping | not implemented. No layout representation exists; `check` uses the identity layout. |
| §8.7 Routing / SWAP insertion | not implemented. `Cost::swap_count` counts only swaps already in the program. |
| §8.8 Basis decomposition | not implemented. Non-native operations are *reported*, never rewritten. |
| §9, §10 Backend execution | not implemented. Nothing here submits, runs or retrieves results; there is no execution adapter and no executable representation type. |
| Stage D §5 decomposition-rule data | **identifiers only.** A profile records rule ids as strings. The data model §5 specifies — source operation, target sequence, parameter transformation, operand mapping, exactness, cost implications — lands with the pass that executes it, so it can be designed against a real consumer rather than guessed at now. Nothing currently reads these ids. |

Also absent:

- **Target context on the `Pass` trait.** `Pass::run` still takes only
  `&Circuit` (see [`pass_manager.md`](pass_manager.md)); no pass can consult a
  profile or a cost model. Stage E §8 anticipates target-aware passes, and
  Stage E exit criterion 3 ("optimization can consult backend-defined costs")
  is therefore not yet met.
- **A pluggable cost-model registry.** `cost::resolve` maps the two built-in
  ids (`uniform`, `nisq-weighted`) to configured `WeightedCostModel`s, and
  returns `None` for anything else rather than substituting a default — a
  profile naming a model nothing can resolve would make the provenance it
  records false. But the mapping is a hard-coded match, not a registry a
  backend can extend, and `WeightedCostModel` is still the only
  implementation of the trait.
- **Per-operation costs and error/noise metadata.** Stage D §2 lists
  "operation-specific costs" and "optional error/noise metadata" among what a
  profile should be able to describe. `BasisProfile` has no field for either,
  which is why `Cost::estimated_duration` and `Cost::estimated_error` are
  always `None` today.
- **A `CostModel::explain` method.** Stage E §3 sketches one; the structured
  `Cost` serves the same purpose here, since every component is already public.
- **Backend-supplied profiles.** There is no loader, no deserialization path
  and no adapter; profiles are built in Rust through `BasisProfileBuilder`.
