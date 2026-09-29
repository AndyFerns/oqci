# Stage G — Benchmarking and Experimental Protocol

Status: PENDING — NOT LOCKED

This stage is intentionally not finalized. Nothing in it is a locked
requirement, and **no implementation agent may invent its numeric
parameters** (`final-deliverables-spec.md` §33, rule 15).

The benchmark infrastructure itself is specified in `final-deliverables-spec.md`
§20; this document defines the experimental protocol and may be revised after
additional research.

## 1. Priorities

*Revised 2026-09-30 per [`framing-and-research-question.md`](framing-and-research-question.md).*

The research question asks whether a strongly validated, quantum-first IR
supports deterministic transformations while staying interoperable with
LLVM/QIR. The **primary** experimental axes are therefore:

1. validation strength (§3.1);
2. determinism (§3.2);
3. interoperability (§3.3).

Compilation-quality and performance comparisons (§4) remain in scope as
secondary axes. Real-hardware experiments on IBM devices are **secondary**:
live IBM submission is outside the Definition of Done
(`final-deliverables-spec.md` §34), so hardware experiments happen only if the
required SDK, credentials and resource budget become available.

## 2. What this document will eventually define

- benchmark families and exact sizes;
- exact circuit counts;
- deterministic random seeds;
- timing methodology and repetitions;
- simulator/noise-model policy;
- baseline compiler configurations;
- statistical treatment;
- primary and secondary metrics (definitions in §3–§4; values pending);
- fair-comparison rules;
- raw-result retention;
- resource-budget constraints;
- final research reporting;
- *(secondary)* IBM backend-selection procedure, hardware shot counts and
  hardware circuit-selection rules.

## 3. Primary metric definitions

These definitions say **what** is measured and **how** it is computed. Every
threshold, count, size, seed and tolerance is **PENDING** and listed in §5.

### 3.1 Validation strength

How much of the compiler's transformation surface is covered by a checker, and
of what kind.

- **Checker coverage matrix.** For each transformation — each optimization
  pass, each lowering step (arity reduction, layout, routing, basis
  decomposition, orientation repair, single-qubit cleanup), parameter binding,
  and QIR emission — record which checker covers it:
  - *none*;
  - *structural*: I1–I8 re-validation, target legality;
  - *semantic, test-only*: a property test against an independent model
    (for example `tests/pass_equivalence.rs`, `tests/lowering_equivalence.rs`);
  - *semantic, in-compiler*: a validator that runs during compilation
    (none exists yet).

  The matrix is reported as-is, not collapsed into a score.
- **Mutation evidence.** For each semantic checker, a recorded list of
  deliberately injected defects and whether the checker caught each one. The
  docs already contain individual records of this kind (e.g. the rotation-merge
  sign inversion in `pass_manager.md`; the dropped `H` in
  `tests/lowering_equivalence.rs`). The mutant set is PENDING.
- **Inconclusive rate** (once an in-compiler validator exists): the fraction of
  checked transformations for which the validator reported "inconclusive"
  rather than a verdict, reported with the reasons.

### 3.2 Determinism

- **Predicate.** A compilation is deterministic for a given input and
  configuration if its *semantic* outputs — optimized and lowered circuits,
  executable, layouts, cost breakdown, QIR text — are byte-identical across
  repeated runs.
- **Partition.** Outputs are split into: semantic outputs (subject to the
  predicate); build identity (compiler version, git commit — equal only within
  one build); and measurements (all durations — never expected to be equal).
  Only the first partition is tested.
- **Scope.** Claimed only for the same build, and only across platforms that
  are in the CI matrix. No cross-version claim.
- Reported as pass/fail per input, per platform. The number of repetitions and
  the corpus are PENDING.

### 3.3 Interoperability

- **Acceptance.** For each external consumer (named tool, pinned version) and
  each emitted artifact: whether the consumer accepted it (parse, verify), and
  what that acceptance proves.
- **Execution agreement** (where the consumer executes): whether its output
  distribution agrees with a reference simulator run of the same artifact.
  The statistical test and tolerance are PENDING.
- Only consumers that actually ran are counted. A consumer that was not run is
  reported as *not run*, never as a pass.

## 4. Secondary metric definitions

The compiler-quality metrics already produced by the analysis module and the
cost model (`final-deliverables-spec.md` §12–§13): gate counts, two-qubit
count, depth, SWAP overhead, target-native counts and compilation time (with
queue time excluded). Baselines are §20.2 of the final spec. Raw metrics are
always retained alongside any derived score (Stage E §7).

## 5. Pending parameters

Every item below must be decided by research and recorded here before any
result is reported. None may be invented by an implementation agent.

| Parameter | Used by |
|---|---|
| Benchmark families, sizes and circuit counts | all |
| Random seeds and the seeding policy | all generated corpora |
| Mutant set for mutation evidence | §3.1 |
| Repetitions per input for the determinism predicate | §3.2 |
| Determinism corpus | §3.2 |
| External consumers and their pinned versions beyond those in CI | §3.3 |
| Statistical test and tolerance for execution agreement | §3.3 |
| Shot counts | §3.3, §4 |
| Timing methodology and repetitions | §4 |
| Baseline configurations | §4 |
| Noise-model policy | §4 |
| IBM backend selection, shots and circuit selection (secondary) | §1 |
