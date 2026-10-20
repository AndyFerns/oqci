# The determinism contract

The research question asks whether a strongly validated IR supports
**deterministic** transformations
([`core_architecture/framing-and-research-question.md`](core_architecture/framing-and-research-question.md)).
This page defines what OQCI claims, how the claim is checked, and its limits.
The metric definition is Stage G §3.2.

## The predicate

A compilation is **deterministic** when, for the same input and configuration,
its *semantic outputs* are byte-identical across repeated runs. The semantic
outputs are:

- the optimized circuit;
- the lowered circuit;
- the executable;
- the initial and final layouts;
- the cost breakdown;
- the pass records apart from their durations;
- the canonical QIR text.

## Three kinds of output

| Kind | Examples | Expected to repeat? |
|---|---|---|
| Semantic outputs | the list above | **yes, byte for byte** |
| Build identity | compiler version, git commit | only within one build |
| Measurements | `timings`, any `*_duration_us` / `*_duration_ms` | never |

`oqci::cli::snapshot::strip_measurements` is the single definition of what a
comparison removes: every `timings` key, and every key ending in `duration_us`
or `duration_ms`, at any depth. The Python harness mirrors it exactly
(`benchmarks/oqci_bench/determinism.py`), and a test pins the two to the same
behaviour.

## Scope

**Claimed:**

- for the same build;
- on each platform in the CI matrix (ubuntu, windows);
- across separate processes.

**Not claimed:**

- **Across versions.**
- **Across platforms.** Floating-point library functions (`sin`, `atan2`) may
  differ in the last bit between platforms. That could change, for example,
  the angles single-qubit fusion synthesizes.

### Why processes, not just repetitions

Rust seeds every `HashMap` randomly per process. An output that leaked
hash-iteration order would pass any number of in-process repetitions and
change from one run to the next. Every determinism check here compares
separate processes.

## How it is checked

- **`tests/determinism.rs`** (every `cargo test`, ubuntu and windows) checks
  every example, for three backends and a loaded target file. Each one is
  compiled in separate `oqci` processes, and the stripped JSON reports must be
  byte-identical.
- **`python -m oqci_bench determinism`** (the `determinism` CI job, ubuntu
  and windows) applies the Stage G protocol:
  - the full 64-circuit corpus on the linear and Heron targets;
  - ten repetitions, each a fresh Python process compiling the whole corpus;
  - pass/fail per item, recorded in `determinism.json`.
- **`tests/lowering_equivalence.rs`** additionally asserts that lowering is
  byte-identical across repeated runs on every generated case.

## What makes it hold

Every ordering in the compiler breaks ties on a stable key:

- layout breaks ties on the lower physical index;
- routing chooses the lexicographically smallest shortest path;
- lint output is sorted;
- provenance lists are ordered.

Maps that reach output are `BTreeMap`s. Nothing reads the clock, the
environment or a random source on a semantic path. The only nondeterministic
inputs are timings, and they are measurements.
