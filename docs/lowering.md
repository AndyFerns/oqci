# Target Lowering — Layout, Routing, Orientation and Decomposition

Status: normative
Implemented by: `src/lowering/`, `src/target/topology.rs`
Verified by: `tests/lowering.rs`, `tests/lowering_equivalence.rs`,
`tests/decomposition.rs`, and the unit tests in `src/lowering/layout.rs` and
`src/lowering/decompose.rs`
Entry points: `lower`, `LoweringConfig`, `LayoutChoice`, `Lowered`,
`LoweringError`, `Layout`, `LayoutStrategy`, `TrivialLayout`, `DenseLayout`,
`CouplingMode`, `RoutingStrategy`, `ShortestPathRouter`, `OrientationPolicy`,
`Scope`, `DecompositionRule`, `RuleSet`, `Exactness`, `ParamTransform`

[`target_model.md`](target_model.md) describes what OQCI knows about a backend.
This document describes what OQCI *does* with that knowledge. The two are
deliberately different layers.

`lower()` now exists end to end: a circuit and a `BasisProfile` go in, and
either a circuit the target model itself certifies as legal comes out, or a
typed refusal naming exactly what could not be done. There is deliberately no
third outcome — no "best effort" circuit that might not run. What is still
missing is listed in [Implementation status](#implementation-status), and the router's
known quality gap is stated plainly in
[Known quality gap](#known-quality-gap-the-router-inserts-more-swaps-than-it-has-to).

The document is ordered bottom-up: the pieces first — layout, coupling modes,
decomposition rules — then [the schedule](#the-lowering-schedule) that assembles
them, then routing, orientation repair and verification. A reader who wants the
shape before the parts should start at
[The lowering schedule](#the-lowering-schedule) and come back.

## Where lowering sits

Stage D §1 fixes the shape of the pipeline:

```text
Abstract Gate Set → Target/Basis Profile → Target Lowering → Executable Circuit
```

Stage D §4 then names four responsibilities — abstract IR, basis profile,
target lowering, routing/mapping — and says they "must not be conflated".
`src/target/` implements the first two; `src/lowering/` implements the third
and fourth.

The split is visible in behaviour rather than merely in directory names.
`target::check` **reports** problems and repairs nothing: it returns every
`Violation` it finds and leaves the circuit untouched. Everything in
`src/lowering/` exists to repair exactly those problems. A validation function
that quietly rewrote the program it was asked to inspect would collapse the two
layers into one, and there would then be no way to ask "is this circuit legal
as written?" — which is the question a report answers and a rewriter destroys.

### Lowering is not a `Pass`

A `Pass` is `Circuit → Circuit`, and `tests/pass_equivalence.rs` asserts that a
pass leaves the state vector alone up to global phase (see
[`pass_manager.md`](pass_manager.md)). That is a **false** specification for
lowering, whose output is deliberately a *permutation* of the input over a
possibly wider register: logical qubit 2 may come out on physical qubit 0, and
the circuit may acquire device qubits the program never declared.

Were lowering registered as a `Pass`, it could be added to
`PassManager::default_pipeline` and the property suite would immediately begin
asserting something untrue about it. So lowering gets its own entry point,
`lower()`, and its own equivalence statement — an isometry comparison related by
the layout rather than by identity of wires, in
`tests/lowering_equivalence.rs`. See
[How lowering is verified](#how-lowering-is-verified) for why the obvious
statement is the wrong one.

### Module layout

```text
src/lowering/
    layout.rs     Layout, LayoutError, LayoutStrategy, TrivialLayout, DenseLayout
    rules.rs      ParamTransform, RuleStep, Exactness, DecompositionRule,
                  RuleSet, RuleError, RuleSetError, the BUILTIN rule table
    decompose.rs  Scope, decompose, rebuild, rebuild_with_width
    routing.rs    OrientationPolicy, orientation_policy, RoutingStrategy,
                  ShortestPathRouter, RoutingOutput, repair_orientation
    mod.rs        lower, LoweringConfig, LayoutChoice, Lowered, LoweringStep,
                  LoweringError, UnroutableReason — the schedule and its errors
```

`CouplingMode` lives in `src/target/topology.rs` rather than here, because it
is a question *about the coupling map* and the coupling map belongs to the
profile. It is documented in this file because lowering is the only consumer
that has to choose between its three answers.

## Layout

Stage D §6 lists "logical qubit IDs, physical qubit IDs, initial layout, layout
updates, … final layout/reporting" as *distinct* required concepts. `Layout`
keeps them distinct.

Until this module existed the compiler had only an implicit identity layout:
`target::check` reads logical qubit `n` as physical qubit `n` because there was
nothing better to read. That assumption is now a named strategy —
`TrivialLayout` — rather than an unstated convention.

`check` itself has still not changed, and deliberately not: it takes no layout
parameter and reads operand indices as physical qubits. Lowering satisfies that
contract from the other side. Routing rewrites every operand into a *physical*
index before `check` ever sees the circuit, so by the time the verification
step runs, "read the identity" is the correct reading. The layout is not
something `check` consults; it is something lowering has already applied.

### A layout is injective, not bijective

A circuit usually has fewer qubits than the device, so the mapping is
logical → physical and injective, not a permutation of the device:

- Every logical qubit has exactly one physical home.
- Physical qubits with no tenant are **ancillas**, and stay in `|0⟩`.
- Nothing may ever map two logical qubits to the same physical one.

`Layout::from_pairs` rejects a collision rather than trusting its caller, and
`Layout::is_injective` is `debug_assert!`ed after every update, so a future
change that breaks the invariant fails where it happens instead of several
stages downstream.

Both directions are stored and kept in step. That is not redundancy for its own
sake: routing asks both questions constantly — "where does logical `q` live?"
when emitting an operation, and "what lives on physical `p`?" when swapping —
and deriving one from the other on every query would be the wrong trade.

### The API

| Item | Behaviour |
|---|---|
| `Layout::trivial(n, device)` | Logical `n` on physical `n`. Errors with `DeviceTooSmall` if the circuit does not fit. |
| `Layout::from_pairs(pairs, device)` | Explicit assignments, validated. |
| `physical(logical)` / `logical(physical)` | The two directions. `logical` returns `None` for an ancilla. |
| `swap_physical(a, b)` | Exchanges whatever occupies two physical qubits — what a routing SWAP does to the mapping. |
| `permutation()` | `logical index → physical index`, ascending by logical qubit. |
| `highest_occupied()` | The highest physical qubit in use, or `None`. |
| `is_injective()` | Whether every logical qubit has a distinct home and the two directions agree. |
| `device_qubits()`, `len()`, `is_empty()` | Device width; number of logical qubits placed. |

`swap_physical` handles one or both qubits being unoccupied. Swapping an
ancilla with a tenant *moves* the tenant, which is exactly what the
corresponding `Swap` instruction does to the state — and is the case an
implementation storing only the forward map gets wrong, because there is
nothing to swap *back*. `swapping_with_an_ancilla_moves_the_tenant` pins it
down. Injectivity is preserved by construction here, since the operation
permutes the occupancy table.

`highest_occupied` exists so that lowering can size its output register from
the layout rather than from the device's full width. A two-qubit program
compiled for a 32-qubit simulator should not become a 32-qubit circuit.

`Layout` serializes as `{ device_qubits, logical_to_physical }` through a
hand-written `Serialize`, not a derived one. The inverse table is recoverable
from the forward one, so emitting it would put the same fact on the wire twice
and let a consumer read a self-contradictory layout. And `QubitId` has no
`Serialize` at all: the IR types are deliberately serde-free so their shape
stays a compiler contract governed by [`ir_spec.md`](ir_spec.md) rather than
acquiring a wire format by accident — the same rule `src/target/legality.rs`
and `src/cli/snapshot.rs` follow.

### Layout errors

| `LayoutError` | Raised for |
|---|---|
| `NotInjective { first, second, physical }` | Two logical qubits assigned the same physical qubit. `first` is always the lower-indexed one. |
| `PhysicalOutOfRange { logical, physical, qubit_count }` | An assignment leaves the device. |
| `Unmapped { logical }` | Assignments are not contiguous from logical qubit 0. Logical qubits are `0..n`, so a gap means a qubit in the circuit has nowhere to live. |
| `DeviceTooSmall { needed, available }` | The circuit declares more qubits than the device has. Raised by `trivial` and by `DenseLayout::plan`; `from_pairs` never raises it, because it is not told what the circuit wanted. |

### Strategies

```rust
pub trait LayoutStrategy: Send + Sync {
    fn id(&self) -> &'static str;
    fn plan(&self, circuit: &Circuit, topology: &Topology)
        -> Result<Layout, LayoutError>;
}
```

`id()` is a stable identifier recorded in lowering output, for the same
reproducibility reason profiles carry `qualified_id()` (Stage D §8). §8.6
requires the mapper to "consult the selected target profile", which is why the
topology is a **parameter** rather than something a strategy infers from the
circuit.

| Strategy | `id()` | Behaviour |
|---|---|---|
| `TrivialLayout` | `"trivial"` | Logical `n` on physical `n`. The old implicit assumption, now named. |
| `DenseLayout` | `"dense"` | Seats interacting logical qubits near each other. |

`DenseLayout`'s algorithm, in full:

1. Count interactions per unordered pair of logical qubits. `interaction_counts`
   keys on `(min, max)`, so `cx q0,q1` and `cx q1,q0` count toward the same
   pair — layout cares about proximity, not direction. A three-qubit gate
   contributes all three of its pairs.
2. Order logical qubits by total two-qubit work, busiest first, ties on the
   lower index.
3. Place each on the free physical qubit minimizing `Σ count × hops` to
   already-placed partners. Ties break on higher degree, then lower index, so
   the first qubit placed lands on the best-connected physical qubit.

Every ordering breaks ties on the lower index, so the strategy is
deterministic: the same circuit and topology always produce the same layout
(`the_dense_strategy_is_deterministic`). A partner in another connected
component is charged `UNREACHABLE_PENALTY = 1 << 20` hops rather than treated
as free — large enough to dominate any real device distance, and deliberately
not `usize::MAX`, which would overflow when multiplied by an interaction count.

Both distance and degree are measured under `CouplingMode::Undirected`, the
most optimistic of the three graphs. On a device where reverse-CX repair is
*not* expressible that is an underestimate of the real cost. It is safe anyway,
for the reason in the next section.

Neither strategy fails on connectivity. `a_disconnected_topology_still_yields_a_layout`
asserts that `DenseLayout` produces a layout even for a topology with no edges
at all. Refusing an unroutable circuit is routing's job, and routing needs a
layout in hand to explain *which* qubits it could not bring together.

### Layout choice cannot make a circuit wrong

Worth stating plainly, because it bounds how much damage a bug in this module
can do. A layout only decides where qubits *start*. Routing repairs whatever
connectivity it leaves broken, and the verification step re-checks the result
against the profile regardless of how it was reached. A bad layout therefore
costs SWAPs; it does not produce an illegal or an incorrect circuit.

`DenseLayout` is a heuristic in exactly that sense, and is allowed to be one.
There is no claim here that it is optimal, or even good — optimal initial
placement is NP-hard, and nothing in this repository measures how close the
heuristic gets.

That argument is no longer conditional. Both the routing step and the
verification step exist, and `lowering_is_equivalent_legal_and_deterministic`
runs the whole property under **both** `LayoutChoice::Trivial` and
`LayoutChoice::Dense`, asserting the same equivalence and the same legality for
each. If a layout choice could make a circuit wrong, that property would fail
on one arm and not the other.

What a layout *does* change is cost, and that is asserted rather than assumed:
`the_dense_layout_reduces_routing_on_a_line` compiles `cx q0,q3` three times
onto `linear-nisq(4)` and requires `dense` to insert strictly fewer SWAPs than
`trivial`. That is a guard against the heuristic silently degenerating into the
trivial layout, not a claim of optimality.

## Coupling modes: three graphs, one coupling map

Stage D §7 is blunt:

> If a backend treats a two-qubit interaction as directed, the target model
> must represent that explicitly. Do not assume that an undirected edge means
> both ordered interactions are equally native.

So edges in a `Topology` are directed: an `(a, b)` edge means a two-qubit
operation with `a` as control and `b` as target is native, and says nothing
about `(b, a)`. `add_undirected` inserts two explicit directed edges rather
than introducing an "undirected" notion the rest of the model would have to
interpret.

That makes "are these two qubits connected?" not one question but three, and
`CouplingMode` forces the caller to say which. Naming the graph at the call
site is the entire point of the enum.

| Mode | Predicate | The question it answers |
|---|---|---|
| `Directed` | `supports(a, b)` | Is this operation native **as written**? |
| `Symmetric` | `supports(a, b) && supports(b, a)` | Can a `SWAP` run here unaided? |
| `Undirected` | `supports(a, b) \|\| supports(b, a)` | Could an interaction happen here at all? |

Note the asymmetry in how operand order matters: under `Directed` the order
*is* the question, and under the other two it is not.

### Why a SWAP needs `Symmetric`

`swap-to-cx` expands `swap a, b` into three alternating CNOTs:

```text
cx a,b ;  cx b,a ;  cx a,b
```

Two of those run one way across the pair and one runs the other way. Writing
the SWAP as `swap b, a` mirrors the sequence but does not remove the
alternation — whichever operand order you choose, the expansion uses **both**
orientations of the pair. A pair with only `(a, b)` declared can host the
first and third CNOT and not the second.

So a SWAP needs `couples(a, b, Symmetric)`. Using `Directed` here would let
routing place a SWAP whose middle CNOT is illegal; using `Undirected` would let
it place one across a strictly one-way link. `a_directed_only_edge_is_unusable_for_an_unaided_swap`
pins the distinction down: across a chain with one one-way link,
`distance(.., Undirected)` is `Some(2)` and `distance(.., Symmetric)` is `None`.

"Unaided" is doing real work in that sentence. A SWAP *can* be made to run
across a one-way edge by repairing the reversed CNOT — but that is
[orientation repair](#orientation-repair), a separate step, and `Symmetric` is
precisely the graph on which no such repair is needed. That is exactly the
choice `routing_mode` makes: `Symmetric` when the repair is unavailable,
`Undirected` when it is.

### Why `Undirected` is conditional

`Undirected` is not a property of the device alone. It encodes the assumption
that a reversed CX is recoverable, which rests on the standard identity

```text
CX(b → a)  =  (H ⊗ H) · CX(a → b) · (H ⊗ H)
```

— conjugating both qubits by `H`. That rewrite is only available if `h` is
*reachable* on the target. Stage D §7 says as much: "If a reverse interaction
can be implemented by basis changes or conjugation, encode the transformation
in the target-specific lowering rules rather than silently reversing operands."

So `Undirected` is a **conditional** graph: it is the right routing graph only
when that repair is genuinely expressible. `topology.rs` deliberately does not
decide that, because the obvious test is the wrong one. "Does the profile list
`h`?" fails on `linear-nisq`, whose basis is `{rz, sx, x, cx}` and which
nonetheless reaches `h` through `h-to-rz-sx`. The right test is over the
decomposition-rule *closure*, which is data this module owns.

`lower` now makes that decision, in one line:

```rust
let can_reverse = reaches(profile, &rules, "h");
```

`reaches` is `profile.supports_operation(m) || rules.rule_for(m).is_some()` —
native, **or** reachable through the rule graph. `routing::routing_mode` then
picks `Undirected` when `can_reverse` and `Symmetric` when not. Deciding from
`supports_operation("h")` alone would make `linear-nisq` refuse every directed
edge: a false negative that looks like caution and is a bug.

Two honest caveats on this, because the predicate is coarser than it looks.

- **It is a CX-specific proxy applied globally.** `h` is the conjugating gate
  for `Cx` and for nothing else in the policy table. A circuit whose only
  two-qubit gate is `Cy` — which has no verified reversal at all — still routes
  under `Undirected` if the target can reach `h`, and the refusal then arrives
  one phase later, from orientation repair, as `UnrepairableOrientation` rather
  than as `Unroutable`. The circuit is still refused, and refused with an
  accurate message; it is simply refused later than it could have been.
  `an_operation_with_no_known_reversal_is_refused_not_guessed_at` is exactly
  that path.
- **`BUILTIN` still contains no rule with source `cx`.** The CX reversal is not
  a `DecompositionRule`; it is an `OrientationPolicy`, and it lives in
  `routing.rs`. That is a deliberate split — see
  [orientation policies](#orientation-policies-are-declared-not-inferred) — but
  it means `reaches(.., "h")` is testing for the *ingredient* of the repair
  rather than for the repair itself.

### Path queries

| Method | Behaviour |
|---|---|
| `couples(a, b, mode)` | Whether an interaction is available under `mode`. |
| `adjacent(q, mode)` | Neighbours under `mode`, **ascending**. |
| `shortest_path(from, to, mode)` | Inclusive of both endpoints, or `None`. |
| `distance(from, to, mode)` | Hop count — one less than the path length. |
| `is_connected(mode)` | Whether every qubit reaches every other. |

`neighbors(q)` is defined as `adjacent(q, Directed)` so the two cannot drift
apart.

Ascending order in `adjacent` is not cosmetic. `shortest_path` is
breadth-first with a FIFO frontier expanding neighbours in ascending order, so
the path it returns is the **lexicographically smallest among the shortest
ones** — the same one every time, regardless of the order edges were inserted.
"Some shortest path" would not be a strong enough contract, because routing
derives a reported final layout from the choice and Stage D §8 requires that to
be reproducible. `shortest_path_picks_the_lexicographically_smallest_of_the_shortest`
asserts both halves on a diamond graph.

`is_connected` is checked up front by routing rather than per-gate: a circuit
whose interacting qubits land in different components can never be repaired by
any number of SWAPs, and discovering that inside a per-gate search means
failing deep in a rewrite instead of at the entrance. Degenerate cases —
zero or one qubit — are connected, not a special-cased panic. A path from a
qubit to itself is `[q]`, of length one and zero hops.

## The decomposition rule model

Stage D §5 requires every non-native operation that reaches target lowering to
have a documented strategy, and enumerates what each rule must define.
`DecompositionRule` is that data model; `RuleSet` is what validates it.

Until now a `BasisProfile` recorded rules as bare identifier strings that
nothing read. Those identifiers are now keys into this library — which is why
`builtin::linear_nisq`'s rule list had to grow from two entries to seventeen.
A profile that names `h-to-rz-sx` but no rule for `swap` cannot be lowered to at
all, because routing inserts `Swap`s and D1 has to get them into the basis.
`RuleSet::new` says so at construction rather than failing later on one unlucky
circuit — `a_target_whose_rules_cannot_reach_its_basis_is_refused_before_any_work`
asserts that `lower` refuses such a profile before touching the circuit.

### Stage D §5's fields

| §5 field | Representation |
|---|---|
| source operation | `DecompositionRule::source` (a mnemonic), with `source_arity` and `source_params` |
| target operation sequence | `steps: &'static [RuleStep]`, each naming `RuleStep::op` |
| parameter transformation | `RuleStep::params: &'static [ParamTransform]` |
| qubit operand mapping | `RuleStep::operands: &'static [usize]`, indices into the **source's** operand list |
| classical/result behaviour | **not stored** — see below |
| semantic-preservation expectation | `DecompositionRule::note`, one line, shown in reports — and enforced numerically by `tests/decomposition.rs` |
| exact or approximate | `DecompositionRule::exactness`, with the effective value folded by `RuleSet::effective_exactness` |
| target-specific cost implications | **not stored** — see below |

Two of those eight are deliberately absent, and the reasoning is worth
recording rather than leaving as an apparent omission.

**Target-specific cost implications.** Storing a cost number on a rule would
create a second source of truth that could contradict the target's `CostModel`
— precisely the failure mode `BasisProfile` already avoids by deciding
measurement legality from `MeasurementSupport` rather than from whether
`"measure"` appears in the operation list. A rule's cost is *derived*, by
costing the sequence it emits against the profile's model. That is Stage E §1's
rule that the target describes what is expensive, applied consistently.

**Classical/result behaviour.** Every rule source and every rule target is a
unitary gate. Rather than modelling a one-variant enum for "this rule has no
classical behaviour", the fact is made structurally true: a `RuleStep` can only
name a gate, and `Measure` and `Reset` are documented as not decomposable. They
are non-unitary; no sequence of gates produces them. The field is not missing,
it is unrepresentable — which is stronger.

### `ParamTransform`: affine, and nothing more

```rust
pub enum ParamTransform {
    Constant(f64),
    Affine { index: usize, scale: f64, offset: f64 },
}
```

`Affine` computes `scale * source[index] + offset`. That covers identity
`(1, 0)`, negation `(-1, 0)`, halving `(0.5, 0)` and `θ + π` `(1, π)` — every
transformation the standard Euler decompositions need.

There is no expression language, and the limitation is the design rather than a
shortcut. An expression language would have to be evaluated against `Param`,
and `Param` is deliberately a symbol *or* a concrete angle, never an expression
(`ir_spec.md` §1.1). Growing an algebra on `ParamTransform` would either force
the same algebra onto `Param` — expanding the IR's contract for one module's
convenience — or leave the two unable to talk to each other. `rotation-merge`
declines to fold `Rz(θ); Rz(φ)` for the same reason (see
[`pass_manager.md`](pass_manager.md)); this is that decision applied at the
lowering boundary.

`ParamTransform::is_transparent()` is true in two cases, for two different
reasons:

- an `Affine` with `scale == 1.0` and `offset == 0.0`, because it passes its
  source parameter through unchanged; and
- a `Constant`, because it reads no source parameter at all — there is no
  symbol for it to damage.

The `Constant` arm is the one worth pausing on, because treating it as opaque
is the intuitive choice and is wrong. `h-to-rz-sx` emits `rz(π/2); sx; rz(π/2)`,
all constants. If a constant counted as non-transparent, then
`RuleSet::is_parameter_transparent("h")` would report `false` — for a gate with
no parameters to lose — and, because the predicate folds over the closure,
that `false` would propagate to every operation that reaches `h`: `cz`, `cy`,
`ccx`. A caller gating on the predicate would then refuse circuits it should
accept, for a parameter that does not exist.

This changes no row of the user-visible table below, because every refusal
there is driven by an `Affine` with a `+π` offset rather than by a constant.
It changes the closure fold for parameterless gates, which is where the bug
would have been.

That single predicate decides the whole of the next section.

## The symbolic-parameter contract

This is the most consequential design decision in the module.

`Param` has no arithmetic. So when a rule must emit `θ + π` and the source's
`θ` is still a `Param::Symbol`, there is no value to emit. Three things could
happen, and two of them are silently wrong.

**Option 1 — apply the transform as if it were the identity.** Emit
`Rz(theta)` where `Rz(-theta)` was meant. This is the worst of the three, not
because the error is large but because it has **no oracle**. `target::check`
cannot catch it: the emitted circuit is perfectly legal. The state-vector
harness cannot catch it either, because it refuses to simulate a symbolic
parameter at all. The bug would ship, and would only surface as wrong numbers
from a variational experiment months later.

**Option 2 — mangle the symbol's name.** Emit `Param::Symbol("theta/2")`.
`Circuit::parameters` would then report a free parameter that nothing knows the
meaning of, and `bind_parameters` would ask a caller to supply a value for it.
Stage F §2 requires the representation to preserve the distinction between a
concrete angle and a symbol, and §6 forbids evaluating symbolic parameters
prematurely; inventing a symbol whose name encodes an unevaluated expression
violates both.

**Option 3 — refuse.** `ParamTransform::apply` returns
`RuleError::SymbolicParameterRequiresTransformation { rule, step, symbol }`,
whose message ends "bind parameters before lowering". This is what the code
does. It is exactly the Stage F §8 boundary: "Before physical execution,
symbolic parameters must be resolved or lowered according to the backend's
capabilities. … Parameter binding must therefore be an explicit compiler/backend
step."

A symbolic parameter therefore survives a rule only if the rule is
*transparent* — every parameter it emits passes its source through unchanged.
`DecompositionRule::is_parameter_transparent` answers that for one rule;
`RuleSet::is_parameter_transparent` folds it over the whole closure, so
lowering can refuse a symbolic circuit **up front**, from the rule graph, before
any circuit is examined. Failing part-way through a rewrite with a half-lowered
circuit in hand is not an option worth offering.

### The user-visible consequence

On `linear-nisq`, whose basis is `{rz, sx, x, cx}`:

| Parameterized gate | Route | Symbolic parameter survives? |
|---|---|---|
| `rz(θ)` | native | **yes** — nothing rewrites it |
| `p(λ)` | `p-to-rz`, `λ` passed through | **yes** |
| `rx(θ)` | `rx-to-rz-sx`, needs `θ + π` | no |
| `ry(θ)` | `ry-to-rz-sx`, needs `θ + π` | no |
| `u(θ, φ, λ)` | `u-to-rz-sx`, needs `θ + π` and `φ + π` | no |

So: **a symbolic parameter survives lowering to `linear-nisq` only on `rz` and
`p`.** Everything else must be bound first.
`transparency_is_computed_over_the_whole_closure` asserts four of those five
rows — `rz`, `p`, `rx`, `u`; `ry` is left to the end-to-end tests — and
`a_symbolic_parameter_survives_a_transparent_rule` checks that `p(theta)` comes
out as `rz(theta)` with the symbol verbatim rather than merely "not refused".

End to end, `a_symbolic_parameter_survives_where_nothing_transforms_it` lowers
`rz(theta); cx` to `linear-nisq` and asserts both halves of the contract: the
lowered circuit still reports `theta` in `Circuit::parameters()`, **and**
`lowered.legality.is_legal()` is `false`. An unbound parameter is still a
legality violation; it is just not a lowering *failure*. The two are different
questions and lowering answers them separately —
`Violation::UnboundParameter` is filtered out of the blocking set in `verify`
and reported through `Lowered::legality` instead.

This is a real restriction on parameterized workloads — a VQE ansatz written
with `ry(θ)` rotations cannot be lowered to this target while `θ` is still
free. `a_symbolic_parameter_needing_arithmetic_is_refused_with_advice` asserts
that the refusal at least says what to do, by requiring the message to contain
"bind parameters"; `binding_a_parameter_makes_the_same_circuit_lower_cleanly`
then shows the advice works. The alternative was a compiler that silently
computed something else, which is not a better position to be in.

## The rule table

Every rule in `BUILTIN`, in the order it is declared. Operand indices are into
the source's operand list; for one-qubit rules they are all `[0]` and are
omitted below.

| Rule id | Source | Emitted sequence | Declared exactness | Used by `linear-nisq` |
|---|---|---|---|---|
| `id-to-nothing` | `id` | *(nothing)* | Exact | yes |
| `swap-to-cx` | `swap` | `cx 0,1; cx 1,0; cx 0,1` | Exact | yes |
| `cz-to-cx` | `cz` | `h 1; cx 0,1; h 1` | Exact | yes |
| `cy-to-cx` | `cy` | `sdg 1; cx 0,1; s 1` | Exact | yes |
| `ccx-to-cx` | `ccx` | the standard six-CNOT Toffoli, 15 steps | Exact | yes |
| `x-to-sx` | `x` | `sx; sx` | Exact | no — `x` is native there |
| `sxdg-to-sx` | `sxdg` | `sx; sx; sx` | Exact | yes |
| `h-to-rz-sx` | `h` | `rz(π/2); sx; rz(π/2)` | UpToGlobalPhase | yes |
| `y-to-rz-x` | `y` | `rz(π); x` | UpToGlobalPhase | yes |
| `z-to-rz` | `z` | `rz(π)` | UpToGlobalPhase | yes |
| `s-to-rz` | `s` | `rz(π/2)` | UpToGlobalPhase | yes |
| `sdg-to-rz` | `sdg` | `rz(−π/2)` | UpToGlobalPhase | yes |
| `t-to-rz` | `t` | `rz(π/4)` | UpToGlobalPhase | yes |
| `tdg-to-rz` | `tdg` | `rz(−π/4)` | UpToGlobalPhase | yes |
| `p-to-rz` | `p(λ)` | `rz(λ)` | UpToGlobalPhase | yes |
| `rx-to-rz-sx` | `rx(θ)` | `rz(π/2); sx; rz(θ+π); sx; rz(5π/2)` | UpToGlobalPhase | yes |
| `ry-to-rz-sx` | `ry(θ)` | `sx; rz(θ+π); sx; rz(π)` | UpToGlobalPhase | yes |
| `u-to-rz-sx` | `u(θ,φ,λ)` | `rz(λ); sx; rz(θ+π); sx; rz(φ+π)` | UpToGlobalPhase | yes |

Sequences read left to right in circuit order: `h-to-rz-sx` applies `rz(π/2)`
first. `sxdg-to-sx` is three `sx` because `SX` has order four — `SX² = X`,
`SX⁴ = I`, so `SX³ = SX†`. For the same reason `SX` is **not** self-inverse and
is never cancelled as such by `gate-cancellation`; see
[`architecture_decision_sx_basis_gate.md`](architecture_decision_sx_basis_gate.md).

Rule identifiers follow the convention `<source>-to-<something>`, and
`every_rule_identifier_names_the_operation_it_rewrites` enforces it rather than
hoping for it. A profile lists rules by identifier, so `swap-to-cx` sitting in a
profile that does not support `swap` should be obvious on sight.

### The exactness column is derived, not asserted

`every_rule_declares_the_exactness_it_actually_has` re-computes each rule's
exactness numerically and compares it to the declaration: entry-for-entry
equality of the two operators gives `Exact`, agreement only up to a shared
phase gives `UpToGlobalPhase`. A rule whose declaration drifts from its
behaviour fails the test suite. A claim nobody checks is a comment.

### Exactness does not compose

A rule's `Exactness` describes that rule **in isolation**, which is what a
per-rule test can check. It is not the fidelity of the lowered circuit.
`cz-to-cx` is exact — but on `linear-nisq` its expansion runs through
`h-to-rz-sx`, which is not, so lowering a `cz` to that target is only exact up
to phase. `RuleSet::effective_exactness` folds the claim over the closure with
`Exactness::combine` (the weaker of two, since one inexact step anywhere makes
the whole expansion inexact):

| Operation | Declared on its own rule | Effective on `linear-nisq` |
|---|---|---|
| `cz` | Exact | **UpToGlobalPhase** (reaches `h`) |
| `cy` | Exact | **UpToGlobalPhase** (reaches `s`, `sdg`) |
| `ccx` | Exact | **UpToGlobalPhase** (reaches `h`, `t`, `tdg`) |
| `swap` | Exact | Exact — reaches only `cx`, which is native |
| `sxdg` | Exact | Exact — reaches only `sx` |
| `id` | Exact | Exact — reaches nothing |
| `rz` | *(native)* | Exact |

Of the seventeen rules `linear-nisq` uses, six are declared exact and only
**three** — `id-to-nothing`, `sxdg-to-sx`, `swap-to-cx` — remain exact once
folded over the closure. A derived value cannot be a lie; a declared one could.

## The global-phase invariant

Every rule marked `UpToGlobalPhase` depends on one invariant, and it is worth
stating as a load-bearing assumption rather than a footnote.

Nothing in OQCI applies a circuit fragment conditionally or under control:
`Instruction` has exactly three variants, `GateKind` is a closed enum with no
control modifier, classical bits are write-only so there is no feed-forward,
and Stage F §5 places measurement-conditioned branches out of scope. A rewrite
that multiplies one fragment's unitary by a scalar therefore multiplies the
**whole circuit** by that scalar, which no experiment can observe.

Measurement and reset do not break it either: `ρ = |ψ⟩⟨ψ|` is unchanged by a
global phase, so every CPTP map is phase-blind.

**The consequence to hold on to:** adding a controlled-composite gate, a
classically-conditioned operation, a `ctrl @` modifier, or any subroutine that
could later be controlled invalidates every `UpToGlobalPhase` rule **at once** —
not one of them, all of them. Under a control, `e^{iφ}U` and `U` are different
operations, and a rewrite that was invisible becomes a bug in every circuit that
used it. Recovering would require either exact replacements for all eleven such
rules or explicit global-phase tracking in the IR (there is no `gphase`
operation today).

This is the same invariant `canonicalize` relies on when it declines to rewrite
`P(π) → Z`, and the reason `pass_equivalence.rs` states its property as
`|⟨ψ_before|ψ_after⟩| ≈ 1` rather than `ψ_before = ψ_after`. It is one
assumption, held in three places.

## Termination

`RuleSet::new` proves that rewriting terminates. This is a proof, not a
heuristic, and the distinction has a concrete payoff: the rewriter needs no
iteration cap to be safe, and there is no "gave up after N rounds" failure mode
to explain to a user.

The argument has three steps.

1. **Acyclicity.** The expansion graph over mnemonics — an edge from a rule's
   source to each mnemonic it emits — must be acyclic. `rank_all` checks this by
   depth-first search with an on-stack marker, so a cycle is reported as
   `RuleSetError::CyclicRuleSet { mnemonic }`, naming the operation that closes
   the cycle, rather than as a stack overflow.
2. **Rank.** Acyclicity makes `rank(m) = longest path from m in the expansion
   graph` well defined. It is computed once, at construction, and cached. A leaf
   — any mnemonic with no rule, i.e. a native operation — has rank 0.
3. **Multiset order.** Each rewrite replaces one operation of rank `r` with a
   sequence of operations all of rank `< r`. The multiset of ranks of the
   circuit's operations therefore strictly decreases in the multiset ordering,
   which is well founded whenever the underlying order is. So no infinite
   rewrite sequence exists, and rewriting terminates from any starting circuit.

Step 3's premise is the one that could silently fail, so
`rank_strictly_decreases_along_every_rewrite` asserts it directly for
`linear-nisq`: for every rule, every emitted mnemonic has strictly lower rank
than the source.

`RuleSet::rank(m)` is public and returns 0 for anything it has no entry for,
which is the right answer for a native operation.

## Rule-set validation

`RuleSet::new(&BasisProfile)` runs five checks. Every one of them fires when
the *set* is built, before any circuit is touched — the same discipline
`BasisProfileBuilder::build` follows. Discovering mid-compilation that a rule
set cannot terminate, or leaves an operation non-native, would mean failing
deep inside a rewrite with a half-lowered circuit in hand and a diagnostic that
names an unlucky circuit rather than the profile that is actually wrong.

| # | Check | Errors | What it buys |
|---|---|---|---|
| 1 | Every named rule exists; no two rewrite the same operation | `UnknownRule`, `DuplicateSource` | "The rule for `cx`" is unambiguous, and a typo in a profile's rule list fails at profile load rather than silently doing nothing. |
| 2 | Each rule is well formed | `MalformedRule` | Declared arity matches the gate table; every step names a registered gate and gives it exactly the operands and parameters it takes; every operand index is `< source_arity` and every parameter index `< source_params`. |
| 3 | The expansion graph is acyclic | `CyclicRuleSet` | Termination, as above. |
| 4 | Closure: everything reachable is native or has a rule | `NotClosed` | The result is *legal*, not merely *finished*. |
| 5 | A one-qubit source never reaches a multi-qubit operation | `ArityIncrease` | The lowering schedule can be a sequence rather than a loop. |

Three of these deserve more than a table row.

**Operand closure** (part of check 2) is the quiet one that everything else
depends on. A `RuleStep`'s operands are indices into the *source's* operand
list, and every index must be within the source's arity — so a rule may only
permute and reuse the qubits it was handed, and can never introduce a new one.
The routing correctness argument rests on this: if decomposition could name a
fresh qubit, it could place a two-qubit gate on a non-adjacent pair and silently
undo routing's work after routing had finished.
`every_builtin_rule_only_permutes_the_operands_it_was_given` checks both the
declaration and the expanded output.

Check 2 also refuses `GateKind::Opaque`. `frontend::map_gate` answers an
unknown name with an opaque gate, which is the right behaviour for a frontend
reading someone else's program and the wrong behaviour here — a rule naming a
gate that does not exist is an authoring mistake, and turning it into an opaque
operation would produce a circuit no backend can run. Resolving step mnemonics
through `map_gate` at all is deliberate: it means this library cannot drift from
the frontends' understanding of a gate name.

**Closure** (check 4) is what the expanded `linear_nisq` rule list exists to
satisfy. `a_rule_set_that_cannot_reach_the_basis_is_rejected_at_construction`
builds a profile with basis `{rz, sx, cx}` naming `ccx-to-cx` but no rule for
`t` or `tdg`, and asserts `NotClosed`. An empty rule set is not the same thing
as an incomplete one: `ideal_simulator` supports every registered gate, so its
rule set is empty and closes trivially.

**Arity non-increase** (check 5) is scoped precisely: it applies only to rules
whose *source* takes one qubit, and asserts that nothing in that rule's closure
is a multi-qubit operation. This is what licenses the lowering schedule to run
single-qubit cleanup *after* orientation repair without reopening the
connectivity question — cleanup cannot introduce a fresh two-qubit gate.
Nothing forbids a two-qubit rule from reaching a three-qubit operation; that
case has no rule depending on it and is not checked.

## Verifying the rule library

`tests/decomposition.rs` is Stage D exit criterion 5, "decomposition rules are
tested". It is the only thing standing between a mistyped angle and a compiler
that silently changes what a program computes, so its strategy is worth stating.

**Operators, not states.** Every rule is checked as a matrix, by simulating it
once per computational basis state and comparing the resulting columns.
Checking a rule on `|0…0⟩` alone would be far too weak: a wrong `swap` or `cx`
decomposition can agree with the real thing on `|00⟩` and disagree everywhere
else.

**One shared phase, not one per column.** The comparison is `|tr(A†B)| ≈ d`,
not "every column matches up to phase". Those are different claims and only the
first is right: if each column had its own phase, a rewrite negating basis state
`|01⟩` and leaving the others alone would pass — and that rewrite is wrong,
because it changes relative phases and so changes what superpositions do.
`a_per_column_comparison_would_have_missed_a_relative_phase` demonstrates the
gap concretely, on the identity versus `Z`.

**Angles chosen to be awkward.** `ANGLES` includes zero, a negative, values on
axis boundaries, and `2.7183456` — deliberately not a named constant, because
an angle with no symmetry cannot hide a sign error the way `π/2` and its
friends can. Every rule is checked at each of them.

**A negative control.** `a_deliberately_broken_rule_is_caught` runs the harness
against `cz-to-cx` with one conjugating `H` dropped and asserts the comparison
rejects it. A check that has never failed is not evidence.

**Independence.** `tests/support/statevector.rs` writes out each gate's matrix
from scratch rather than consulting the tables the rules are built from, so a
mis-signed angle in a rule shows up as a diverging column instead of cancelling
against a matching mistake in the checker.

One honest limit on that last point. Three places in the tree — the module docs
of `tests/decomposition.rs` and `src/lowering/rules.rs`, and the doc comment on
`orientation_policy` in `src/lowering/routing.rs` — cite a second, independent
check against Qiskit's `quantum_info.Operator` in `python/tests/test_rules.py`.
**That file is not present in this tree**: `python/tests/` contains only
`test_adapter.py`. The in-repo state-vector harness is currently the only
executable oracle for both the rule table and the orientation policies. The
claim describes how the identities were arrived at, not something CI re-runs,
and should be read that way until the file exists.

`Layout` no longer lacks integration coverage, which it did while nothing
consumed a layout. `lowering_is_equivalent_legal_and_deterministic` asserts
`is_injective()` on both the initial and the final layout of every generated
case; `the_reported_final_layout_matches_the_emitted_swaps` recomputes the
final layout from the emitted `Swap`s and compares permutations; and
`the_equivalence_check_rejects_a_wrong_layout` shows the comparison notices a
layout that contradicts the circuit. The unit tests in
`src/lowering/layout.rs` still carry injectivity, ancillas and swap
composition.

## The lowering schedule

`lower()` is a **sequence of seven steps, not a loop to a fixed point**:

```text
D0  arity reduction     every gate -> at most two qubits
L   layout              logical -> physical, injective
R   routing             insert Swaps; program order; insertion-only
D1  basis decomposition rewrite everything outside the basis
O   orientation repair  single sweep over reversed two-qubit gates
D2  single-qubit cleanup
V   verify              check() plus what check() cannot see
```

`Lowered::steps` reports the schedule that actually ran, one `LoweringStep` per
phase with a stable `id`, the operation count after it, and a one-line detail.
`the_reported_steps_describe_the_schedule_that_ran` pins the seven ids and their
order, so a phase that is silently skipped or reordered fails the suite rather
than quietly changing what the compiler does. Three of the steps — D0, D1 and
D2 — are the *same* function, `decompose::decompose`, run at three different
`Scope`s:

| `Scope` | In scope | Phase |
|---|---|---|
| `ArityOnly` | arity > 2 and not native | D0 |
| `Basis` | anything not native | D1 |
| `SingleQubitOnly` | arity == 1 and not native | D2 |

Two orderings in that schedule are load-bearing, and both close a hole rather
than express a preference.

### Why D0 must precede layout

A coupling map describes **pairs**. "Which pair should be adjacent?" has no
answer for a three-qubit gate, so routing cannot make three qubits mutually
adjacent and would pass a `Ccx` through untouched.

That alone would only be a missed optimization. What makes it a correctness
hole is the other half: `target::check` only checks connectivity for operations
with *exactly two* operands (`legality.rs`, `if qubits.len() == 2`). A surviving
`Ccx` is therefore **reported legal**. That is a complete path from a valid
input, through a compiler that raises no error, to a "verified" output no device
can run — the worst failure mode this repository has, because every checker
agrees and the circuit is still wrong.

Reducing arity first closes it, and gives layout a real two-qubit interaction
graph to count over rather than a graph with a three-way clique in it.

`a_three_qubit_gate_is_reduced_before_anything_consults_the_coupling_map`
demonstrates the hazard before demonstrating the fix: it first asserts that
`check` reports **no** `ConnectivityViolation` for an unlowered `ccx` on
`linear-nisq(3)` — "which is the whole problem" — and then lowers the same
circuit and asserts every surviving operation has at most two operands.

### Why orientation repair is separate from decomposition

Putting orientation repair inside decomposition creates an apparent cycle:

```text
swap -> cx        (swap-to-cx)
cx   -> h         (when reversed: CX(b,a) = (H⊗H) CX(a,b) (H⊗H))
h    -> rz, sx    (h-to-rz-sx)
```

and `cx` re-enters through orientation. A rewriter that ran all of this to a
fixed point would have no termination argument, and `RuleSet::new`'s acyclicity
proof would have to be abandoned — a `cx` rule would make the expansion graph
genuinely cyclic and `RuleSetError::CyclicRuleSet` would fire at construction.

The cycle is an **artifact of conflating two different measures**. Decomposition
reduces a gate's *mnemonic* toward the basis. Orientation repair reduces its
*operand order*. Those are independent quantities, and the apparent loop only
exists if you project both onto one axis. Separated, each phase terminates for
its own reason:

| Phase | Terminates because |
|---|---|
| D0, D1, D2 | `RuleSet::new` proved the expansion graph acyclic, so every rewrite strictly decreases a well-founded rank (see [Termination](#termination)). |
| O | It is a **single sweep**, not a fixed point. The two-qubit gate it emits is natively oriented *by construction* — it writes the operands in the order the device declares — so it can never need repair again. |
| D2 after O | No one-qubit rule can reach a two-qubit operation, so cleanup cannot reopen the connectivity question. This is check 5 of [rule-set validation](#rule-set-validation), proved once at construction rather than assumed per circuit. |

This is why there is no outer loop and no iteration cap in `lower`. The one
exception is `MAX_DEPTH = 64` inside `decompose_instruction`, which is an
assertion rather than a policy: the deepest legitimate chain in `BUILTIN` is
three (`ccx -> h -> rz`), so reaching 64 means `RuleSet::new` has a hole, and
`LoweringError::DecompositionDidNotConverge` says exactly that rather than
overflowing the stack.

## The invariant chain

What makes the output *legal*, rather than merely *finished*. Each link is
established by one phase and preserved by the rest.

| # | Statement | Established by | Preserved because |
|---|---|---|---|
| **I1** | Every gate has arity at most two. | D0 | No rule may increase a one-qubit source's arity (check 5), and routing inserts only two-qubit `Swap`s. Re-asserted at V. |
| **I2** | Every two-qubit gate sits on a coupled pair. | R | See I3. |
| **I3** | A decomposition rule may only permute the operands it was given, never name a new qubit. | `RuleSet::new`, check 2 | Enforced when the rule set is built, so it holds for every circuit before any circuit is seen. |
| **I4** | Every two-qubit gate is natively oriented, and every mnemonic is in the basis. | O and D2 | O emits in the declared operand order; D2 only rewrites one-qubit gates. |

The load-bearing composition is **I2 ∧ I3 ⇒ I2 survives D1, O and D2**. Because
a rule can only permute the operands it was handed, no rewrite after routing can
move a gate onto a pair routing did not make adjacent. Without I3 this would not
follow: a rule that introduced a fresh qubit could place a two-qubit gate on a
non-adjacent pair and silently undo routing's work *after* routing had finished,
with nothing between it and the output.
`every_builtin_rule_only_permutes_the_operands_it_was_given` checks I3 on both
the declaration and the expanded output.

I3 also buys **measurement terminality for free**. Since rules preserve operand
sets, the question "is this wire touched after instruction *n*?" has the same
answer before and after decomposition — so a program that was terminal-measured
on entry is still terminal-measured on exit, and D1 cannot turn a legal
measurement into a mid-circuit one.

`every_two_qubit_operation_ends_up_on_a_declared_coupling` asserts I2 and I4
together, directly against `Topology::couples(.., CouplingMode::Directed)`
rather than through `check`, so the two are not checking each other.

## Routing

`ShortestPathRouter` is the one implementation of `RoutingStrategy`:
deterministic shortest path, **no lookahead**. §8.7 asks routing to detect
non-local two-qubit operations, select a strategy, insert SWAPs, update the
mapping, maintain correctness and report the overhead; it does all six.

### Insertion-only, in program order — and why that is the whole correctness proof

The router walks the instruction list in program order. It **deletes no
operation, reorders no pair of original operations, and only inserts `Swap`s
between existing instructions.**

That one sentence discharges §33.14 — "no operation may be optimized away across
a measurement/reset/control barrier without proving the transformation safe".
Nothing is optimized away and nothing moves past anything, so every
`DepKind::Control` edge of the input DAG survives into the output unchanged.
There is no proof obligation left to discharge.

This is worth contrasting with how the pass framework satisfies the same rule.
A `Pass` deletes and merges operations, so it has to reason about barriers, and
[`pass_manager.md`](pass_manager.md) explains how QCO-IR's `Control` edges make
that mechanical. Routing does not need that machinery at all, because it never
does the thing §33.14 restricts.

A router with lookahead would produce shorter circuits and would owe that proof.
The simplicity here *is* the correctness argument. The extra SWAPs are real and
are accounted for in [Known quality gap](#known-quality-gap-the-router-inserts-more-swaps-than-it-has-to).

### The algorithm

For each two-qubit gate, in order:

1. Map both logical operands through the **current** layout to physical qubits.
2. If they already `couples(a, b, mode)`, emit the gate on `(a, b)` and move on.
3. Otherwise take `topology.shortest_path(a, b, mode)` and walk the **first**
   operand along it, emitting a `Swap` per hop and calling
   `Layout::swap_physical` after each, stopping one hop short so the two
   operands end up adjacent.
4. Re-read both operands from the updated layout and emit the gate.

Three details in there are contract rather than implementation.

**Which endpoint moves is part of the contract.** Moving the second operand
instead would give a different final layout for the same input. `shortest_path`
already returns the lexicographically smallest of the shortest paths, so fixing
the moving endpoint is the remaining degree of freedom, and fixing it is what
makes the whole router reproducible. Stage D §8 requires that: the reported
final layout is what a caller uses to interpret which physical wire a
measurement result came from, so "some shortest path" is not a strong enough
contract. `lowering_is_equivalent_legal_and_deterministic` asserts the circuit,
the final layout and the swap count are all identical across two runs of the
same input.

**Which graph paths are planned on** is `routing_mode(can_reverse)`:
`Undirected` when a reversed CX is expressible on this target, `Symmetric` when
not. See [Why `Undirected` is conditional](#why-undirected-is-conditional). When
`Undirected` is in play, an inserted `Swap` may cross a one-way edge — that is
sound only because D1 expands it into three CNOTs and O then repairs whichever
of them faces the wrong way. The three phases are a unit; `route: true,
decompose: false` deliberately does not produce a legal circuit, and says so.

**Operand positions, not `control()`/`target()`.** Routing reads `qubits[0]` and
`qubits[1]` directly. `Instruction::control` and `Instruction::target` answer
only for `Cx`/`Cy`/`Cz`/`Ccx`; they return `None` for `Swap` and for a two-qubit
`Opaque`. A router keyed off them would silently skip the very gates it inserts.
`target::check` reads operand positions for the same reason, so the two agree by
construction.

`swap_instruction` writes each inserted `Swap` in an order the device declares,
choosing `(from, to)` or `(to, from)` by `topology.supports`. `check` reads
`qubits[0]` as the control of *any* two-qubit operation, `Swap` included, so a
swap emitted the other way round is reported as a connectivity violation even
though the operation is symmetric. Emitting it the right way round is cheaper
than teaching `check` about symmetry, and keeps `check` conservative.

### Classical bits are never remapped

A layout is a statement about **qubits**. A measurement's destination register
is untouched and `num_clbits` never changes. `routing_never_moves_a_clbit`
measures `q0 -> c1` and `q2 -> c0` across a routed circuit and asserts the
destinations come out as `[1, 0]` — a classic bug, asserted rather than assumed.

### The frozen-wire rule

When a device cannot measure mid-circuit, a measured wire must stay untouched
for the rest of the program. `ShortestPathRouter` keeps a `BTreeSet` of frozen
physical qubits, adds each measured wire to it, and refuses any path that
crosses one.

The concrete counterexample, which is also
`routing_refuses_to_cross_a_measured_wire_it_cannot_reuse`:

```text
target:   linear-nisq(3)      # couplings p0-p1-p2, mid_circuit_measurement: false
program:  measure q1 -> c0
          cx q0, q2
```

Under the trivial layout the measurement freezes `p1`. Then `cx p0, p2` is not
local, and on a 3-line the only path between `p0` and `p2` is `p0 - p1 - p2`,
which runs straight through the frozen wire. Routing refuses with
`Unroutable { reason: MeasurementFrozenWire }`.

**Without the rule**, routing would cheerfully emit `swap p0,p1; cx p1,p2`. That
circuit touches `p1` after `p1` was measured, so `check` would report
`MidCircuitMeasurementUnsupported` — against **instruction 0, the measurement**.
The user would be told their measurement is the problem. It is not: the
measurement was fine as written, and the compiler put the operation after it.
Blaming an instruction that was correct when it was written is worse than
refusing, because it sends the user to fix the wrong line.

Two honest limits on this rule:

- **Only one candidate path is considered.** `shortest_path` returns a single
  path and the frozen check filters that one path. On a graph with a detour —
  a ring, say, where `p0 - p3 - p2` is just as short as `p0 - p1 - p2` — routing
  refuses even though a free path of equal length exists, because the
  lexicographically smallest shortest path is the only one it looks at. On a
  line, where the counterexample above lives, there is no detour and the refusal
  is genuinely forced. This is conservative in the safe direction: it refuses
  circuits it could have compiled, never the reverse.
- **The freeze is only consulted when a gate is non-local.** If two operands are
  already coupled, no path is planned and no frozen check runs. That cannot
  produce a wrong circuit, because a logical qubit being used after its own
  measurement is caught up front by `reject_unsupported_classical`, and no other
  logical qubit can be parked on a frozen wire — doing so would require a `Swap`
  on a path containing it, which is exactly what is filtered.

## Orientation repair

`repair_orientation` is a **single sweep** over the instruction list, run after
D1. For each two-qubit gate it asks the topology three questions in order:

1. Is `supports(a, b)` true — native as written? Leave it alone.
2. Is `supports(b, a)` **also** false? Then the pair is not coupled in either
   direction, which is not an orientation problem at all. Leave it alone: routing
   should have prevented it, and V reports it if routing did not. Silently
   "fixing" it here would mask a routing bug behind an orientation rewrite, and
   the resulting circuit would still be illegal.
3. Otherwise the edge exists only backwards, and the operation's
   `OrientationPolicy` decides what happens.

### Orientation policies are declared, not inferred

Stage D §7 is the governing sentence:

> If a reverse interaction can be implemented by basis changes or conjugation,
> encode the transformation in the target-specific lowering rules rather than
> silently reversing operands.

`orientation_policy(kind)` is that encoding — a closed table, per operation:

| Policy | Operations | What repair emits |
|---|---|---|
| `Symmetric` | `Cz`, `Swap` | Nothing. Exchanging the operands gives the same matrix back, so the repair is a pure relabelling. |
| `ConjugateBoth("h")` | `Cx` | `h a; h b; cx b,a; h a; h b` — four extra one-qubit gates, then D2 lowers them. |
| `None` | everything else, including `Cy` | Refusal: `UnrepairableOrientation`. |

**Declared, never inferred**, and the reason is that guessing wrong is silent. A
`Cx` with control and target exchanged agrees with the real thing on *every
computational basis state* and disagrees only on superpositions. A test written
against `|00⟩`, `|01⟩`, `|10⟩`, `|11⟩` would pass. So anything without a verified
identity is `None`, and the refusal is the honest answer rather than an
optimistic relabelling.

`the_declared_orientation_policies_are_the_identities_they_claim` checks the
table as physics rather than trusting it. It prepares full support on both wires
(`h; t` on one, `h` on the other — a product state would hide the error), then:

- asserts `Cz` and `Swap` really are unchanged by exchanging operands;
- asserts `Cx` is **not** — "if this passed, operands could be swapped freely
  and no repair would be needed", which is the negative control that makes the
  rest of the table mean something;
- asserts the declared repair `CX(a,b) = (H⊗H) CX(b,a) (H⊗H)` holds up to global
  phase.

End to end, `an_operation_facing_the_wrong_way_down_a_one_way_edge_is_repaired`
builds a two-qubit device with only `p0 -> p1` declared, lowers `cx q1, q0`, and
asserts `orientations_repaired == 1`, `legality.is_legal()`, and that
`h-to-rz-sx` fired — so no stray `h` survives into a basis that does not contain
one. That last assertion is what makes D2 necessary rather than decorative.

`an_operation_with_no_known_reversal_is_refused_not_guessed_at` covers the
`None` arm with `cy` on a one-way edge.

### Where this diverges from §7, honestly

§7 says "encode the transformation in the target-specific lowering rules".
`OrientationPolicy` is a compiled-in table keyed by `GateKind`, not a
*target-specific* one: a backend cannot declare its own reversal for an
operation OQCI has not tabulated. `BUILTIN` contains no rule with source `cx`,
so the repair is genuinely not in the rule table at all — it could not be,
because a `cx -> h -> ... -> cx` rule would make the expansion graph cyclic and
`RuleSet::new` would reject it. The split is deliberate and the termination
argument depends on it, but the result is narrower than §7's wording: the
transformation is encoded and never silent, but it is not per target.

## The verification step

V re-runs `target::check` on lowering's own output. That is the backstop: a bug
anywhere above surfaces as a refusal rather than as a circuit that looks fine
and is not.

`verify` then asserts two things **`check` is structurally unable to see**:

| Assertion | Why `check` cannot make it | Error |
|---|---|---|
| Every gate has arity ≤ 2 (when `decompose` is on) | `check` skips connectivity for anything that is not exactly two operands, so a surviving wide gate is reported legal. | `VerificationFailed`, carrying a synthesized `UnsupportedOperation { mnemonic: "ccx (arity 3)" }` |
| No symbolic parameter survives on an operation whose lowering would have had to transform it | `check` reports `UnboundParameter` without distinguishing "awaiting binding" from "should have been bound before lowering", and says nothing about what to do. | `UnboundParameter`, whose message ends "bind parameters before lowering" |

And it makes one deliberate **subtraction**: `Violation::UnboundParameter` is
filtered out of the blocking set. A symbolic parameter on a *transparent* path
is a legitimate state — `rz(theta)` on an `rz`-native target is a parameterized
circuit awaiting binding, not a compilation failure — so it is reported through
`Lowered::legality` and does not fail the call.

Blocking violations only fail lowering when `config.route && config.decompose`.
This is what makes `route: false` an inspection aid rather than a lie:
`skipping_routing_reports_the_illegality_instead_of_hiding_it` lowers a non-local
Bell pair with routing off, gets `Ok`, and asserts `!legality.is_legal()`. The
result is returned *and* labelled illegal, rather than the function pretending
either that the circuit is fine or that nothing happened.

**What `Ok` therefore guarantees**, under the default config: `check` reports the
output legal except possibly for unbound parameters on transparent paths, every
gate has at most two operands, every two-qubit gate sits on a declared coupling
in the declared direction, and every mnemonic is in the basis.

## `lower()` — configuration and output

```rust
pub fn lower(circuit: &Circuit, profile: &BasisProfile, config: &LoweringConfig)
    -> Result<Lowered, LoweringError>;
```

`LoweringConfig` has three fields: `layout: LayoutChoice` (`Trivial`, `Dense`,
or `Explicit(Layout)`), `route: bool` and `decompose: bool`, defaulting to
trivial layout with both phases on. Turning a phase off is an inspection aid,
not a compilation mode.

`LayoutChoice::Explicit` is the one path that bypasses a strategy's own fit
check, so `lower` validates it at the entrance instead — without that guard, a
layout shorter than the circuit surfaces much later as `UnmappedQubit` against
whichever instruction happened to touch the missing qubit first
(`an_explicit_layout_too_small_for_the_circuit_is_caught_at_the_entrance`).

`Lowered` carries what §8.7 and Stage D §6 ask to be reported:

| Field | Meaning |
|---|---|
| `circuit` | The result, over **physical** qubit indices. |
| `initial_layout`, `final_layout` | Where each logical qubit started and ended. Stage D §6's "initial layout" and "final layout/reporting". |
| `swaps_inserted` | §8.7's routing overhead. |
| `orientations_repaired` | How many reversed two-qubit operations O fixed. |
| `rules_applied` | Rule identifiers that fired, deduplicated and ascending. |
| `steps` | One `LoweringStep` per phase. |
| `profile_id` | `id@version`, e.g. `linear-nisq@1`. |
| `legality` | The report for the output, including non-blocking unbound parameters. |

The output register is sized from the highest physical qubit actually used, not
from the device's width. `ideal-simulator` declares 32 qubits; a Bell pair
compiled for it comes back as a **2**-qubit circuit, because a 32-qubit result
would need 2³² amplitudes to simulate and would be useless to every consumer
downstream (`the_output_register_is_sized_to_what_is_used_not_to_the_device`).

On a target with nothing to repair, lowering is the identity:
`an_unconstrained_target_needs_no_work_at_all` asserts zero swaps, zero
orientation repairs, no rules applied, and `lowered.circuit == input`.

## Every `LoweringError`

Each variant is a **refusal**, not a failure to try, and each names something a
user can act on. `LoweringError` is `#[non_exhaustive]`.

| Variant | Fires when | What to do about it |
|---|---|---|
| `RuleSet { target, source }` | The profile's rule set is malformed, cyclic, or cannot reach its own basis. Raised before any circuit is touched. | Fix the profile's `decomposition_rules`. The inner `RuleSetError` names the gap. `a_target_whose_rules_cannot_reach_its_basis_is_refused_before_any_work`. |
| `Layout { source }` | No layout could be chosen. In practice always `LayoutError::DeviceTooSmall`: a circuit wider than the device, or an `Explicit` layout shorter than the circuit. The other `LayoutError`s are reachable only through `Layout::from_pairs`, which both strategies satisfy by construction. | Use a wider device or a smaller circuit; fix the explicit layout. `a_circuit_wider_than_the_device_is_refused`, `an_explicit_layout_too_small_for_the_circuit_is_caught_at_the_entrance`. |
| `Rule { source }` | A rule refused to apply to a specific operation — almost always a symbolic parameter it would have to transform. | Bind parameters before lowering. |
| `Ir { source }` | A rewritten instruction list failed QC-IR revalidation on rebuild. | A compiler bug. Rebuilding goes back through `CircuitBuilder` precisely so a decomposition bug becomes this error instead of a corrupt circuit (`a_rebuilt_circuit_is_revalidated`). |
| `NoDecompositionRule { mnemonic, target }` | An operation is outside the basis and the target lists no rule for it. | Add a rule identifier to the profile, or avoid the gate. `an_operation_with_no_rule_is_refused_by_name`. |
| `OpaqueOperation { name, target }` | An opaque gate the target does not natively support. | Declare it in the profile if the backend really implements it (`an_opaque_gate_the_target_supports_is_left_alone`), or replace it. Guessing is not an option: an opaque gate could be a controlled-composite, which is exactly what the global-phase invariant excludes. |
| `Unroutable { index, from, to, reason: NoPath }` | The two operands are in different components of the routing graph. | A property of the **device**. No number of SWAPs helps; use a different backend or a different initial placement of the workload. `a_disconnected_device_refuses_rather_than_emitting_something_illegal`. |
| `Unroutable { index, from, to, reason: MeasurementFrozenWire }` | Every path between the operands crosses a wire already measured, on a device without mid-circuit measurement. | A property of **where the measurement sits**. Move the measurement later, or restructure so the interaction happens before the measurement. See [the frozen-wire rule](#the-frozen-wire-rule). |
| `UnrepairableOrientation { index, mnemonic, detail }` | A two-qubit operation faces the wrong way down a one-way edge and its `OrientationPolicy` is `None`. | Express the operation in terms of one that has a verified reversal — on a directed device, `cx` rather than `cy`. |
| `UnsupportedClassicalOperation { index, operation, target }` | The device cannot `measure`, or cannot `reset`, and the circuit does. Checked against the **input**. | Remove the operation. Neither is decomposable: both are non-unitary, and no sequence of gates produces either. `a_device_that_cannot_reset_refuses_a_circuit_that_does`, `a_device_that_cannot_measure_refuses_a_measurement`. |
| `InputMeasuresMidCircuit { index, qubit, target }` | The **input program as written** measures a qubit and then uses it again, on a device without mid-circuit measurement. | Move the measurement to the end of that qubit's life. See below. |
| `UnmappedQubit { index, qubit }` | A logical qubit has no place in the layout. | Should be unreachable through `lower`'s own entrances, which validate fit first; reaching it means a layout was constructed inconsistently. |
| `DecompositionDidNotConverge { mnemonic, depth }` | Rewriting exceeded `MAX_DEPTH`. | Should be unreachable: `RuleSet::new` makes termination a theorem. Reaching it means rule-set validation has a gap, and the message says so. File it as a bug. |
| `VerificationFailed { violations }` | Lowering finished and its own output is still illegal. | The backstop. Always a compiler bug, never a user error — the whole point is that it is reported rather than returned. |
| `UnboundParameter { mnemonic, symbol }` | V finds a symbolic parameter on an operation whose lowering would have had to transform it. With decomposition on, `Rule` fires first during D1, so this is mostly the backstop for `decompose: false`. | Bind it, as the message says. |

### `MeasurementFrozenWire` and `InputMeasuresMidCircuit` are deliberately different

Both are about a measured wire on a device that cannot measure mid-circuit.
They are separate variants because **the fix is different, and so is who is at
fault**:

| | `Unroutable { reason: MeasurementFrozenWire }` | `InputMeasuresMidCircuit` |
|---|---|---|
| Raised by | routing, mid-schedule | `reject_unsupported_classical`, before D0 |
| Blames | a routing choice this program's measurement placement forced | the program as written |
| Could a different layout or router have avoided it? | Yes, in principle — a smarter router or a luckier placement might find a free path | **No.** The program itself uses a qubit after measuring it; no compilation choice changes that |
| The user's move | move the measurement, or accept the extra structure | rewrite the program |

Collapsing them into one error would tell a user to rewrite their program when
the compiler's own path choice was the problem, or send them hunting for a
routing option when their program is simply not runnable on that device. The
distinction is asserted by two tests that sit next to each other:
`routing_refuses_to_cross_a_measured_wire_it_cannot_reuse` and
`a_program_that_already_measures_mid_circuit_is_reported_against_the_program`.

Checking the *input* up front is the other half of that. If the mid-circuit
check ran only on the output, the diagnostic would name an instruction index
that routing had moved, and the user would be pointed at a line they never
wrote.

## How lowering is verified

`tests/lowering.rs` covers the individual behaviours — 24 tests, each named for
the guarantee it pins. `tests/lowering_equivalence.rs` carries the property, and
its comparison is the part worth explaining, because the obvious version of it
is wrong.

For every generated circuit and generated device, lowering must **either** refuse
with a typed error whose cause the test independently confirms, **or** produce a
circuit that is simultaneously (a) semantically equivalent to the original modulo
the final layout, (b) reported fully legal by `check`, and (c) identical across
repeated runs.

### Why comparing full unitaries fails on *correct* routing

The obvious check — build the whole unitary of the routed circuit, compare it to
the original's — rejects correct output. The worked counterexample, from the
test's own module docs:

```text
program:        C = [ CX(q0, q1) ]
device:         a 3-qubit line
initial layout: q0 -> p0,  q1 -> p2
correct output: SWAP(p0,p1) ; CX(p1,p2)
final layout:   q0 -> p1,  q1 -> p2
```

The routed circuit and the expected `CX(p1,p2)` are genuinely **different**
three-qubit unitaries. They differ on inputs where `p1` is not `|0⟩` — the SWAP
moves whatever was sitting on the ancilla. They agree exactly where it matters:
on the subspace the program actually uses, with ancillas in the ground state.

A full-unitary comparison would therefore report a bug in a correct router.
Worse, the natural "fix" — loosening the tolerance until it passes — would
loosen it far enough to accept real errors.

### The isometry comparison

So the claim is about a **map from the logical input space**, not about a full
operator. For each of the `2^n` logical basis states `j`:

```text
col_R[j] = simulate( prepare j through the INITIAL layout ++ routed circuit )
col_E[j] = simulate( prepare j through the FINAL layout   ++ original, relabelled by the FINAL layout )
assert  | Σ_j ⟨col_R[j] | col_E[j]⟩ |  ≈  2^n
```

Three properties fall out of that shape rather than needing special handling:

- **Permutation** is handled by preparing the input where the logical qubits
  *start* and the expectation where they *end*. This is precisely where a
  layout-inversion bug is caught — the one line in the harness to read twice.
- **Padding** is free. Both sides live on the routed circuit's register and start
  from `|0…0⟩`, so ancillas contribute no amplitude to either.
- **Global phase** is quotiented out by the modulus, the same way
  `pass_equivalence.rs` and `tests/decomposition.rs` do it.

### Why the overlaps are summed before the modulus

`| Σ_j ⟨col_R[j] | col_E[j]⟩ |` is **one shared phase over the whole map**, not
one phase per column. Comparing each column up to its own phase is strictly
weaker and false: it accepts a rewrite that negates a single basis state and
leaves the rest alone. That rewrite is wrong — it changes every relative phase,
so it changes what superpositions do, which is the entire content of a quantum
program. `a_per_column_comparison_would_have_missed_a_relative_phase` in
`tests/decomposition.rs` demonstrates the gap concretely on the identity versus
`Z`.

### Breaking the circularity

The comparison above uses lowering's own reported `final_layout`. A bug that
corrupted the circuit and the layout *in the same direction* would pass
unnoticed. `the_reported_final_layout_matches_the_emitted_swaps` closes that:
it lowers with `decompose: false`, replays the `Swap`s the output actually
contains against the initial layout, and requires the replayed permutation to
equal the reported one. That is what catches "updated the layout but emitted the
swap on the wrong pair".

Two real conditions on the replay, both stated in the test: decomposition must
be off, or the `Swap`s have already become CNOTs and there is nothing to replay;
and the input must contain no `Swap` of its own, because a program-level swap
exchanges the two qubits' *states* while a routing swap exchanges the *mapping*,
and the two are indistinguishable in the output. Keeping that distinction
outside the compiler is what preserves the check's independence — having routing
tag its own insertions would make the test trust the thing it is auditing.

`the_equivalence_check_rejects_a_wrong_layout` then shows the harness can
actually fail: it takes a correctly lowered circuit, overwrites `final_layout`
with `initial_layout` — claiming the qubits never moved when the emitted SWAPs
say otherwise — and asserts the comparison rejects it.

### Coverage floors

A property test that never generates a hard case passes for the wrong reason. On
a four-qubit device a random pair of operands is usually *already* adjacent, so a
naive generator would exercise no routing at all.
`generators_reach_the_hard_cases` samples 200 cases and asserts hard floors:

| Floor | Requirement | Why this one |
|---|---|---|
| SWAPs inserted | ≥ 10% of cases | Otherwise routing is never exercised. |
| Orientation repaired | ≥ 5% of cases | Otherwise a whole phase of the compiler goes untested. |
| Final layout has a 3-cycle | ≥ 2% of cases | Only a cycle of length ≥ 3 distinguishes a layout from its **inverse**. A single swap is its own inverse, as is any product of disjoint transpositions, and those dominate small linear cases — so without this floor a layout-inversion bug would be invisible. |
| Circuit contains a wide gate | ≥ 5% of cases | Otherwise D0 is never exercised. |
| Refusals | **exactly zero** | Deliberately not a floor but a ceiling: the generator builds every device from a spanning tree, so every device is connected and every circuit is lowerable. That all 200 succeed is the *completeness* half of the property — lowering does not refuse work it should accept. |

Refusals get their own test, `refusals_happen_only_when_they_are_forced`, over a
generator that deliberately splits the device into two disjoint halves. It
requires at least a quarter of those cases to be refused, and confirms every
refusal against a witness the test computes itself — `refusal_is_justified`
re-derives reachability from the topology rather than believing the compiler,
and rejects an `UnrepairableOrientation` blamed on `cx`, `cz` or `swap`, all of
which have known reversals. Without that, an implementation that refused almost
everything would sail through the entire suite.

### Evidence the suite can fail

A harness that has never rejected anything is not evidence, so it was made to
reject something. Dropping **one** of the two `H` gates from the `Cx` orientation
repair in `src/lowering/routing.rs` — a plausible typo, and one that leaves the
circuit perfectly **legal**, so `check` reports nothing wrong — makes
`lowering_is_equivalent_legal_and_deterministic` fail with an overlap of
`1.3e-15` where `8` was required.

That number matters. It is not a near miss inside a tolerance: the two circuits
come out **orthogonal**. Restoring the gate returns the suite to green.

This is the failure mode the file exists for. Legality and semantics are
independent properties, and only one of them has a checker inside the compiler.

## Known quality gap: the router inserts more SWAPs than it has to

`ShortestPathRouter` is greedy and has **no lookahead**. It routes each two-qubit
gate against the current layout with no regard for what the next gate will need,
so a pair that is about to be needed again is routinely dragged apart and dragged
back. A SABRE-style router — heuristic search with a lookahead window over the
front layer — would produce measurably shorter circuits on the same inputs.

This is a deliberate trade, and the terms are worth stating plainly:

| | No-lookahead shortest path | SABRE-style lookahead |
|---|---|---|
| SWAP count | Higher, sometimes substantially | Lower |
| Correctness argument | One paragraph: insertion-only, in program order, therefore §33.14 is discharged with no proof obligation | Reordering across barriers must be shown safe, case by case |
| Determinism | By construction — lexicographically smallest shortest path, fixed moving endpoint | Needs a seeded, recorded search to be reproducible at all |

Nothing in this repository measures the size of the gap. There is no benchmark
comparing OQCI's SWAP counts against Qiskit's or against an optimal router, so
the honest statement is "more than necessary, by an unmeasured amount", not a
percentage. `the_dense_layout_reduces_routing_on_a_line` shows the layout
heuristic reduces SWAP count on one hand-built case; that is the only
quantitative statement in the tree about routing cost, and it is about layout
rather than about the router.

Improving this is not blocked on anything — `RoutingStrategy` is a trait with a
stable `id()` for exactly this reason, and a second implementation can sit
beside `ShortestPathRouter` without touching `lower`. What a second
implementation would owe is the §33.14 proof the current one gets for free.

## Implementation status

Per `final-deliverables-spec.md`'s Critical Rule — a feature is not implemented
merely because a module, directory, interface or document names it — this
section tracks what is and is not real. Everything below the first table is
**absent**, not planned-and-partly-there.

### Now done

Recorded because earlier revisions of this document listed all four as absent,
and a status table that only ever grows is not a useful one.

| Spec | Status |
|---|---|
| Arity reduction | **done.** `Scope::ArityOnly`, scheduled as D0 ahead of layout. |
| §8.7 Routing / SWAP insertion | **done.** `ShortestPathRouter` inserts `Swap`s, updates the layout as it goes, reports `swaps_inserted`, and refuses with a typed reason when no path exists. Quality caveat in [Known quality gap](#known-quality-gap-the-router-inserts-more-swaps-than-it-has-to). |
| Stage D §7 orientation repair | **done**, with the scope caveat in [Where this diverges from §7](#where-this-diverges-from-7-honestly). `lower` derives the routing `CouplingMode` from the rule closure via `reaches(profile, rules, "h")`. |
| §8.8 `lower()` entry point | **done.** Takes a `Circuit`, a `BasisProfile` and a `LoweringConfig`; returns a `Lowered` or a typed refusal. |

### Not started

| Spec | Status |
|---|---|
| §9, §10 Backend execution | **not part of `src/lowering/`**, deliberately. Nothing in this module submits, runs or retrieves results; `lower` returns a `Lowered` and stops. A separate `src/backend/` tree now exists and is where that contract lives — it is outside this document's scope and is **not described here**. Read its own documentation for what it does and does not execute; do not infer anything about it from this file. |
| §9 IBM target lowering | **no vendor-specific code in `src/lowering/`**, and that is the design. Neither `lower` nor any rule, policy or strategy here names a device — Stage C §6 rules out `if IBM … else if Cirq …` branching through the compiler core. Both built-in profiles are explicitly synthetic and neither describes real hardware (see [`target_model.md`](target_model.md)); `linear-nisq`'s basis is IBM-*shaped*, which is not the same as IBM-derived. Any IBM-specific stage lives behind the backend contract, not in this module. |
| Cost-model-guided routing | **not started.** `RoutingStrategy::route` receives a `&BasisProfile`, so a `CostModel` is reachable in principle, but `ShortestPathRouter` never asks for one. It minimizes hop count, which is a proxy for cost only on a device where every link costs the same. Nothing weights a path by two-qubit error rate or duration, and `BasisProfile` has no per-link calibration data to weight it with. |
| Lookahead routing | **not started.** No SABRE-style or search-based router exists. The trait is ready for one; see [Known quality gap](#known-quality-gap-the-router-inserts-more-swaps-than-it-has-to) for what the current router costs and what a replacement would owe. |

### Also absent

- **Approximate decomposition.** Stage D §5 asks whether a rule is "exact or
  approximate". `Exactness` has two variants, `Exact` and `UpToGlobalPhase`,
  and both are exact claims — the second is exact about an unobservable phase.
  There is no `Approximate { epsilon }` variant and no rule with a bounded
  error, so the "approximate" half of §5 is representable only by adding a
  variant, which would invalidate nothing but has not been needed yet.
- **Serialized layout reporting.** Stage D §6 lists "final layout/reporting" as
  a required concept. `Lowered` now carries `initial_layout` and `final_layout`
  in process, and `Layout` is `Serialize` — but `Lowered` itself is not, and
  nothing writes a layout into a compilation report, a snapshot or a benchmark
  record. Both layouts are now persisted: `Provenance` carries
  `initial_layout` and `final_layout`, and every prepared executable carries a
  `Provenance`. A consumer outside the process can therefore tell which
  physical wire a measurement result came from, which it could not before.
- ~~Any CLI surface for lowering.~~ **Now present.** `oqci lower` renders
  both layouts, the SWAP count, the rules that fired, each step of the
  schedule and the legality report, with `--layout trivial|dense`,
  `--no-route` and `--no-decompose`; `oqci prepare` writes the executable a
  backend would run. Both reach lowering through `crate::compile`, which is
  the single orchestration path the CLI, the Python SDK and library callers
  all take — see [`compiler.md`](compiler.md). The final layout *is* now
  reported, and travels in the provenance record attached to every prepared
  executable, which is what lets a caller interpret which physical wire a
  measurement came from.
- **Target context *used* by a pass.** `Pass::run` now takes a `&PassContext`
  carrying an optional `&BasisProfile` and `&dyn CostModel`, and `run_optimize`
  populates it when a target is selected — but none of the four generic passes
  reads it; all four bind it as `_context`. Stage E §8 is explicit that target
  awareness must not make every pass backend-specific, so the channel being open
  and unused is the intended state, not an oversight. Lowering deliberately sits
  outside the pass framework either way (see
  [`pass_manager.md`](pass_manager.md)).
- **Profile-supplied rules.** `RuleSet::new` resolves identifiers against the
  compiled-in `BUILTIN` table. A backend cannot supply a rule OQCI does not
  already know, and there is no registry, loader or deserialization path for
  one.
- **Noise- or cost-aware layout.** `LayoutStrategy::plan` receives a
  `&Topology`, not a `&BasisProfile` or a `&CostModel`, so a strategy can see
  connectivity and nothing else. `BasisProfile` has no per-qubit error data to
  give it in any case (see [`target_model.md`](target_model.md)), so this is
  blocked on the profile growing calibration fields, not on the trait.
