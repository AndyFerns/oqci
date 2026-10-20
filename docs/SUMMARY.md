# Summary

[Overview](README.md)

---

# Source of Truth

- [Documentation Index and Locked Decisions](core_architecture/index.md)
  - [Framing and Research Question](core_architecture/framing-and-research-question.md)
  - [Stage A — Rust-Native IR Foundation](core_architecture/stage-a-rust-native-ir-foundation.md)
  - [Stage B — Modular MLIR Boundary](core_architecture/stage-b-modular-mlir-boundary.md)
  - [Stage C — Explicit Backend Contract](core_architecture/stage-c-explicit-backend-contract.md)
  - [Stage D — Backend-Specific Basis Profiles](core_architecture/stage-d-backend-specific-basis-profiles.md)
  - [Stage E — Backend-Defined Cost Model](core_architecture/stage-e-backend-defined-cost-model.md)
  - [Stage F — Static Parameterized Circuits](core_architecture/stage-f-static-parameterized-circuit-scope.md)
  - [Stage G — Benchmarking Protocol](core_architecture/stage-g-benchmarking-protocol.md)
  - [Final Deliverables Specification](core_architecture/final-deliverables-spec.md)

# Reference

- [IR Specification](ir_spec.md)
- [QIR Emission](qir_lowering.md)
- [MLIR Dialect](mlir_dialect.md)

# Frontends

- [Adapter Layout](adapters.md)
- [Gate Mapping](gate_mapping.md)
- [OpenQASM 3 Frontend](openqasm_frontend.md)
- [CUDA-Q Frontend](cudaq_frontend.md)
- [Qiskit Adapter](qiskit_adapter.md)

# Optimization

- [Pass Manager](pass_manager.md)
- [Verification and Lint](verification.md)

# Targets

- [Target Model](target_model.md)
- [Target Lowering](lowering.md)
- [Backend Contract](backend_contract.md)
- [IBM Quantum Hardware](hardware.md)

# Tooling

- [Compiler Orchestration](compiler.md)
- [Command Line](cli.md)
- [Live Visualization](visualization.md)
- [Extensions](extensions.md)

# Evaluation

- [Benchmarks and Research Package](benchmarks.md)
- [Determinism Contract](determinism.md)

# Positioning

- [Related Work](related_work.md)

# Architecture Decisions

- [The IR Contract (C1–C8)](architecture_decision_ir_contract.md)
- [No Frontend in Phase 0](architecture_decision_no_frontend.md)
- [Pure-Rust IR, MLIR as an Optional Later Layer](architecture_decision_mlir_phase2.md)
- [Adding SX / SXdg to the Gate Set](architecture_decision_sx_basis_gate.md)
