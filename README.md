<div align="center">

# oqci - Open Quantum Computing Infrastructure

</div>

---

## About

OQCI is an open-source, modular, vendor-neutral compiler framework that:

- Ingests quantum programs from multiple SDKs (Qiskit, Cirq, CUDA-Q, OpenQASM)
- Translates them into a unified MLIR-based intermediate representation (IR)
- Optimizes via configurable compiler passes (gate cancellation, fusion, scheduling, mapping/routing)
- Lowers to QIR/LLVM IR for simulator or hardware execution
- Exposes a plugin SDK for new optimizations and analyses without core modifications

### Core Architectural Principle

LLVM-inspired ecosystem for quantum compilers:

- Separation of frontends, IR, optimization passes, and backends
- Progressive lowering through well-defined IR levels
- Hardware-agnostic middle layer (MLIR) that supports multiple targets
- Extensibility through a plugin/pass architecture

## THREE-LAYER COMPILER ARCHITECTURE

- LAYER 1: FRONTEND ADAPTERS

```plaintext
  OpenQASM Parser
  Qiskit Adapter
  Cirq Adapter
  CUDA-Q Adapter

  -> QC-IR (Imperative input)
```

- LAYER 2: MIDDLE (MLIR-Based)

```plaintext
  QC-IR ──▶ QCO-IR (Optimization IR, Functional Form)
             │
      ╔══════╩═════════════════════════════╗
      ║   [PASS MANAGER & OPTIMIZATION]   ║
      ╚══════╤═════════════════════════════╝
             │
             ├── Gate Cancellation
             ├── Gate Fusion
             ├── Rotation Merging
             ├── DAG Scheduling
             └── Qubit Mapping / Routing
```

- LAYER 3: BACKEND & EXECUTION

```plaintext
  QCO-IR ──▶ QIR / LLVM IR
          ──▶ Simulator (Qiskit Aer)
          ──▶ Hardware (IBM, future)
```

# License

This project is licensed under the [Apache v0.2](http://www.apache.org/licenses/LICENSE-2.0) license
