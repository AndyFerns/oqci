# OQCI IR Specification — QC-IR and QCO-IR

**Status:** Phase 0 (stable). **Scope:** the two IR levels and the conversion
between them. Backend/QIR emission is specified in
[`qir_lowering.md`](qir_lowering.md); the MLIR dialect these types mirror is in
[`mlir_dialect.md`](mlir_dialect.md).

This document is the normative reference for OQCI's intermediate representation.
It defines the value types, the two IR levels (QC-IR and QCO-IR), their
invariants, their operational semantics, and a proof sketch that the
QC-IR → QCO-IR conversion is semantics-preserving.

---

## 1. Value types

All IR levels share a small set of value types (`src/ir/types.rs`). They are
newtype-wrapped rather than raw primitives so that they map one-to-one onto
typed MLIR SSA values / attributes in Phase 2 (see §7 and `mlir_dialect.md`).

| Type | Rust | Meaning | Range / domain |
|------|------|---------|----------------|
| Qubit reference | `QubitId(u32)` | index into a circuit's qubit register | `0 ≤ id < num_qubits` |
| Classical-bit reference | `ClbitId(u32)` | index into the classical register | `0 ≤ id < num_clbits` |
| Angle / parameter | `Angle(f64)` | rotation angle in **radians** | any finite `f64` |
| Gate identity + params | `GateKind` | closed enum + `Opaque` escape hatch | see §2 |

**Design rationale — newtypes over primitives.** A bare `u32` for a qubit and a
bare `u32` for a classical bit are interchangeable to the compiler and to the
type checker; a `QubitId`/`ClbitId` split makes "measure this classical bit as
if it were a qubit" a *compile error*, and makes the eventual MLIR mapping
(`!quantum.qubit` vs `!quantum.result`) mechanical. `Angle` is a distinct type
so that Phase 2 can substitute MLIR's `FloatAttr` representation without editing
any call site. All three carry zero runtime cost.

**Angles are not normalised.** An `Angle` stores exactly the radian value it was
given; the IR never reduces modulo `2π`. This preserves frontend intent and
keeps conversion a pure structural operation. Finiteness (`NaN`/`inf` rejection)
is enforced at validation time (§3.3), not at construction, so that builder
chaining stays infallible.

## 2. Gates: the `GateKind` enum

`GateKind` is a **closed enum of registered gates plus one `Opaque` escape
hatch**, mirroring MLIR's registered-op + unknown-op model. It carries *only the
gate's identity and its angle parameters* — **not** its qubit operands. This
separation is deliberate: parameters lower to MLIR **attributes**, whereas qubit
operands lower to MLIR **SSA operands** (see `mlir_dialect.md` §3).

Registered gates and their fixed qubit arity:

| Gate | `GateKind` | Params | Arity | Notes |
|------|-----------|--------|-------|-------|
| Identity | `I` | — | 1 | |
| Pauli-X/Y/Z | `X`, `Y`, `Z` | — | 1 | |
| Hadamard | `H` | — | 1 | |
| Phase / adj | `S`, `Sdg` | — | 1 | `S = diag(1, i)` |
| π/8 / adj | `T`, `Tdg` | — | 1 | |
| Rotations | `Rx(θ)`, `Ry(θ)`, `Rz(θ)` | 1 | 1 | radians |
| Phase gate | `P(λ)` | 1 | 1 | `diag(1, e^{iλ})` |
| General 1q | `U{θ,φ,λ}` | 3 | 1 | Euler / OpenQASM `U` |
| CNOT / CY / CZ | `Cx`, `Cy`, `Cz` | — | 2 | operands `[control, target]` |
| Swap | `Swap` | — | 2 | operands `[a, b]` |
| Toffoli | `Ccx` | — | 3 | operands `[c0, c1, target]` |
| Escape hatch | `Opaque{name, params}` | n | *variable* | any ≥1 distinct qubits |

**Operand-order convention.** For controlled gates the leading operands are
controls and the final operand is the target. `Instruction::control()` and
`Instruction::target()` expose this without callers hard-coding indices.

**Why an `Opaque` escape hatch and not a stringly-typed gate everywhere.** A
fully open `Gate(String, …)` representation would make every pass stringly-typed
and defeat exhaustive matching. A fully closed enum would reject any gate we
did not foresee (custom pulse-level gates, vendor extensions). The registered
enum + single `Opaque` variant gives exhaustiveness for the common path and an
explicit, clearly-marked slow path for the rest — exactly MLIR's model, so the
Phase 2 mapping is `registered → registered op`, `Opaque → UnregisteredOp`.

## 3. QC-IR — the imperative IR

QC-IR (`src/ir/qc.rs`) is the direct lowering target for future frontends. A
**`Circuit`** is:

- a name,
- a qubit register of size `num_qubits` (owning `QubitId(0..num_qubits)`),
- a classical register of size `num_clbits`,
- an ordered `Vec<Instruction>`.

### 3.1 Instructions

```text
Instruction ::= Gate   { kind: GateKind, qubits: Vec<QubitId> }   // → quantum.gate
              | Measure { qubit: QubitId, target: ClbitId }        // → quantum.measure
              | Reset   { qubit: QubitId }                          // → quantum.reset
```

`Gate` is unitary. `Measure` projects a qubit in the computational (Z) basis and
writes the outcome to a classical bit. `Reset` returns a qubit to `|0⟩`. Both
`Measure` and `Reset` are **non-unitary / state-collapsing**.

### 3.2 Construction — the builder

Circuits are built **only** through `CircuitBuilder` (mirroring MLIR's
`OpBuilder`); direct struct mutation is not possible from outside the module.
Register allocation (`alloc_qubit`, `alloc_clbit`, …) hands back typed ids;
instruction methods are infallible and chainable; **all validation is deferred
to `build()`**, keeping every invariant check in one auditable place.

### 3.3 Well-formedness invariants

`CircuitBuilder::build()` returns `Ok(Circuit)` iff every instruction satisfies,
in order, each rule below; otherwise it returns the first violating `IrError`.
Each rule maps to exactly one error variant and one test.

| # | Invariant | Error variant |
|---|-----------|---------------|
| I1 | every qubit operand is in range (`< num_qubits`) | `QubitOutOfRange` |
| I2 | every measurement target is in range (`< num_clbits`) | `ClbitOutOfRange` |
| I3 | a registered gate has exactly its declared arity | `GateArityMismatch` |
| I4 | no qubit appears twice within one gate | `DuplicateQubit` |
| I5 | an `Opaque` gate has a non-empty name | `EmptyOpaqueName` |
| I6 | an `Opaque` gate acts on ≥1 qubit | `EmptyOpaqueOperands` |
| I7 | every angle parameter is finite | `NonFiniteAngle` |

A `Circuit` value is a *witness* that all seven hold; downstream stages
(conversion, lowering) therefore never re-validate and never panic.

### 3.4 Operational semantics

A circuit denotes a map from an empty input to a classical outcome distribution:

1. Allocate `num_qubits` qubits in state `|0…0⟩` and `num_clbits` classical bits
   set to 0.
2. Execute instructions in list order. `Gate{kind, qubits}` applies the unitary
   `U_kind` to the named qubits (control/target as per §2). `Measure{q, c}`
   samples `q` in the Z basis, collapses it, and stores the bit in `c`.
   `Reset{q}` discards `q`'s state and sets it to `|0⟩`.
3. The observable result is the joint distribution of the classical register.

This is the standard operational semantics of a quantum circuit with mid-circuit
measurement and no classical feed-forward (see §6 for the feed-forward scope
decision).

## 4. QCO-IR — the optimization IR

QCO-IR (`src/ir/qco.rs`) is the same circuit as a **directed acyclic dependency
graph** (DAG). It is the form Phase 3 passes will consume, because it makes
explicit which operations may be reordered.

### 4.1 Graph structure

Every wire — each qubit and each classical bit — is threaded from a boundary
**`Input`** node to a boundary **`Output`** node. Operation nodes (`Op`) lift QC-IR
instructions and sit on the wires they touch. Each node carries:

```text
NodeKind ::= Input(Wire) | Output(Wire) | Op { index: usize, instruction }
Wire     ::= Qubit(QubitId) | Clbit(ClbitId)
```

`index` is the operation's position in the originating program order; it is a
stable identity used to make traversal deterministic (§4.3).

### 4.2 Dependency edges

A directed edge `A → B` labelled with a `Wire` means *the value on that wire
flows from `A` to `B`*, so `B` must execute after `A`. Each edge carries a
`DepKind`:

- **`Data`** — the predecessor produced a quantum or classical value the
  successor consumes: a unitary gate, a boundary input, or a classical write.
- **`Control`** — the predecessor is a **state-collapsing** op (`Measure` or
  `Reset`) on that *qubit* wire. The successor must observe the collapse and
  therefore may **not** be commuted across the predecessor.

**Why distinguish Data from Control.** For pure unitary gates on a shared wire,
a future pass may in principle commute or cancel them subject to the unitary
algebra. An edge out of a measurement/reset is categorically different: it is a
hard barrier that no algebraic identity can cross, because the state was
projected. Tagging the edge kind lets a pass reason about "may I reorder across
this?" locally, without re-deriving which node was a measurement. Classical-wire
edges are always `Data` (classical value flow).

### 4.3 Traversal and determinism

`topological_ops()` returns the operation nodes in a **deterministic**
topological order produced by Kahn's algorithm with ties broken by
`(node-class, program-index)` — boundary inputs first, then ops by ascending
program index, then boundary outputs. Consequences:

- The order is a pure function of the graph (reproducible across runs).
- For a circuit that was linear to begin with, it reproduces the original
  program order exactly.

`linearize()` returns operations sorted by program `index`, i.e. the canonical
QC-IR order — used for round-trip checks.

### 4.4 Invariants

- **Acyclicity.** The graph produced from any valid `Circuit` is a DAG (§5).
- **Wire completeness.** Every wire has exactly one `Input` and one `Output`
  node, and a directed path from input to output through the ops touching it.
- **Boundary purity.** `Input` nodes have no predecessors; `Output` nodes have
  no successors.

## 5. QC-IR → QCO-IR conversion

The conversion (`src/ir/convert.rs`) is a single left-to-right pass.

```text
last[w]           := Input(w)          for every wire w        # last writer
last_collapse[w]  := false                                     # was it a measure/reset?

for (index, op) in circuit.instructions:
    n := new Op(index, op)
    for w in op.qubit_wires (in operand order):
        kind := Control if last_collapse[w] else Data
        add_edge(last[w] -> n, wire=w, kind)
        last[w] := n
        last_collapse[w] := op.is_collapsing        # Measure/Reset
    if op writes clbit c:
        add_edge(last[Clbit(c)] -> n, wire=Clbit(c), kind=Data)
        last[Clbit(c)] := n
for every wire w:
    add_edge(last[w] -> Output(w), wire=w, kind by last_collapse[w])
```

**Determinism.** Instructions are visited in program order; within an
instruction, qubit operands are visited left to right, then the clbit. Node and
edge insertion is thus a pure function of the input circuit.

### 5.1 Semantics-preservation argument

We argue that the DAG denotes the same circuit as the QC-IR it came from, i.e.
every topological order of the DAG is an execution equivalent to the original
program, and the canonical order reproduces the original exactly.

**Claim 1 — the graph is a DAG.** Each added edge goes from an
earlier-or-equal-`last` node to the freshly created node `n`. Because `n` is new
at each step and only becomes a `last[w]` *after* its incoming edges are added,
every edge points from a node created at step `i` to a node created at step
`j > i` (or from a boundary `Input`, created before all ops). A strictly
increasing creation order along every edge forbids cycles. ∎

**Claim 2 — the edge set is exactly the necessary ordering constraints.** Two
instructions in a quantum circuit are order-sensitive **iff they act on a common
wire** (they touch a shared qubit, or one writes and the other reads a shared
classical bit). Instructions on disjoint wires commute (operators on disjoint
tensor factors commute). The conversion adds an edge between two ops precisely
when they are consecutive writers of a shared wire, and transitively chains all
ops on a wire in program order. Hence:

- *(Soundness)* every edge corresponds to a real ordering constraint — it links
  two ops sharing a wire, whose relative order affects the state.
- *(Completeness)* every real ordering constraint is represented — any two ops
  sharing a wire are connected by a directed path along that wire, so no valid
  reordering is lost and no invalid reordering is permitted.

**Claim 3 — topological orders are semantically equivalent.** A topological
order respects every edge, hence every wire-sharing constraint (Claim 2). Any
two orders that both respect all constraints differ only by transpositions of
operations on **disjoint** wires, which commute. Therefore all topological
orders denote the same operator/measurement semantics. ∎

**Claim 4 — the canonical order is the original program.** Program order is
itself a topological order (Claim 1's creation order witnesses it), and
`linearize()` sorts by `index`, so it returns the original instruction sequence
verbatim. Combined with Claim 3, the conversion round-trips exactly and any
optimization performed on the DAG is a re-selection among semantically
equivalent orders. ∎

The tests `linear_chain_linearizes_to_program_order`,
`mid_circuit_measurement_full_pipeline`, and `ghz3_full_pipeline` exercise
Claims 3–4 concretely (shared-wire ops stay ordered; disjoint ops need not).

## 6. Scope decision — no classical feed-forward (Phase 0)

Phase 0 supports measurement and reset but **not** classically-controlled
operations (`if (c == 1) X q`). Consequences that keep this document simple:

- Classical bits are **write-only**: gates never read them, so there is no
  classical→quantum data edge and the DAG cannot gain a cycle through the
  classical register.
- "Control dependency" therefore means *only* the measurement/reset barrier of
  §4.2, not data-dependent branching.

Rationale: feed-forward materially enlarges the DAG (classical read edges), the
semantics (probabilistic branching), and the QIR profile (Adaptive rather than
Base). Adding it before the unitary core is validated would entangle two hard
problems. It is deferred to a later phase; the `Opaque` hatch and the
`#[non_exhaustive]` error enum leave room to add it without a redesign.

## 7. MLIR correspondence (summary)

Every type and op here has a named counterpart in the `quantum` MLIR dialect;
the full table is in [`mlir_dialect.md`](mlir_dialect.md). In brief:
`Instruction::Gate → quantum.gate`, `Measure → quantum.measure`,
`Reset → quantum.reset`; `QubitId → !quantum.qubit` SSA value,
`ClbitId → !quantum.result`; gate params → attributes; qubit operands → SSA
operands. The binding constraints that keep the Phase 2 port mechanical are in
[`architecture_decision_mlir_phase2.md`](architecture_decision_mlir_phase2.md).
