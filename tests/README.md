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
| [`cudaq_frontend.rs`](cudaq_frontend.rs) | The CUDA-Q frontend end to end, asserted by equivalence: a kernel must emit byte-identical QIR to the same circuit written in OpenQASM 3. |
| [`target_model.rs`](target_model.rs) | The target model's seams: profiles, legality checking and cost, exercised the way a caller would. |
| [`decomposition.rs`](decomposition.rs) | Every decomposition rule checked as a **matrix** against the independent simulator in `support/`, plus the rule-set closure checks each profile relies on. |
| [`lowering.rs`](lowering.rs) | Target lowering's behaviour: each refusal fires for its own reason, reported metadata means what it says, orientation policies are the identities they claim to be. |
| [`lowering_equivalence.rs`](lowering_equivalence.rs) | **The central lowering property.** Generated circuits on generated devices either refuse for an independently confirmed reason, or lower to a circuit that is equivalent modulo the final layout, legal, and byte-identical across runs. |
| [`qir_conformance.rs`](qir_conformance.rs) | QIR 1.0 Base Profile structure of every emitted module (every example × every backend × both pointer styles). OQCI's own reading of the spec, so *not independent evidence*; it also writes the corpus CI feeds to PyQIR and `llvm-as`. |
| [`target_file.rs`](target_file.rs) | Compiling for a target supplied at runtime (§9.2): loading, kind and id overrides, refusals, and a CZ-native fixture compiled, verified and recorded in provenance. |
| [`extensions.rs`](extensions.rs) | The extension registry: an empty registry is the built-in compiler, placement, the refusals, extension frontends, and per-stage timings. |
| [`verifier.rs`](verifier.rs) | The in-compiler semantic checker: property agreement with the test oracle, a deliberately wrong extension pass that must stop compilation, every example verified on every backend. |
| [`lint.rs`](lint.rs) | One firing and one non-firing case per lint rule, deterministic ordering, an extension rule. |
| [`determinism.rs`](determinism.rs) | The determinism contract across separate `oqci` processes, with measurements stripped ([`../docs/determinism.md`](../docs/determinism.md)). |
| [`limits.rs`](limits.rs) | Untrusted register widths refused before allocation, at every boundary (found by fuzzing). |
| [`cli.rs`](cli.rs) | The compiled `oqci` binary, invoked as a subprocess against the fixtures in [`../examples/`](../examples/) — the parts unit tests can't reach: argument parsing, file I/O, exit codes, and `--json` output actually parsing as JSON. |

## `support/`

[`support/`](support/) holds test-only helpers, currently a single dense
state-vector simulator (`support/statevector.rs`) used by the equivalence
and decomposition tests. It is **not** a backend and is not built as part of
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

## Elsewhere

- [`../examples/plugin-demo/tests/no_core_modification.rs`](../examples/plugin-demo/tests/no_core_modification.rs) — the extension demonstration, end to end.
- [`../fuzz/`](../fuzz/) — cargo-fuzz targets; `fuzz/tests/corpus.rs` runs their seed corpus on any toolchain.
- [`../python/tests/`](../python/tests/) and [`../benchmarks/tests/`](../benchmarks/tests/) — the SDK, adapters and benchmark harness.
