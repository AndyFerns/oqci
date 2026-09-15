# `src/` — the OQCI Compiler Crate

This directory is the `oqci` Rust crate: the compiler library plus a thin CLI
binary built on top of it. It is one crate in the workspace defined by the
repo-root `Cargo.toml`; the other member is [`../python/`](../python/).

## Layout

| Path | What it is |
|---|---|
| [`lib.rs`](lib.rs) | Crate root. Declares the five top-level modules below and nothing else — there is no logic here. |
| [`main.rs`](main.rs) | The `oqci` binary's entry point. A few lines: it calls `oqci::cli::run()` and maps the result to a process exit code. All behaviour lives in the library so the binary and any other consumer share one implementation. |
| [`ir/`](ir/) | **QC-IR and QCO-IR** — the compiler's intermediate representation, its validation invariants, the QC-IR→QCO-IR conversion, and QIR (LLVM-compatible) lowering. Everything else in this crate is built on top of this module and touches it only through its public API (principally `CircuitBuilder`). |
| [`frontend/`](frontend/) | Translates external programs into QC-IR: a hand-written OpenQASM 3 lexer/parser/translator, and the pure-Rust core of the Qiskit adapter (the PyO3 boundary that feeds it live `QuantumCircuit` objects lives in `../python/`, not here). |
| [`pass/`](pass/) | The pass manager and the target-independent optimization passes (canonicalization, gate cancellation, rotation merging, scheduling-as-analysis). |
| [`analysis/`](analysis/) | Circuit measurement: gate counts, depth, and structural diffing. The single place any metric is computed — the pass manager and the CLI both call into it rather than computing their own numbers. |
| [`cli/`](cli/) | The `oqci compile / optimize / analyze / watch / passes` inspector. Contains no compiler logic of its own; it only calls the modules above and renders what comes back. |

## Reading order

If you are new to this codebase, read in this order:

1. [`ir/`](ir/) — nothing else makes sense without the IR.
2. [`frontend/`](frontend/) — how a program becomes a `Circuit`.
3. [`pass/`](pass/) and [`analysis/`](analysis/) — what happens to a `Circuit` next, and how it's measured.
4. [`cli/`](cli/) — how all of the above is exposed to a user.

Each module has its own `mod.rs` doc comment with the specifics; this file
only orients you at the directory level. The normative specs live in
[`../docs/`](../docs/) — in particular `ir_spec.md`, `pass_manager.md`, and
`cli.md` — and are the source of truth when this code and a doc comment
disagree.

## A structural rule this crate enforces

Two invariants are enforced by module boundaries, not just by convention:

- **Nothing outside `ir/` constructs a `Circuit` by hand.** It is built only
  through `CircuitBuilder`, so every `Circuit` in existence has already
  passed validation. A pass that produces a bad rewrite gets an `IrError`
  back, not a corrupted circuit silently handed downstream.
- **Nothing outside `cli/pipeline.rs` calls the compiler from the CLI.**
  Every number the CLI prints was computed by `analysis` or `pass`, which is
  what keeps the tool's view of a circuit and the compiler's own view from
  ever disagreeing.

## What is not here yet

No target/basis profile, no qubit mapping or routing, no backend execution,
and no compiler orchestration layer — see
[`../docs/core_architecture/index.md`](../docs/core_architecture/index.md)
for the current baseline and what's next.
