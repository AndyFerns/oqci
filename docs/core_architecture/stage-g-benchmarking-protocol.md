# Stage G — Benchmarking and Experimental Protocol

Status: **LOCKED — 2026-10-20**

The numeric parameters in §5 were proposed in the 0.6.0 completion plan and
locked by the project owner's approval of that plan on 2026-10-20. Until then
this stage was pending, and no implementation agent could invent its values
(`final-deliverables-spec.md` §33, rule 15). They are now decided, by the
owner, and may be changed only by amending this document.

They are mirrored as data in
[`benchmarks/configs/stage_g.toml`](../../benchmarks/configs/stage_g.toml).
This document is normative: if the two ever disagree, this document wins and
the config file is the bug.

The benchmark infrastructure itself is specified in `final-deliverables-spec.md`
§20 and documented in [`../benchmarks.md`](../benchmarks.md).

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
(`final-deliverables-spec.md` §34). The submission path exists since `0.6.0`
(`oqci.backends.ibm`, [`../hardware.md`](../hardware.md)), and the hardware
experiment is run by its operator with their own account and budget.

## 2. What this document defines

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
    (`crate::verify`, `--verify`, since `0.6.0`; see
    [`../verification.md`](../verification.md)).

  The matrix is reported as-is, not collapsed into a score.
- **Mutation evidence.** For each semantic checker, a recorded list of
  deliberately injected defects and whether the checker caught each one. The
  docs already contain individual records of this kind (e.g. the rotation-merge
  sign inversion in `pass_manager.md`; the dropped `H` in
  `tests/lowering_equivalence.rs`). The mutant set is PENDING.
- **Inconclusive rate**: the fraction of checked transformations for which
  the in-compiler validator reported "inconclusive" rather than a verdict,
  reported with the reasons.

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

## 5. Locked parameters

Locked 2026-10-20 by the project owner. Every value below is used by
`benchmarks/configs/stage_g.toml` and nowhere invented in code.

| Parameter | Value | Used by |
|---|---|---|
| Benchmark families and sizes | QFT n ∈ {4, 6, 8, 10, 12, 14, 16, 20}. Grover on data n ∈ {3, 4, 5, 6}: all-ones mark, ⌊π/4·√2ⁿ⌋ iterations, CCX-ladder ancillas. VQE hardware-efficient ansatz (RY+RZ layers, linear CX) n ∈ {4, 8, 12, 16} × reps ∈ {1, 2, 3}. Random n ∈ {4, 8, 12, 16}: 10·n gates, two-qubit fraction 0.3, gate pool {h, x, y, z, s, sdg, t, tdg, sx, rx, ry, rz, cx, cz, swap}, 10 instances per size. All end in full measurement: **64 circuits**. | all |
| Targets | T1 all-to-all-n and T2 linear-n, both `{rz, sx, x, cx}`. T3: the coupling map of the Heron-family fake backend `FakeFez` shipped with the pinned `qiskit-ibm-runtime`, `{rz, sx, x, cz}`. Every compiler gets connectivity and basis only, no calibration data. | all |
| Random seeds and seeding policy | Master seed `20261020`. Per-item seed = the first 8 bytes, big-endian, of SHA-256(`master\|family\|n\|reps\|instance`). VQE angles drawn uniformly from [0, 2π) with it. Qiskit's `seed_transpiler` is the item seed. Nothing else is random. | all generated corpora |
| Baseline configurations | Qiskit 2.5.x `transpile` at optimization level 3, and level 0 as a control. t\|ket⟩ with the fixed recipe `FullPeepholeOptimise(allow_swaps=False)` → `DefaultMappingPass` → `AutoRebase` to the target basis → `RemoveRedundancies`. OQCI default: dense layout, default pipeline. | §4 |
| Ablation | OQCI with all passes, no passes, each pass disabled in turn, and trivial versus dense layout, on T2 and T3. | §4 |
| Timing methodology and repetitions | `perf_counter_ns` around each compiler's compile call, plus OQCI's internal per-stage timings. One warm-up discarded, five timed repetitions, median and IQR reported. Main configurations only; ablations compile once. Circuit generation and I/O excluded. Machine and thread settings recorded. | §4 |
| Statistical treatment | Quality metrics are deterministic per item; random-family results are aggregated as the median with min and max. Paired OQCI/baseline ratios: geometric mean with a 95% bootstrap CI (10,000 resamples, seeded) and a two-sided Wilcoxon signed-rank test. Reported only, with no claims beyond. | §4 |
| Correctness of compiled outputs | Every output is checked: operator check for n ≤ 10 (equal up to global phase, or up to the diagonal phase terminal measurement leaves unobservable), and the same comparison from \|0⟩ for n ≤ 20. A failure aborts the run. | all |
| Determinism corpus and repetitions | All 64 circuits × {T2, T3} × OQCI default, plus every example program. Ten repetitions, each in a fresh process. Platforms: ubuntu and windows, in CI. | §3.2 |
| External consumers and versions | `llvm-as` 18 (opaque and typed pointers), `llvm-as` 14 (typed), and PyQIR 0.12.5, on the canonical QIR of the whole corpus. Execution agreement: Aer as reference, and CUDA-Q (pinned 0.16.0.post1) on n ≤ 12. | §3.3 |
| Statistical test and tolerance for execution agreement | 8192 shots, seeded. Chi-square goodness of fit of each consumer's counts against the exact source probabilities, with bins whose expected count is below 5 merged. Reject at p < 0.001. TVD reported alongside. | §3.3 |
| Mutant set | Ten defects: rotation-merge sign flip; cancellation of a non-inverse pair (S;S); cancellation across a measurement; canonicalize reordering non-commuting gates (H;X); fusion with a wrong Euler angle; `h-to-rz-sx` dropping a step; `rx-to-rz-sx` with a wrong angle; `swap-to-cx` with one orientation; routing reporting a forged final layout; QIR swapping CX operands. See `benchmarks/mutants/mutants.toml`. | §3.1 |
| Noise-model policy | Correctness and agreement runs are noiseless. A noisy run uses `FakeFez`'s noise model (IBM local testing mode) for the hardware subset only, as secondary data. | §4 |
| Shot counts | 8192 for execution agreement; 4096 per circuit on hardware. | §3.3, §1 |
| IBM backend selection, shots and circuit selection (secondary) | The operator's choice of Heron backend, by default the least busy operational CZ-native one; its name and a target snapshot are stored. Circuits: Bell(2), GHZ(3), GHZ(5), Grover(3), VQE(4, reps 1), random(4, instance 0), and QFT(4) followed by its inverse on \|0101⟩. Compiled by OQCI and by Qiskit O3, 14 circuits in one Sampler job, 4096 shots each. Metric: Hellinger fidelity to the exact distribution, with no pass/fail threshold. | §1 |
| Raw-result retention and resource budget | All raw JSONL records, configs, the environment record, compiled circuits and QIR are kept with each run (the compiled-circuit artifacts are regenerable and not committed). The full local run should take ≤ ~2 h on a laptop; CI runs the smoke profile. Hardware use ≤ 10 QPU-minutes. | all |
