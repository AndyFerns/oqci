# Fuzzing

`fuzz/` holds OQCI's fuzzing (`final-deliverables-spec.md` §23.3). Its job is to
show that untrusted input can only produce a circuit or a typed error. It must
never cause a panic, an abort, or a silent miscompilation. The crate is
separate from the main workspace, because libFuzzer needs a nightly toolchain
and sanitizers.

## Targets

The target bodies are plain functions in [`fuzz/src/lib.rs`](../fuzz/src/lib.rs).
Each file in `fuzz/fuzz_targets/` wraps one of them for libFuzzer, so every
fuzzing method below drives the same code.

| Target | Input | Property |
|---|---|---|
| `openqasm` | any bytes | The OpenQASM 3 frontend returns a circuit, which has passed I1–I8 by construction, or a typed error. |
| `cudaq` | any bytes | The same, for the CUDA-Q frontend. |
| `target_json` | any bytes | A target description from outside the process (§29) is refused with a typed error, or loaded. A loaded target must then compile a small circuit without panicking. |
| `qiskit_handoff` | bytes decoded into a handoff struct | The Qiskit translation core accepts any struct the PyO3 layer could build, with arbitrary names, operands and parameters, without panicking. |
| `compile_verified` | any bytes | **Semantic.** A program the OpenQASM frontend accepts is compiled for three backends with `--verify` on. A `SemanticMismatch` is a miscompilation, and the target panics so the input is kept. |

## Three ways to run them

**The seed corpus regression test, on any toolchain.**
[`fuzz/tests/corpus.rs`](../fuzz/tests/corpus.rs) runs every seed in
`fuzz/corpus/` through every target. It also runs a few deliberately hostile
inputs: empty, binary, truncated, deeply nested, and huge widths. This is not
fuzzing. It is the floor that every change must keep.

```bash
cd fuzz && cargo test --no-default-features
```

**The mutation fuzzer, on any toolchain, Windows included.**
[`fuzz/tests/mutate.rs`](../fuzz/tests/mutate.rs) mutates the seeds by byte
flips, inserted syntax characters, deletions, duplications and splices. Its
seeds come from the corpus, the target fixtures and `examples/bell_cudaq.py`.
It feeds each mutant to the targets and catches panics. The random generator
has a fixed seed, so any failure reproduces exactly. The test is ignored by
default:

```bash
cd fuzz && OQCI_MUTATION_SCALE=10 cargo test --no-default-features --release --test mutate -- --ignored --nocapture
```

| Variable | Effect |
|---|---|
| `OQCI_MUTATION_SCALE` | Multiplies every budget. The defaults are 200,000 mutants for `openqasm`, `cudaq` and `qiskit_handoff`, and 20,000 for `target_json` and `compile_verified`. |
| `OQCI_MUTATION_ONLY=<target>` | Runs one target. |
| `OQCI_MUTATION_TRACE=1` | Writes each input to `fuzz/last-input` before running it. An abort, such as a failed allocation, cannot be caught, but this keeps the input that caused it on disk. |

A panicking input is written to `fuzz/crash-<target>-<n>`. Both `crash-*` and
`last-input` are git-ignored. The fuzzer is not coverage-guided, but it finds
shallow crashes anywhere. All the 0.7.1 findings below came from it.

**libFuzzer, coverage-guided, Linux and nightly only.**

```bash
cargo +nightly fuzz run --fuzz-dir fuzz --target x86_64-unknown-linux-gnu openqasm fuzz/corpus/openqasm -- -max_total_time=60
```

Always pass `--target`. A prebuilt `cargo-fuzz`, such as the one CI installs,
is a static musl binary, and by default it builds for its own triple.
AddressSanitizer cannot instrument that target. Coverage-guided fuzzing does
not run on Windows. With the GNU toolchain there is no libFuzzer runtime, and
MSVC is not set up here.

## In CI

The `fuzz` job in `.github/workflows/ci.yml` runs once per target, on Ubuntu.
Its steps are:

1. the corpus regression test;
2. the mutation fuzzer, at scale 1, for that target;
3. 60 seconds of libFuzzer from the committed corpus.

Any crash fails the job. So does any miscompilation that `compile_verified`
finds. A failing step publishes the tail of its output as a public
annotation (`scripts/ci_annotate.sh`), and libFuzzer's crashing inputs are
uploaded as the `fuzz-artifacts-<target>` artifact.

## What it has found

The 0.7.1 run found five defects, each now fixed and covered by
[`tests/limits.rs`](../tests/limits.rs):

- a rotation-merge miscompilation for huge angles;
- matrices built from platform-dependent trigonometry;
- parser stack overflows on deep nesting;
- a CUDA-Q register allocated in full before any size check;
- an overflow in the running qubit count.

The bounds that resulted are tabulated in
[`verification.md`](verification.md#numerical-and-structural-limits). An input
that reproduces a fixed defect belongs in the seed corpus as a `.qasm` file.
For example, `fuzz/corpus/compile_verified/huge_rotation_merge.qasm`.
