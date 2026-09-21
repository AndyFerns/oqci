# `src/` — the OQCI Compiler Crate

This directory is the `oqci` Rust crate: the compiler library plus a thin CLI
binary built on top of it. It is one crate in the workspace defined by the
repo-root `Cargo.toml`; the other member is [`../python/`](../python/).

## Layout

| Path | What it is |
|---|---|
| [`lib.rs`](lib.rs) | Crate root. Declares the top-level modules below and nothing else — there is no logic here. |
| [`main.rs`](main.rs) | The `oqci` binary's entry point. A few lines: it calls `oqci::cli::run()` and maps the result to a process exit code. All behaviour lives in the library so the binary and any other consumer share one implementation. |
| [`ir/`](ir/) | **QC-IR and QCO-IR** — the compiler's intermediate representation, its validation invariants, the QC-IR→QCO-IR conversion, and QIR (LLVM-compatible) lowering. Everything else in this crate is built on top of this module and touches it only through its public API (principally `CircuitBuilder`). |
| [`frontend/`](frontend/) | Translates external programs into QC-IR: a hand-written OpenQASM 3 lexer/parser/translator, and the pure-Rust core of the Qiskit adapter (the PyO3 boundary that feeds it live `QuantumCircuit` objects lives in `../python/`, not here). |
| [`pass/`](pass/) | The pass manager and the target-independent optimization passes (canonicalization, gate cancellation, rotation merging, scheduling-as-analysis). |
| [`analysis/`](analysis/) | Circuit measurement: gate counts, depth, and structural diffing. The single place any metric is computed — the pass manager and the CLI both call into it rather than computing their own numbers. |
| [`target/`](target/) | **What a backend accepts.** Basis profiles, directed connectivity, legality checking and the backend-supplied cost model. It *describes* targets; it does not apply them. |
| [`lowering/`](lowering/) | **Applying a target.** Qubit mapping, routing with SWAP insertion, and basis decomposition — the stages that turn a target-independent circuit into one a specific device will accept. Ends by re-checking its own output against `target/`. |
| [`backend/`](backend/) | The backend contract: target lowering, validation, execution preparation and execution as four separate stages, plus the executable representation and the provenance a result is cited by. No backend executes in this process; each says where execution actually happens. |
| [`compile.rs`](compile.rs) | **The orchestrator.** One path from source text to artifacts: frontend, passes, lowering, preparation. The CLI, the Python SDK and library callers all go through it. |
| [`cli/`](cli/) | The `oqci compile / optimize / analyze / lower / prepare / watch / passes / targets / backends` inspector. Contains no compiler logic of its own; it calls `compile.rs` and renders what comes back. |

## Reading order

If you are new to this codebase, read in this order:

1. [`ir/`](ir/) — nothing else makes sense without the IR.
2. [`frontend/`](frontend/) — how a program becomes a `Circuit`.
3. [`pass/`](pass/) and [`analysis/`](analysis/) — what happens to a `Circuit` next, and how it's measured.
4. [`target/`](target/) — what a device accepts, and what it finds expensive.
5. [`lowering/`](lowering/) — how a circuit is made to fit one.
6. [`backend/`](backend/) and [`compile.rs`](compile.rs) — how a fitted circuit becomes something runnable, and what ties the whole pipeline together.
7. [`cli/`](cli/) — how all of the above is exposed to a user.

Each module has its own `mod.rs` doc comment with the specifics; this file
only orients you at the directory level. The normative specs live in
[`../docs/`](../docs/) — in particular `ir_spec.md`, `pass_manager.md`,
`target_model.md`, `lowering.md`, `backend_contract.md`, `compiler.md` and
`cli.md` — and are the source of truth when this code and a doc comment
disagree.

## A structural rule this crate enforces

Two invariants are enforced by module boundaries, not just by convention:

- **Nothing outside `ir/` constructs a `Circuit` by hand.** It is built only
  through `CircuitBuilder`, so every `Circuit` in existence has already
  passed validation. A pass that produces a bad rewrite gets an `IrError`
  back, not a corrupted circuit silently handed downstream.
- **Nothing outside `compile.rs` orchestrates the pipeline.** The CLI, the
  Python SDK and any library caller take the same path, so every number a
  user sees was computed by `analysis`, `pass` or `lowering` — never by the
  tool doing the printing. Two orchestrators would eventually disagree, and
  the visible one would be the wrong one.
- **`target/` reports; `lowering/` repairs.** `check` never rewrites the
  circuit it inspects, which is precisely what lets it serve as lowering's
  postcondition oracle. A function that repaired what it inspected could not
  also certify it.
- **No vendor name appears in the compiler core.** Backend selection is a
  registry lookup, so adding a device does not touch `compile.rs`, `pass/` or
  `lowering/`.

## What is not here yet

The compiler reaches a validated, target-legal executable and stops there.
What is missing is on the other side of that boundary:

- **Live hardware submission.** `backend/ibm.rs` does lowering, validation,
  executable preparation and provenance; it does not submit. That needs an SDK
  this project does not depend on and credentials it does not have. **No claim
  of IBM hardware executability is made.**
- **Cirq and CUDA-Q adapters.** The contract is SDK-agnostic, so these are
  adapter work rather than compiler work.
- **Benchmark infrastructure**, which waits on the Stage G protocol.

Execution itself is not missing so much as elsewhere: `../python/oqci/backends/aer.py`
runs a prepared executable on Qiskit Aer. It lives there because the project's
non-goals rule out writing a simulator into this crate.

See [`../docs/core_architecture/index.md`](../docs/core_architecture/index.md)
for the full baseline and what is next.
