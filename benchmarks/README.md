# `benchmarks/` — Reserved for the Benchmark Suite (Not Yet Populated)

This directory is currently empty (a `.gitkeep` only). It exists as the
reserved location for OQCI's benchmark infrastructure, which has **not been
implemented yet** — this is a placeholder, not a stub of working code.

Per the project's own anti-hallucination rule
([`../docs/core_architecture/final-deliverables-spec.md`](../docs/core_architecture/final-deliverables-spec.md)
§33 and the "Critical Rule" in
[`../docs/core_architecture/index.md`](../docs/core_architecture/index.md)):
a directory existing is not evidence a feature is implemented. Nothing in
this file should be read as a benchmark result or a working harness.

## What's planned here

Per `final-deliverables-spec.md` §20 (Benchmark Infrastructure Deliverable),
once built this directory is expected to hold:

- benchmark circuit loading/generation (QFT, Grover, VQE/variational
  circuits, random/QASMBench-style circuits — §20.1);
- deterministic, reproducible metadata for each run (§20.3's result schema:
  circuit identity, compiler version, git commit, pass pipeline, target
  profile, timing, metrics, artifact paths);
- a compiler runner and a backend runner abstraction;
- baseline runners (e.g. the Qiskit transpiler, t|ket⟩, and OQCI with
  selected passes disabled for ablation — §20.2);
- result serialization, report generation, and plotting/data-export
  utilities (§21).

## What's explicitly deferred

The exact experimental protocol — benchmark sizes, circuit counts, random
seeds, repetition counts, hardware-selection rules, and statistical
treatment — is **Stage G**
([`../docs/core_architecture/stage-g-benchmarking-protocol.md`](../docs/core_architecture/stage-g-benchmarking-protocol.md)),
which is deliberately **not locked**. The benchmark *software* may be built
before Stage G is finalized, but no implementation should invent Stage G's
numeric parameters in the meantime.

Building this out depends on work that doesn't exist yet either: a target
model, backend execution, and the compiler orchestration layer (see
[`../docs/core_architecture/index.md`](../docs/core_architecture/index.md)
for the current baseline).
