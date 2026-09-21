<div align="center">

# <svg width="24" height="24" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M21 16.5V5.75A2.755 2.755 0 0 0 18.25 3H5.75A2.755 2.755 0 0 0 3 5.75V16.5H2v1.75A2.755 2.755 0 0 0 4.75 21h14.5A2.755 2.755 0 0 0 22 18.25V16.5h-1ZM4.5 5.75c0-.69.56-1.25 1.25-1.25h12.5c.69 0 1.25.56 1.25 1.25V16.5h-15V5.75Zm16 12.5c0 .69-.56 1.25-1.25 1.25H4.75c-.69 0-1.25-.56-1.25-1.25V18h17v.25Z" fill="#ffffff"/></svg> oqci <svg width="24" height="24" fill="none" xmlns="http://www.w3.org/2000/svg"><path fill-rule="evenodd" clip-rule="evenodd" d="M19.58 11.485a7.99 7.99 0 0 1-.425.515v.01l.128.154c.102.121.202.241.297.361 1.36 1.74 1.745 3.335 1.08 4.485-.375.645-1.225 1.4-3.175 1.4H17.3l.035-1.5c1.04.03 1.775-.21 2.025-.65.33-.57-.03-1.62-.96-2.815a4.684 4.684 0 0 0-.197-.24l-.098-.115c-.53.49-1.115.97-1.745 1.435-.495 4.49-2.225 7.485-4.36 7.485-1.615 0-2.82-1.58-3.575-3.81-.685.135-1.34.215-1.94.215-1.43 0-2.575-.42-3.145-1.405-.385-.665-.615-1.835.465-3.61l1.28.78c-.54.885-.7 1.64-.45 2.08.34.59 1.585.825 3.38.495-.165-.71-.29-1.46-.375-2.225C4 11.855 2.27 8.86 3.34 7.01c.81-1.4 2.78-1.65 5.085-1.19.755-2.23 1.96-3.81 3.575-3.81.775 0 1.9.385 2.9 2.22l-1.32.715c-.495-.915-1.07-1.44-1.58-1.44-.68 0-1.505.96-2.12 2.68.7.215 1.41.475 2.115.785C16.13 5.15 19.59 5.15 20.66 7c.66 1.15.28 2.74-1.08 4.485Zm-1.18-.92c.93-1.195 1.29-2.245.96-2.815-.475-.82-2.645-.93-5.555.11a21.124 21.124 0 0 1 4.3 3.06l.098-.116c.069-.08.136-.158.197-.239ZM8.02 7.255c-1.795-.33-3.04-.095-3.38.495-.475.82.51 2.755 2.865 4.755C7.5 12.335 7.5 12.17 7.5 12c0-1.615.18-3.265.52-4.745Zm1.445.355C9.18 8.84 9 10.315 9 12c0 1.685.18 3.16.465 4.39 1.205-.365 2.575-.945 4.035-1.79 1.36-.785 2.575-1.68 3.565-2.6-.99-.925-2.205-1.815-3.565-2.6-1.46-.84-2.825-1.42-4.035-1.79Zm.415 10.205c.61 1.72 1.44 2.685 2.12 2.685v-.005c.95 0 2.13-1.825 2.685-4.865l-.094.059c-.112.07-.224.14-.341.206-1.4.81-2.915 1.475-4.37 1.92ZM13.5 12a1.5 1.5 0 1 1-3 0 1.5 1.5 0 0 1 3 0Z" fill="#ffffff"/></svg>

**Open Quantum Compiler Infrastructure**

An open-source, vendor-neutral compiler stack for quantum circuits - built like LLVM, not like a single SDK's transpiler.

[Docs](docs/README.md) · [CLI guide](docs/cli.md) · [Pass manager](docs/pass_manager.md) · [Changelog](CHANGELOG.md)

</div>

## What is this?

Most quantum SDKs (Qiskit, Cirq, ...) ship their own compiler baked into the SDK, so an optimization pass you write only works inside that one ecosystem. OQCI tries the LLVM approach instead: pull circuits from different front-ends into one shared, well-specified intermediate representation, run optimizations against *that*, and lower the result to a common output (QIR) so the SDK you started in stops mattering.

Concretely, that means:

- **Frontends** translate circuits from a source language/SDK into OQCI's IR, and nothing else. A frontend never gets to redefine what the IR means.
- **QC-IR / QCO-IR** are the two IR levels - an imperative instruction list and a dependency-graph form for optimization - with every invariant validated at construction, not hoped for later.
- **A pass manager** runs explicit, ordered, individually-toggleable optimization passes over the IR.
- **QIR lowering** emits LLVM-compatible textual IR as the output boundary, so downstream simulators or hardware backends aren't OQCI's problem to solve twice.

It's Rust-native at its core (fast, and the invariants are enforced by the type system rather than by convention), with Python bindings for the parts that need to talk to Qiskit.

## Project status

**Current version: `0.3.0`.** This is early-stage, actively-developed infrastructure - treat anything not checked off below as *not there yet*, regardless of what a directory name or diagram might suggest.

| Layer | Status | Notes |
|---|---|---|
| QC-IR / QCO-IR core | ✅ Implemented | Validated construction, deterministic QC-IR → QCO-IR conversion, QIR (LLVM-compatible) lowering. See [`docs/ir_spec.md`](docs/ir_spec.md). |
| Symbolic parameters | ✅ Implemented | Gate angles can be concrete or symbolic (`Rz(theta)`), with explicit binding before lowering - for VQE-style parameterized circuits. |
| OpenQASM 3 frontend | ✅ Implemented | A documented, intentionally partial subset - see [`docs/openqasm_frontend.md`](docs/openqasm_frontend.md) for exactly what's in and what's refused. |
| Qiskit frontend | ✅ Implemented | Pure-Rust translation core plus a PyO3 boundary exposed to Python. See [`python/README.md`](python/README.md). |
| Cirq / CUDA-Q frontends | ⛔ Not started | Reserved in the design; no code yet. |
| Pass manager | ✅ Implemented | Explicit registration/ordering, enable/disable per pass, deterministic execution, ablation support. See [`docs/pass_manager.md`](docs/pass_manager.md). |
| Optimization passes | ✅ Implemented (target-independent only) | Canonicalization, gate cancellation, rotation merging, and scheduling-as-analysis - all verified against an independent state-vector simulator, not just unit-tested. Passes can now consult the selected target, though none of the shipped ones needs to. |
| Target model | ✅ Implemented | Basis profiles, **directed** connectivity, legality checking, and a cost model the *target* supplies rather than the optimizer assuming. See [`docs/target_model.md`](docs/target_model.md). |
| Qubit mapping / routing / basis decomposition | ✅ Implemented | Layout, deterministic shortest-path routing with SWAP insertion, and 18 decomposition rules — each verified against *two* independent implementations of gate semantics. Lowering re-checks its own output and refuses to return a circuit the target rejects. See [`docs/lowering.md`](docs/lowering.md). |
| Backend contract | ✅ Implemented | Lowering, validation, execution preparation and execution as four distinct stages, with full provenance on every result. See [`docs/backend_contract.md`](docs/backend_contract.md). |
| Compiler orchestration | ✅ Implemented | One path from source to artifacts, taken by the CLI, the Python SDK and library callers alike. See [`docs/compiler.md`](docs/compiler.md). |
| `oqci` CLI | ✅ Implemented | Inspect every pipeline stage for a real program — now including `lower` and `prepare` — plus a live `watch` mode. See [`docs/cli.md`](docs/cli.md). |
| Simulator execution | ✅ Implemented | `oqci.backends.aer` runs a prepared circuit on Qiskit Aer, which is how the compiler's output is checked against something other than itself. |
| IBM hardware execution | 🚧 Prepared, not submitted | Lowering, validation, the executable artifact and its provenance all exist and are tested. **Submission does not.** That needs an SDK this project doesn't depend on and credentials it doesn't have — and untested code between a verified circuit and real hardware is worse than an honest gap. **No claim is made that anything here will run on an IBM device.** |
| MLIR integration | ⛔ Not started (by design) | Rust owns the IR first; MLIR comes later, per the [locked architecture decisions](docs/core_architecture/index.md). |
| Benchmark suite | ⛔ Not started | [`benchmarks/`](benchmarks/) is a placeholder; the experimental protocol isn't locked yet either. |

For the authoritative, continuously-updated picture (not just this table), see
[`docs/core_architecture/index.md`](docs/core_architecture/index.md) and
[`CHANGELOG.md`](CHANGELOG.md).

## Try it

```bash
git clone https://github.com/AndyFerns/oqci.git
cd oqci
cargo run -- compile examples/bell.qasm
```

That parses a small OpenQASM 3 program and prints the QC-IR, the dependency graph, and the emitted QIR, all in one shot. A couple more things worth trying:

```bash
# Compile a circuit for a device it doesn't fit on, and watch it get fixed:
# routing inserts SWAPs, decomposition rewrites every gate into the basis.
cargo run -- lower examples/ghz3.qasm --backend simulator-nisq

# See an optimization pass actually cancel a redundant gate pair, with a diff.
cargo run -- optimize examples/ghz3.qasm --diff

# Keep the pipeline re-running as you edit a file.
cargo run -- watch examples/bell.qasm --mode optimize
```

See [`examples/README.md`](examples/README.md) for what each sample file demonstrates, and [`docs/cli.md`](docs/cli.md) for the full command reference (JSON output, parameter binding, ablation runs, and so on).

## How it's organized

```text
        parse/adapt          build         optimize        convert       lower
 source ──────────▶ QC-IR ─────────▶ Circuit ────────▶ Circuit ───────▶ QCO-IR ─────▶ QIR
(QASM 3, Qiskit)   (imperative)     (validated)     (pass pipeline)     (DAG)    (LLVM-compatible)
```

| Directory | What's there |
|---|---|
| [`src/ir/`](src/ir/) | QC-IR, QCO-IR, validation, QIR lowering - the foundation everything else builds on. |
| [`src/frontend/`](src/frontend/) | OpenQASM 3 and the Qiskit adapter's translation core. |
| [`src/pass/`](src/pass/) | The pass manager and the optimization passes. |
| [`src/analysis/`](src/analysis/) | Gate counts, depth, circuit diffing - the numbers everything else reports. |
| [`src/cli/`](src/cli/) | The `oqci` inspector CLI. |
| [`python/`](python/) | PyO3 bindings for the Qiskit-facing side. |
| [`docs/`](docs/) | Normative specs for the IR, frontends, passes, and CLI - read these before the source if you're getting oriented. |
| [`examples/`](examples/) | Small runnable `.qasm` programs. |

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
