# `tests/` — Integration and Property Tests

Everything here exercises the `oqci` crate the way an external caller
would — through its public API or the compiled binary — rather than through
`#[cfg(test)] mod tests` inside the source files (those cover
implementation-internal cases and live next to the code they test).

## Files

| File | What it covers |
|---|---|
| [`pipeline.rs`](pipeline.rs) | The core `Circuit` → `QcoCircuit` → QIR pipeline: the required corpus (identity, Bell, GHZ-3, mid-circuit measurement), one malformed-circuit case per validation invariant, and Stage F parameter binding end to end. |
| [`error_model.rs`](error_model.rs) | Every `IrError` variant, exercised directly. |
| [`qco_structure.rs`](qco_structure.rs) | QCO-IR's dependency-graph shape: data vs. control edges, boundary nodes, deterministic traversal. |
| [`qir_emit.rs`](qir_emit.rs) | QIR text output: intrinsic mapping, angle encoding, declaration dedup. |
| [`roundtrip.rs`](roundtrip.rs) | QC-IR → QCO-IR → linearize reproduces the original program. |
| [`validation.rs`](validation.rs) | `CircuitBuilder::build`'s invariant checks, one test per invariant. |
| [`openqasm_pipeline.rs`](openqasm_pipeline.rs) | The OpenQASM 3 frontend end to end: the required corpus, parameterized circuits, and one rejection test per construct outside the documented subset. Parsed circuits are asserted **byte-identical in emitted QIR** to the equivalent hand-built circuit — the test that would fail if the frontend ever started reshaping semantics instead of just mapping them. |
| [`qiskit_adapter.rs`](qiskit_adapter.rs) | The Qiskit adapter's pure-Rust translation core (`QiskitCircuitIr` → `Circuit`), with no Python interpreter involved — see [`../python/README.md`](../python/README.md) for the PyO3-boundary tests that do need one. |
| [`pass_equivalence.rs`](pass_equivalence.rs) | **The test that makes the optimization passes trustworthy.** Runs `proptest`-generated circuits through each pass and asserts the resulting state vector is unchanged up to global phase, using an independent state-vector simulator (see `support/`) rather than the tables the passes themselves consult. See [`../docs/pass_manager.md`](../docs/pass_manager.md#correctness-verification). |
| [`pass_equivalence.proptest-regressions`](pass_equivalence.proptest-regressions) | Saved failing-case seeds from `pass_equivalence.rs`, checked in per `proptest` convention so everyone who runs the suite benefits from cases it has found before. Read the file's header comment for the provenance of the current entries. |
| [`cli.rs`](cli.rs) | The compiled `oqci` binary, invoked as a subprocess against the fixtures in [`../examples/`](../examples/) — the parts unit tests can't reach: argument parsing, file I/O, exit codes, and `--json` output actually parsing as JSON. |

## `support/`

[`support/`](support/) holds test-only helpers, currently a single dense
state-vector simulator (`support/statevector.rs`) used exclusively by
`pass_equivalence.rs`. It is **not** a backend and is not built as part of
the library — see its module doc comment for why an independent
implementation is the point, and why this isn't the "custom simulator" the
project's non-goals rule out.

## Conventions

- Prefer a small, hand-built `Circuit`/`CircuitBuilder` fixture over a large
  one; most tests build exactly the shape needed to exercise one rule.
- When a rewrite or transformation is involved, prefer asserting
  **equivalence** to an independently-constructed expected circuit (or QIR
  string) over asserting properties of the output in isolation — that's what
  catches a subtly-wrong implementation rather than one that merely looks
  plausible.
- Malformed-input tests are paired with the exact invariant/error variant
  they exercise, one test per rule, so a coverage gap is visible by
  inspection.
