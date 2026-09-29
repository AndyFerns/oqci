# Related Work and Positioning

Status: informative. This page positions OQCI relative to other quantum
compilation infrastructure, per
[`core_architecture/framing-and-research-question.md`](core_architecture/framing-and-research-question.md).

**Evidence rule.** Every statement about an external project below was checked
against a primary source on **2026-09-30**, and the source is linked. Anything
that could not be checked is marked **to verify** and must not be cited as
fact until it is. This page makes **no performance or quality comparisons**;
those belong to Stage G, and only with evidence.

## 1. The QIR specification

The Quantum Intermediate Representation specification is maintained by the
QIR Alliance in [`qir-alliance/qir-spec`](https://github.com/qir-alliance/qir-spec).
Tagged releases exist for **1.0** (commit `e872897`), **2.0** and **2.1**.

What OQCI relies on, from the 1.0 tag:

- **The Base Profile** (`specification/profiles/Base_Profile.md`) prescribes
  an entry point returning `i64`, a four-block structure (initialization,
  unitary body, irreversible measurements, output recording), an
  `irreversible` attribute on measurement functions, `writeonly` result
  parameters, labeled output recording, entry-point attributes, and four
  module flags (`qir_major_version`, `qir_minor_version`,
  `dynamic_qubit_management`, `dynamic_result_management`).
- **No normative instruction set.** `specification/Instruction_Set.md` states
  that backends are not required to support every quantum instruction and that
  each backend declares which ones it supports. The spec's own examples use
  LLVM 13 typed pointers (`%Qubit*`).

**Relation to OQCI.** QIR is OQCI's interoperability boundary, not its IR. OQCI
emits QIR 1.0 at the output of compilation (`final-deliverables-spec.md` §14)
and does not read it back (§5.7).

## 2. PyQIR

[`qir-alliance/pyqir`](https://github.com/qir-alliance/pyqir), release
**v0.12.5**, is a Python library for generating, parsing and evaluating QIR.
Checked against that tag:

- Its QIS builder module (`pyqir/qis.py`) exposes `barrier`, `ccx`, `cx`, `cz`,
  `h`, `mz`, `reset`, `rx`, `ry`, `rz`, `s`, `s_adj`, `swap`, `t`, `t_adj`,
  `x`, `y`, `z` and `if_result`.
- It builds against LLVM 18, 19 or 20 (Cargo features `llvm18-1`, `llvm19-1`,
  `llvm20-1`), and its reference outputs use opaque pointers (`ptr`).
- Its `SimpleModule` helper writes QIR **2.0** module flags by default, and its
  reference QIS modules are labeled `"qir_profiles"="custom"`.
- It exposes `Module.from_ir` and `Module.verify`, which OQCI uses as an
  external validator.

**Relation to OQCI.** PyQIR's instruction set is the pinned reference for
OQCI's `qir-base` target profile, because QIR 1.0 itself defines none.

## 3. MLIR-based quantum compilers

These projects use MLIR as their compiler infrastructure and produce QIR
downstream. OQCI makes the opposite ordering choice: a Rust-native, validated
IR first, with MLIR optional (Stage B) and QIR at the boundary.

| Project | What was checked | Source |
|---|---|---|
| **CUDA-Q** (NVIDIA) | Defines MLIR dialects (e.g. `Quake`, `CC`, `QEC` `.td` files under `cudaq/include/cudaq/Optimizer/Dialect/`) and has QIR conversion passes (`cudaq/lib/Optimizer/CodeGen/ConvertToQIR*.cpp`). | [`NVIDIA/cuda-quantum`](https://github.com/NVIDIA/cuda-quantum), `main`, 2026-09-30 |
| **Catalyst** (PennyLane) | Its README states the core compiler is built using MLIR and lowers to LLVM + QIR, with a QIR-based runtime. | [`PennyLaneAI/catalyst`](https://github.com/PennyLaneAI/catalyst), `main` README, 2026-09-30 |
| **MQT Core** (Munich Quantum Toolkit) | Its repository description names an MLIR- and LLVM-based compiler collection and QIR execution. | [`munich-quantum-toolkit/core`](https://github.com/munich-quantum-toolkit/core), 2026-09-30 |

Details beyond the table — dialect semantics, how each validates its IR, which
QIR profile each targets — are **to verify**.

OQCI also ingests CUDA-Q kernels as a *frontend* (a documented subset;
[`cudaq_frontend.md`](cudaq_frontend.md)); that is independent of CUDA-Q's
compiler internals.

## 4. SDK transpilers used as baselines

The Qiskit transpiler and t|ket⟩ are named as benchmark baselines in
`final-deliverables-spec.md` §20.2. How their internal representations and
validation compare to OQCI's is **to verify**; no comparison is made here.

## 5. Other QIR tooling

[`qir-alliance/qir-runner`](https://github.com/qir-alliance/qir-runner) is
described by its repository as a QIR bytecode runner "to assist with QIR
development and validation". It is a candidate additional consumer for Stage G
interoperability evidence; which QIR versions and profiles it accepts is
**to verify**.
