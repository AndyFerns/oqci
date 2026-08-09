# ADR: Pure-Rust IR Now, MLIR Integration in Phase 2

**Status:** Accepted. **Applies to:** the Rust IR type design (`src/ir/`).
**This is the binding-constraint document** — it enumerates what must **not**
change in the Rust design in order to keep the future MLIR port mechanical.

## Context

OQCI's long-term middle layer is MLIR (see the README architecture). MLIR brings
a pass manager, a dialect conversion framework, and op verification — real
leverage for Phase 3 optimization passes and multi-target lowering. But MLIR also
brings a C++ build, TableGen, and an FFI boundary from Rust.

Phase 0 builds the IR in **pure Rust**, with **no** TableGen, C++ MLIR library,
or FFI. The `quantum` dialect is specified on paper
([`mlir_dialect.md`](mlir_dialect.md)) and the Rust types are designed to mirror
it one-to-one.

## Decision

**Implement the IR in pure Rust now, but constrain the Rust type design so that
the Phase 2 MLIR port is a mechanical translation rather than a redesign.** Every
Rust op/type has a named dialect counterpart; the seam for MLIR marshalling
(`src/ir/mlir_compat.rs`) exists and is empty.

## Why pure Rust now (not MLIR from day one)

- **Iteration speed.** The IR shape is the thing most likely to be wrong and most
  expensive to retrofit. Iterating on it in Rust — no TableGen regen, no C++
  build, no FFI marshalling — is far faster while the design is still moving.
- **Toolchain surface.** Requiring a full LLVM/MLIR build on every contributor's
  machine (and CI) before a single IR type is settled is a heavy, premature tax.
  (Concretely: the Phase 0 dev environment has no `llvm-as`/`opt` on `PATH`.)
- **Semantics first.** Phase 0's job is to validate the IR against quantum-circuit
  semantics (see the no-frontend ADR). That validation needs the types and the
  conversion/lowering, not a dialect registration.
- **De-risking the port.** A paper dialect spec + a one-to-one Rust mapping means
  Phase 2 is "transcribe the spec into TableGen and write the marshalling," a
  bounded task, instead of "discover the dialect while fighting the build."

## What MLIR adds later (Phase 2)

1. **Pass manager** — hosts Phase 3 passes (cancellation, fusion, scheduling,
   routing) over `quantum.circuit` regions, with pass scheduling/analysis reuse.
2. **Dialect conversion framework** — `quantum` → QIR/LLVM lowering as rewrite
   patterns, replacing the textual emitter in `qir.rs`. The mapping table in
   [`qir_lowering.md`](qir_lowering.md) §2 becomes the pattern set.
3. **Op verification** — Rust invariants I1–I7 (`ir_spec.md` §3.3) become op
   `verify()` methods, checked by the framework at every stage boundary.
4. **Traits/interfaces** — e.g. a `Collapsing` trait marking
   `quantum.measure`/`quantum.reset`, the MLIR encoding of QCO-IR's
   `DepKind::Control` barrier.

## Binding constraints — what must NOT change

Changing any of these turns the Phase 2 port from mechanical into a redesign.
Each is load-bearing for a specific part of the mapping; do not alter one without
updating this ADR and [`mlir_dialect.md`](mlir_dialect.md).

| # | Constraint | Why it is load-bearing |
|---|-----------|------------------------|
| C1 | **`GateKind` stays a closed enum + a single `Opaque` escape hatch.** | Registered variants → registered ops; `Opaque` → unregistered op. An open/stringly-typed gate would destroy exhaustive lowering and the registered/unregistered split. |
| C2 | **`GateKind` carries parameters only; qubit operands live on the instruction/node.** | This *is* the attribute-vs-operand boundary (`mlir_dialect.md` §3). If qubits moved into `GateKind`, params and operands would be entangled and the mapping would need per-variant special-casing. |
| C3 | **`QubitId`/`ClbitId` stay distinct newtypes.** | They become the distinct SSA types `!quantum.qubit` / `!quantum.result` and their operand type-constraints. Collapsing them to a shared integer erases a verifier rule. |
| C4 | **`Angle` stays a dedicated parameter type.** | It becomes a `FloatAttr`. Reverting to bare `f64` at call sites would scatter the attribute mapping across the codebase. |
| C5 | **All construction goes through `CircuitBuilder` (no public struct mutation).** | Mirrors `OpBuilder`; keeps a single validation point that becomes the op verifier. Ad-hoc mutation paths would have no verifier analogue. |
| C6 | **Validation is centralized and total (invariants I1–I7, no panics).** | Each invariant becomes an op `verify()` rule. A panic-on-malformed path has no MLIR equivalent and would leak as a crash. |
| C7 | **QCO-IR keeps the wire-threaded, value-semantics DAG with explicit Input/Output boundaries and `Data`/`Control` edge kinds.** | The DAG is isomorphic to MLIR SSA use-def chains; boundaries are block args/terminator operands; `Control` edges are the `Collapsing` trait. A different graph model would require re-deriving dependencies in C++. |
| C8 | **`src/ir/mlir_compat.rs` remains the sole MLIR seam; the rest of `ir/` stays MLIR-free.** | Confines the FFI/marshalling blast radius to one module, so Phase 2 does not thread MLIR types through `qc`/`qco`/`convert`/`qir`. |

## Consequences

- **Positive:** fast IR iteration, no premature toolchain tax, a bounded and
  well-specified Phase 2, and a codebase whose MLIR blast radius is one module.
- **Negative / accepted cost:** the constraints above are a standing discipline —
  contributors must respect them, and this ADR must be consulted before changing
  core IR types. The paper dialect spec can drift from the Rust types if not kept
  in sync; the op-correspondence table (`mlir_dialect.md` §5) is the checklist
  that keeps them aligned.

## Related

- [`mlir_dialect.md`](mlir_dialect.md) — the dialect spec, correspondence table,
  and implied TableGen skeleton.
- [`ir_spec.md`](ir_spec.md) — the invariants (I1–I7) and DAG model referenced
  by C6/C7.
- [`architecture_decision_no_frontend.md`](architecture_decision_no_frontend.md)
  — the companion scope ADR.
