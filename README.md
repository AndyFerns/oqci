<div align="center">

# <svg width="24" height="24" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M21 16.5V5.75A2.755 2.755 0 0 0 18.25 3H5.75A2.755 2.755 0 0 0 3 5.75V16.5H2v1.75A2.755 2.755 0 0 0 4.75 21h14.5A2.755 2.755 0 0 0 22 18.25V16.5h-1ZM4.5 5.75c0-.69.56-1.25 1.25-1.25h12.5c.69 0 1.25.56 1.25 1.25V16.5h-15V5.75Zm16 12.5c0 .69-.56 1.25-1.25 1.25H4.75c-.69 0-1.25-.56-1.25-1.25V18h17v.25Z" fill="#ffffff"/></svg> oqci <svg width="24" height="24" fill="none" xmlns="http://www.w3.org/2000/svg"><path fill-rule="evenodd" clip-rule="evenodd" d="M19.58 11.485a7.99 7.99 0 0 1-.425.515v.01l.128.154c.102.121.202.241.297.361 1.36 1.74 1.745 3.335 1.08 4.485-.375.645-1.225 1.4-3.175 1.4H17.3l.035-1.5c1.04.03 1.775-.21 2.025-.65.33-.57-.03-1.62-.96-2.815a4.684 4.684 0 0 0-.197-.24l-.098-.115c-.53.49-1.115.97-1.745 1.435-.495 4.49-2.225 7.485-4.36 7.485-1.615 0-2.82-1.58-3.575-3.81-.685.135-1.34.215-1.94.215-1.43 0-2.575-.42-3.145-1.405-.385-.665-.615-1.835.465-3.61l1.28.78c-.54.885-.7 1.64-.45 2.08.34.59 1.585.825 3.38.495-.165-.71-.29-1.46-.375-2.225C4 11.855 2.27 8.86 3.34 7.01c.81-1.4 2.78-1.65 5.085-1.19.755-2.23 1.96-3.81 3.575-3.81.775 0 1.9.385 2.9 2.22l-1.32.715c-.495-.915-1.07-1.44-1.58-1.44-.68 0-1.505.96-2.12 2.68.7.215 1.41.475 2.115.785C16.13 5.15 19.59 5.15 20.66 7c.66 1.15.28 2.74-1.08 4.485Zm-1.18-.92c.93-1.195 1.29-2.245.96-2.815-.475-.82-2.645-.93-5.555.11a21.124 21.124 0 0 1 4.3 3.06l.098-.116c.069-.08.136-.158.197-.239ZM8.02 7.255c-1.795-.33-3.04-.095-3.38.495-.475.82.51 2.755 2.865 4.755C7.5 12.335 7.5 12.17 7.5 12c0-1.615.18-3.265.52-4.745Zm1.445.355C9.18 8.84 9 10.315 9 12c0 1.685.18 3.16.465 4.39 1.205-.365 2.575-.945 4.035-1.79 1.36-.785 2.575-1.68 3.565-2.6-.99-.925-2.205-1.815-3.565-2.6-1.46-.84-2.825-1.42-4.035-1.79Zm.415 10.205c.61 1.72 1.44 2.685 2.12 2.685v-.005c.95 0 2.13-1.825 2.685-4.865l-.094.059c-.112.07-.224.14-.341.206-1.4.81-2.915 1.475-4.37 1.92ZM13.5 12a1.5 1.5 0 1 1-3 0 1.5 1.5 0 0 1 3 0Z" fill="#ffffff"/></svg>

**Open Quantum Compiler Infrastructure**

Quantum-native compiler infrastructure first, LLVM/QIR interoperability second.

[Docs](docs/README.md) · [CLI guide](docs/cli.md) · [Pass manager](docs/pass_manager.md) · [Changelog](CHANGELOG.md)

</div>

## What is this?

OQCI builds on the broader LLVM/QIR quantum compilation ecosystem but draws a different architectural boundary. Its core is a quantum-native, strongly validated intermediate representation written in Rust; every transformation — optimization passes, target lowering, output — works on that IR. QIR is where OQCI meets the LLVM world: an interoperability artifact emitted at the end, not the representation the compiler thinks in.

The research question behind it:

> Can a quantum-first, strongly validated intermediate representation provide a clean foundation for deterministic quantum compiler transformations while remaining interoperable with established LLVM/QIR infrastructure?

See [`docs/core_architecture/framing-and-research-question.md`](docs/core_architecture/framing-and-research-question.md) for what that does and does not claim.

Concretely, that means:

- **Frontends** translate circuits from a source language/SDK into OQCI's IR, and nothing else. A frontend never gets to redefine what the IR means.
- **QC-IR / QCO-IR** are the two IR levels — an imperative instruction list and a dependency-graph form for optimization. Every circuit is validated at construction against invariants I1-I8, and the constraints that keep that true are an explicit contract ([`docs/architecture_decision_ir_contract.md`](docs/architecture_decision_ir_contract.md)).
- **A pass manager** runs explicit, ordered, individually-toggleable optimization passes over the IR.
- **Target lowering** maps, routes and decomposes a circuit onto a specific device's basis and topology, and re-checks its own output.
- **QIR emission** produces LLVM-compatible textual QIR as an interoperability artifact. Execution does not go through it: backends consume their own executable representation.

It's Rust-native at its core (fast, and the invariants are enforced by the type system rather than by convention), with Python bindings for the parts that need to talk to Qiskit. Vendor neutrality is a design property — no SDK type reaches the IR and no vendor branch appears in the compiler core — rather than the headline.

## Project status

**Current version: `0.6.0`.** This is early-stage, actively-developed infrastructure - treat anything not checked off below as *not there yet*, regardless of what a directory name or diagram might suggest.

| Layer | Status | Notes |
|---|---|---|
| QC-IR / QCO-IR core | ✅ Implemented | Validated construction (I1-I8), deterministic QC-IR → QCO-IR conversion. Untrusted register widths are refused before allocation. See [`docs/ir_spec.md`](docs/ir_spec.md). |
| QIR emission | ✅ Implemented (interop artifact) | One canonical QIR 1.0 module per compilation, emitted from the most-lowered circuit. It is labeled `base_profile` only when legal against the `qir-base` profile, `custom` otherwise. Accepted by PyQIR 0.12.5, `llvm-as` 18 and `llvm-as` 14, observed in CI. Acceptance proves well-formed IR and readable metadata, not execution. See [`docs/qir_lowering.md`](docs/qir_lowering.md). |
| Symbolic parameters | ✅ Implemented | Concrete or symbolic angles (`Rz(theta)`), with explicit binding before lowering. |
| OpenQASM 3 frontend | ✅ Implemented | A documented, intentionally partial subset. See [`docs/openqasm_frontend.md`](docs/openqasm_frontend.md). |
| Qiskit frontend | ✅ Implemented | A pure-Rust translation core plus a PyO3 boundary. A `QuantumCircuit` reaches the whole pipeline through `oqci.compile`. See [`docs/qiskit_adapter.md`](docs/qiskit_adapter.md). |
| CUDA-Q frontend | ✅ Implemented (documented subset) | The `@cudaq.kernel` function in a `.py` file. See [`docs/cudaq_frontend.md`](docs/cudaq_frontend.md). |
| Cirq frontend | ⛔ Out of scope | Out of scope by the project owner's decision for 0.6.0. The spec reserves it "to the committed scope", and that scope is none. |
| Pass manager | ✅ Implemented | Explicit ordering, enable/disable per pass, deterministic execution, ablation, and extension passes. See [`docs/pass_manager.md`](docs/pass_manager.md). |
| Optimization passes | ✅ Implemented | Canonicalization, gate cancellation, rotation merging, single-qubit gate fusion (target-aware: it only fuses when the result is cheaper on the target), and scheduling as analysis. All are property-tested against an independent state-vector model. |
| Semantic verification | ✅ Implemented | `--verify` checks binding, every pass and lowering during compilation, and refuses to produce an executable if one changed what the circuit computes. See [`docs/verification.md`](docs/verification.md). |
| Lint | ✅ Implemented (small rule set) | Five rules behind a modular `LintRule` boundary: `oqci lint`. |
| Target model | ✅ Implemented | Basis profiles, **directed** connectivity, legality, and a target-supplied cost model. Profiles can be loaded at runtime (`--target-file`, `target=`). See [`docs/target_model.md`](docs/target_model.md). |
| Qubit mapping / routing / basis decomposition | ✅ Implemented | Layout, deterministic shortest-path routing with SWAP insertion, and 22 decomposition rules covering CX- and CZ-native bases. Each rule is verified against *two* independent implementations of gate semantics. See [`docs/lowering.md`](docs/lowering.md). |
| Backend contract | ✅ Implemented | Lowering, validation, preparation and execution as distinct stages, with full provenance. Adapter results convert back into `ExecutionResult`. See [`docs/backend_contract.md`](docs/backend_contract.md). |
| Extensions | ✅ Implemented (source-level) | Frontends, passes, backends and lint rules can be added without modifying the core. A demonstration crate is tested end to end. See [`docs/extensions.md`](docs/extensions.md). |
| Compiler orchestration | ✅ Implemented | One path from source to artifacts, with per-stage timings. See [`docs/compiler.md`](docs/compiler.md). |
| `oqci` CLI | ✅ Implemented | Every pipeline stage, `lint`, `--verify`, `--target-file`, and `watch`. `python -m oqci run` compiles and executes. See [`docs/cli.md`](docs/cli.md). |
| Live visualization | ✅ Implemented (local dev-companion) | `server/` + `frontend/`. See [`docs/visualization.md`](docs/visualization.md). |
| Simulator execution | ✅ Implemented (Aer, CUDA-Q) | `oqci.backends.aer` and `oqci.backends.cudaq`. CUDA-Q is checked against a real, pinned CUDA-Q in a Linux CI job, because it has no Windows wheels. See [`docs/adapters.md`](docs/adapters.md). |
| IBM hardware execution | ✅ Implemented (CZ-native Heron) · tested offline | `oqci.backends.ibm` builds the profile from the device's live target, checks the ISA (it never transpiles), and submits verbatim with twirling and dynamical decoupling off. It is tested end to end through IBM's local testing mode on a Heron snapshot. A run on real hardware is operator-run with your own account. See [`docs/hardware.md`](docs/hardware.md). |
| MLIR integration | ⛔ Out of scope (by design) | Rust owns the IR; MLIR stays optional (Stage B). |
| Benchmark suite | ✅ Implemented | Stage G is locked. The harness covers a 64-circuit corpus on three targets, OQCI against Qiskit O3/O0 and t\|ket⟩, an exhaustive correctness check of every output, determinism, interop, mutation evidence and a generated report. See [`docs/benchmarks.md`](docs/benchmarks.md). |
| Fuzzing and dependency policy | ✅ Implemented | Five cargo-fuzz targets, one of which hunts for miscompilations under `--verify`, plus cargo-deny and cargo-audit, in CI. |

For the authoritative, continuously-updated picture (not just this table), see
[`docs/core_architecture/index.md`](docs/core_architecture/index.md) and
[`CHANGELOG.md`](CHANGELOG.md).

## Try it

```bash
git clone https://github.com/AndyFerns/oqci.git
cd oqci
cargo run -- compile examples/bell.qasm
```

That parses a small OpenQASM 3 program and prints the QC-IR, the dependency graph, and the emitted QIR text, all in one shot. A couple more things worth trying:

```bash
# Compile a circuit for a device it doesn't fit on, and watch it get fixed:
# routing inserts SWAPs, decomposition rewrites every gate into the basis.
cargo run -- lower examples/ghz3.qasm --backend simulator-nisq

# See an optimization pass actually cancel a redundant gate pair, with a diff.
cargo run -- optimize examples/ghz3.qasm --diff

# Keep the pipeline re-running as you edit a file.
cargo run -- watch examples/bell.qasm --mode optimize

# Check that every transformation preserved the program's semantics, and lint it.
cargo run -- lower examples/ghz3.qasm --backend simulator-nisq --verify --lint

# Compile and run on Aer (after building the Python bindings, below).
python -m oqci run examples/bell.qasm --backend simulator-nisq --shots 2000
```

See [`examples/README.md`](examples/README.md) for what each sample file demonstrates, and [`docs/cli.md`](docs/cli.md) for the full command reference (JSON output, parameter binding, ablation runs, and so on).

## How it's organized

```text
  parse / adapt       bind          optimize            lower                 prepare
source ─────────▶ Circuit ─────▶ Circuit ─────────▶ Circuit ─────────▶ Lowered ──────────▶ Executable
(OpenQASM 3,      (QC-IR,      (Stage F,  (pass pipeline;   (layout, routing,     (what backends
 CUDA-Q, Qiskit)   validated)   optional)  QCO-IR built      decomposition,        and Aer
                                           internally)       verification)         consume)

Interop output: Circuit ──▶ QCO-IR (DAG) ──emit──▶ QIR text (for external LLVM/QIR tools)
```

| Directory | What's there |
|---|---|
| [`src/ir/`](src/ir/) | QC-IR, QCO-IR, validation, QIR emission — the foundation everything else builds on. |
| [`src/frontend/`](src/frontend/) | OpenQASM 3, CUDA-Q, and the Qiskit adapter's translation core. |
| [`src/pass/`](src/pass/) | The pass manager and the optimization passes. |
| [`src/analysis/`](src/analysis/) | Gate counts, depth, circuit diffing — the numbers everything else reports. |
| [`src/target/`](src/target/) | Basis profiles, topology, legality checking, cost models — what a device accepts. |
| [`src/lowering/`](src/lowering/) | Layout, routing, basis decomposition — applying a target to a circuit. |
| [`src/backend/`](src/backend/) | The backend contract, the executable representation, provenance. |
| [`src/compile.rs`](src/compile.rs) | The orchestrator: the one path from source to artifacts. |
| [`src/extension.rs`](src/extension.rs) | The extension registry: frontends, passes, backends and lint rules from outside the core. |
| [`src/verify/`](src/verify/) | In-compiler semantic verification. |
| [`src/cli/`](src/cli/) | The `oqci` inspector CLI. |
| [`python/`](python/) | PyO3 bindings, the Python SDK, and the execution adapters (`oqci/backends/`: Aer, CUDA-Q, IBM). |
| [`docs/`](docs/) | The locked architecture decisions and the normative specs for the IR, frontends, passes, targets, lowering, backends and CLI — read these before the source if you're getting oriented. |
| [`examples/`](examples/) | Small runnable programs: OpenQASM 3 (`.qasm`) and CUDA-Q kernels (`.py`), and [`plugin-demo/`](examples/plugin-demo/), an extension crate. |
| [`benchmarks/`](benchmarks/) | The Stage G benchmark and research harness, and the committed reference run. |
| [`fuzz/`](fuzz/) | cargo-fuzz targets and their seed corpus. |

Each directory also has its own `README.md` going into more depth.

## Building from source

```bash
cargo build                    # the oqci crate + CLI
cargo test                     # the Rust test suite
scripts/build.sh               # the full check suite (fmt, clippy, tests, docs)
```

Python bindings (optional, needed for the Qiskit path):

```bash
pip install -r python/requirements-dev.txt
maturin develop --manifest-path python/Cargo.toml
```

## License

Apache License 2.0 - see [`LICENSE`](LICENSE).
