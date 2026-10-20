# Benchmarks and the research package

[`benchmarks/`](../benchmarks/) implements the benchmark infrastructure of
`final-deliverables-spec.md` §20 under the experimental protocol locked as
Stage G ([`stage-g-benchmarking-protocol.md`](core_architecture/stage-g-benchmarking-protocol.md)).
It also covers the plots of §21, the reproducibility record of §22, the timing
of §24 and the research package of §30.

Every numeric parameter comes from a config file:

- [`benchmarks/configs/stage_g.toml`](../benchmarks/configs/stage_g.toml)
  mirrors the locked protocol;
- `smoke.toml` is a small CI profile whose numbers are never reported.

The harness code contains no experimental values of its own.

## Running it

From `benchmarks/`, with OQCI installed (`maturin develop --release
--manifest-path python/Cargo.toml`) and `pip install -r requirements.txt`:

```bash
python -m oqci_bench all --config configs/stage_g.toml --out results/<run-id>
python -m oqci_bench report results/<run-id>          # rebuild tables and figures
pytest tests -q                                        # the harness's own tests
```

`all` runs the following steps, each also available on its own:

| Step | Writes | What it is |
|---|---|---|
| `run` | `records.jsonl`, `artifacts/` | every item × target × compiler configuration: metrics, timing, correctness, the compiled circuit (OpenQASM 3) and OQCI's canonical QIR |
| `determinism` | `determinism.json` | Stage G §3.2: the corpus compiled in ten fresh processes, compared byte for byte after removing measurements |
| `interop` | `interop.json`, `interop/qir/` | Stage G §3.3: QIR acceptance by PyQIR, `llvm-as` 18 and 14 (`not run` when absent), and execution agreement of Aer and CUDA-Q with the exact distribution |
| `coverage` | `coverage.json` | the checker coverage matrix, validated against the code |
| `mutants` | `mutants.json` | Stage G §3.1 mutation evidence |
| `report` | `report/` | CSV, Markdown and LaTeX tables and PNG figures, built only from the files above |

`environment.json` records:

- the repository commit, and whether the tree was dirty;
- the OQCI version, and the commit its binary was built from;
- Python, platform and CPU;
- `rustc` and every relevant package version;
- the thread-count environment variables.

## The corpus, targets and compilers

There are 64 circuits, all ending in a full measurement:

- QFT at 8 sizes;
- Grover at 4 sizes, with a clean-ancilla Toffoli ladder;
- a hardware-efficient VQE ansatz at 4 sizes × 3 depths;
- QASMBench-style random circuits at 4 sizes × 10 instances.

They are generated in `oqci_bench/circuits.py` using **only gates in OQCI's
registered set**, so every compiler receives the identical `QuantumCircuit`.
Seeds follow the locked SHA-256 policy in `oqci_bench/seeds.py`.

There are three targets, each defined once and rendered for every compiler
(`oqci_bench/targets.py`):

- all-to-all and linear, both on `{rz, sx, x, cx}`;
- the coupling map of the Heron fake backend `FakeFez`, on `{rz, sx, x, cz}`.

Every compiler gets the same constraints, connectivity and basis, and no
calibration data.

| Configuration | What it is |
|---|---|
| `oqci-default` | OQCI through its Python SDK, dense layout, default pipeline |
| `qiskit-o3`, `qiskit-o0` | `transpile` with the coupling map and basis, `seed_transpiler` = the item seed |
| `tket` | `FullPeepholeOptimise(allow_swaps=False)` → `DefaultMappingPass` → `AutoRebase` → `RemoveRedundancies`. If the first pass still leaves an implicit qubit permutation, it is rerun without the final measurements and the permutation made explicit as SWAPs; the record's `compiler_notes` says so |
| `oqci-no-passes`, `oqci-no-<pass>`, `oqci-trivial-layout` | the ablation |

## Correctness of every output

Every compiled output is checked against its source, with layouts applied
(`oqci_bench/equivalence.py`). Gate semantics come from Qiskit's matrices, not
OQCI's.

- **For n ≤ 10:** an exhaustive operator check over every basis input. It
  passes at one of two strengths:
  - `operator`: equal up to one global phase;
  - `operator-mod-measured-phase`: `U_compiled = D·U_source` with `D`
    diagonal. When every qubit is measured at the end, this is exactly "the
    same outcome distribution for every input state". It is the freedom
    optimizers use when they drop diagonal gates before measurement (Qiskit
    O3's `RemoveDiagonalGatesBeforeMeasure`, for one).
- **For n ≤ 20:** the same comparison from `|0…0⟩`.
- **Measurements:** each classical bit must be read from the physical qubit
  where its logical qubit ends.
- **Wider circuits** are recorded as `not-checked`, never as `pass`.
- **A `fail` aborts the run.**

The harness's tests feed the checker deliberately broken outputs and require
a `fail` for each:

- a dropped gate;
- a forged layout;
- a mid-circuit relative-phase error;
- swapped classical bits.

## Metrics and statistics

Metrics (`oqci_bench/metrics.py`) are computed by one function for every
compiler:

- gate counts (total, one-qubit, two-qubit, native) and depth, excluding
  measurement;
- routing overhead, derived uniformly as the two-qubit count on a constrained
  target minus the count on the all-to-all target, for the same circuit and
  configuration.

Timing is `perf_counter_ns` around the compile call: one warm-up, five timed
repetitions, reported as median and IQR. OQCI's own per-stage `Timings` are
recorded alongside.

The report (`oqci_bench/report.py`) gives:

- per-record and per-family tables;
- paired ratios, primary over baseline: a geometric mean with a 95%
  bootstrap CI (10,000 resamples) and a two-sided Wilcoxon signed-rank
  p-value;
- the ablation deltas;
- pass-by-pass effect;
- stage timings;
- the determinism, interop, mutation, coverage and hardware tables.

It states what was measured and nothing more (§30.1).

## Mutation evidence

[`benchmarks/mutants/mutants.toml`](../benchmarks/mutants/mutants.toml) defines
ten injected defects, the locked Stage G set. Each is an exact text
replacement, applied to a throwaway `git worktree` of `HEAD`:

- rotation-merge sign flip;
- cancelling `S;S`;
- cancelling across a measurement;
- canonicalize reordering `H;X`;
- a wrong Euler angle in fusion;
- `h-to-rz-sx` dropping a step;
- `rx-to-rz-sx` with a wrong angle;
- `swap-to-cx` with one orientation;
- routing forging its final layout;
- QIR swapping CX operands.

Six checkers run against each mutant:

- `pass-equivalence`, `lowering-equivalence` and `decomposition` (semantic,
  test-only);
- the in-compiler verifier, run on the probe programs in `mutants/probes/`
  with `--verify`;
- `qir-structure` (structural);
- the unit tests (example-based).

The unmutated baseline must pass every checker, or the evidence is void. A
mutant that no checker catches is reported as **surviving**, not hidden.

## The reference run

The committed reference run is `benchmarks/results/20261020-stage-g/`, built
from `d42b413` at `0.7.1`. A first attempt, built at `0.6.0`, stopped at
`random_4_i6` on the t\|ket⟩ permutation that the fallback now handles. It was
dropped rather than mixed with a second build. The mutation step stopped on a
stale mutant and was rerun alone at `80827f9`, whose compiler source is
identical to `d42b413`'s. `mutants.json` records that commit.

It holds:

- the records, the configuration and the environment;
- the determinism, interop, coverage and mutation results;
- the full report.

The compiled-circuit artifacts and the interop working files are regenerated
by rerunning, and are not committed. Compile times in this run are noisier
than the protocol intends: the interquartile range exceeds a quarter of the
median for 163 of 768 timed records, which suggests other load on the machine.
The quality metrics do not depend on timing. Its findings, as measured, are summarized in that run's
`report/README.md`.

## Hardware

The IBM experiment is operator-run: see [`hardware.md`](hardware.md). Its
results join the report as a further table.
