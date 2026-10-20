# ADR: Pure-Rust IR, MLIR as an Optional Later Layer

**Status:** Accepted. Its binding-constraint table (C1–C8) is superseded by
[`architecture_decision_ir_contract.md`](architecture_decision_ir_contract.md),
which restates those constraints as IR-validity guarantees.
**Applies to:** whether, when and how MLIR enters OQCI.

*Historical note:* this ADR was written in Phase 0 (`0.0.1`), when "Phase 2"
named the planned MLIR integration step; the file name keeps that label. See
[`core_architecture/index.md`](core_architecture/index.md) for how the old
phase labels map to versions.

## Context

MLIR is the candidate middle-layer technology for OQCI's IR (Stage B,
[`core_architecture/stage-b-modular-mlir-boundary.md`](core_architecture/stage-b-modular-mlir-boundary.md));
none of it is implemented, and the root README lists MLIR integration as not
started. MLIR brings a pass manager, a dialect conversion framework, and op
verification. It also brings a C++ build, TableGen, and an FFI boundary from
Rust.

The IR is built in **pure Rust**, with **no** TableGen, C++ MLIR library, or
FFI. The `quantum` dialect is specified on paper
([`mlir_dialect.md`](mlir_dialect.md)) so that the correspondence stays
explicit.

## Decision

**Implement the IR in pure Rust, and keep MLIR an optional layer that may be
added later where Stage B says it provides concrete value.** Every Rust op and
type has a named dialect counterpart, and the seam for MLIR marshalling
(`src/ir/mlir_compat.rs`) exists and is empty. The IR contract that keeps a
future port mechanical — C1–C8 — is owned by
[`architecture_decision_ir_contract.md`](architecture_decision_ir_contract.md).

## Why pure Rust (not MLIR from day one)

- **Iteration speed.** The IR shape is the thing most likely to be wrong and most
  expensive to retrofit. Iterating on it in Rust — no TableGen regen, no C++
  build, no FFI marshalling — is far faster while the design is still moving.
- **Toolchain surface.** Requiring a full LLVM/MLIR build on every contributor's
  machine (and CI) before a single IR type is settled is a heavy, premature tax.
  (When this ADR was written, the development environment had no
  `llvm-as`/`opt` on `PATH`.)
- **Semantics first.** The IR is validated against quantum-circuit semantics
  (see the no-frontend ADR). That validation needs the types and the
  conversion/lowering, not a dialect registration.
- **De-risking a port.** A paper dialect spec plus a one-to-one Rust mapping
  means a port would be "transcribe the spec into TableGen and write the
  marshalling," a bounded task, instead of "discover the dialect while fighting
  the build."

## What MLIR could add

1. **Pass infrastructure** — hosting optimization passes (cancellation,
   fusion, scheduling, routing) over `quantum.circuit` regions, with pass
   scheduling/analysis reuse.
2. **Dialect conversion framework** — `quantum` → QIR/LLVM lowering as rewrite
   patterns. The mapping table in [`qir_lowering.md`](qir_lowering.md) §2
   would become the pattern set.
3. **Op verification** — invariants I1–I8 (`ir_spec.md` §3.3) as op
   `verify()` methods, checked by the framework at every stage boundary.
4. **Traits/interfaces** — e.g. a `Collapsing` trait marking
   `quantum.measure`/`quantum.reset`, the MLIR encoding of QCO-IR's
   `DepKind::Control` barrier.

*Status as of 0.5.0 (unchanged since 0.4.1):* items 1 and 2 have Rust counterparts that exist without
MLIR. The optimization passes run under a Rust `PassManager` (`src/pass/`,
[`pass_manager.md`](pass_manager.md)), target lowering is Rust
(`src/lowering/`), and the textual emitter in `qir.rs` is the only QIR path.
The list above is what MLIR could provide; whether any of it replaces the Rust
implementation is governed by Stage B, and the Rust core stays authoritative
either way (Stage B exit criterion 1).

## Consequences

- **Positive:** fast IR iteration, no premature toolchain tax, a bounded and
  well-specified port if one is ever made, and a codebase whose MLIR blast
  radius is one module.
- **Negative / accepted cost:** the paper dialect spec can drift from the Rust
  types if not kept in sync; the op-correspondence table (`mlir_dialect.md` §5)
  is the checklist that keeps them aligned.

## Related

- [`architecture_decision_ir_contract.md`](architecture_decision_ir_contract.md)
  — the constraints C1–C8, formerly listed here.
- [`mlir_dialect.md`](mlir_dialect.md) — the dialect spec, correspondence table,
  and implied TableGen skeleton.
- [`ir_spec.md`](ir_spec.md) — the invariants (I1–I8) and DAG model.
- [`architecture_decision_no_frontend.md`](architecture_decision_no_frontend.md)
  — the companion scope ADR.
