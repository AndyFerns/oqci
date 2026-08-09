# The `quantum` MLIR Dialect — Specification

**Status:** Phase 0 (written spec only). **No TableGen, C++, or FFI is built in
this phase.** This document defines the dialect that OQCI's Rust IR types
(`src/ir/`) mirror one-to-one, so that Phase 2 MLIR integration is a *mechanical
translation, not a redesign*. The binding constraints that keep it mechanical
are recorded in
[`architecture_decision_mlir_phase2.md`](architecture_decision_mlir_phase2.md).

The correspondence is intentionally total: **every** QC-IR/QCO-IR op has a named
MLIR op counterpart here, **every** Rust type boundary that would become an MLIR
type constraint is stated, and **every** field is classified as attribute vs.
operand.

---

## 1. Dialect overview

- **Namespace:** `quantum`
- **Levels represented:** QC-IR (imperative) and QCO-IR (DAG) both lower onto the
  same op set; the difference is regional/structural (see §6), not a different
  op vocabulary.
- **Value model:** qubits and results are **SSA values** of dedicated dialect
  types. A gate consumes a qubit value and produces a new qubit value (value
  semantics / linear threading), which is exactly the wire-threading QCO-IR
  already performs (`convert.rs`). This makes the QCO-IR graph and the MLIR SSA
  use-def graph *the same graph*.

## 2. Types

| MLIR type | Rust origin | Constraint |
|-----------|-------------|-----------|
| `!quantum.qubit` | `QubitId(u32)` | opaque handle to one qubit; SSA-typed |
| `!quantum.result` | `ClbitId(u32)` | classical measurement result; SSA-typed |
| `!quantum.angle` (≙ builtin `f64`/`FloatAttr`) | `Angle(f64)` | rotation parameter, radians |

**Type-constraint rationale.** In Rust the newtypes make `QubitId` and `ClbitId`
non-interchangeable; in MLIR the same guarantee is a *type constraint* on op
operands (a `quantum.gate` operand must be `!quantum.qubit`). The Rust newtype is
the cheap stand-in for that constraint today, so the Phase 2 verifier rule is
already implied by the Rust type. `Angle` maps to a `FloatAttr` (when a gate
parameter) or an `f64` SSA value (never needed in Phase 0, since all parameters
are compile-time constants → attributes).

## 3. Attributes vs. operands — the central mapping rule

> **Parameters (angles) become MLIR attributes. Qubit arguments become MLIR SSA
> operands. Classical targets become SSA operands/results of `!quantum.result`.**

This is enforced *structurally* in the Rust design: `GateKind` holds only
parameters (→ attributes), while `Instruction`/op nodes hold the qubit operands
(→ SSA operands). No field crosses the boundary, so the lowering never has to
"decide" whether something is an attribute or an operand — the Rust type already
committed.

## 4. Operations

Each op lists its assembly form, operands (SSA), attributes, and results, and
the exact Rust construct it lowers from.

### 4.1 `quantum.gate`  ⟵  `Instruction::Gate { kind, qubits }`

The single generic gate op. The gate identity is the `gate` string attribute
(the `GateKind` mnemonic); parameters are the `params` attribute array; qubits
are SSA operands and, under value semantics, are also results.

```mlir
%q0_1 = quantum.gate "h"() %q0_0 : (!quantum.qubit) -> !quantum.qubit
%q0_2, %q1_1 = quantum.gate "cx"() %q0_1, %q1_0
                 : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
%q_1  = quantum.gate "rz"(%theta) %q_0
                 : (!quantum.qubit) -> !quantum.qubit   // %theta is a FloatAttr
```

| Slot | Contents | Rust source |
|------|----------|-------------|
| attr `gate` | gate mnemonic, e.g. `"h"`, `"cx"`, `"rz"`, `"iswap"` | `GateKind::mnemonic()` |
| attr `params` | `array<f64>`, canonical param order | `GateKind::params()` |
| operands | `variadic<!quantum.qubit>`, control(s) then target | `Instruction::qubits` |
| results | one `!quantum.qubit` per operand (value semantics) | threaded in QCO-IR |

Registered mnemonics map to registered handling; the `Opaque` variant maps to a
`quantum.gate` whose `gate` attribute is an unregistered name — MLIR's
unregistered-op analogue. **Verifier rule (implied by Rust):** for a registered
mnemonic, `#operands == GateKind::arity()`, and operands are pairwise distinct
(Rust invariants I3, I4).

### 4.2 `quantum.measure`  ⟵  `Instruction::Measure { qubit, target }`

```mlir
%q_1, %r0 = quantum.measure %q_0 : (!quantum.qubit) -> (!quantum.qubit, !quantum.result)
```

Operand: one `!quantum.qubit`. Results: the post-measurement qubit value and one
`!quantum.result`. The `target` `ClbitId` becomes the SSA identity of the
`!quantum.result`. Non-unitary — a Phase 2 op trait `Collapsing` marks it so
passes can honor the control-dependency barrier.

### 4.3 `quantum.reset`  ⟵  `Instruction::Reset { qubit }`

```mlir
%q_1 = quantum.reset %q_0 : (!quantum.qubit) -> !quantum.qubit
```

Operand: one `!quantum.qubit`; result: the reset qubit value. Non-unitary
(`Collapsing`).

### 4.4 Boundary ops  ⟵  QCO-IR `NodeKind::Input` / `Output`

In a `quantum.circuit` region (§6), the qubit/result SSA values are the region's
block arguments (inputs) and terminator operands (outputs). QCO-IR's explicit
`Input`/`Output` boundary nodes correspond exactly to these block
arguments/terminator operands — no separate ops are required, but they are
listed here because the Rust IR materialises them as nodes.

## 5. The op ↔ Rust correspondence table (complete)

| Rust | MLIR op | Attributes | SSA operands | SSA results |
|------|---------|-----------|--------------|-------------|
| `Instruction::Gate` | `quantum.gate` | `gate`, `params` | qubits | qubits' |
| `Instruction::Measure` | `quantum.measure` | — | qubit | qubit', result |
| `Instruction::Reset` | `quantum.reset` | — | qubit | qubit' |
| `QcoCircuit` | `quantum.circuit` (region) | `sym_name` | — | — |
| `NodeKind::Input` | region block-arg | — | — | qubit/result |
| `NodeKind::Output` | region terminator operand | — | qubit/result | — |
| `QubitId` | `!quantum.qubit` value | — | — | — |
| `ClbitId` | `!quantum.result` value | — | — | — |
| `Angle` | `FloatAttr` (`f64`) | (is an attribute) | — | — |

## 6. Structural (regional) form

A whole circuit is a `quantum.circuit` op carrying a single region:

```mlir
quantum.circuit @bell() {
^entry(%q0_0: !quantum.qubit, %q1_0: !quantum.qubit):
  %q0_1        = quantum.gate "h"() %q0_0 : (!quantum.qubit) -> !quantum.qubit
  %q0_2, %q1_1 = quantum.gate "cx"() %q0_1, %q1_0
                   : (!quantum.qubit, !quantum.qubit) -> (!quantum.qubit, !quantum.qubit)
  %q0_3, %r0   = quantum.measure %q0_2 : (!quantum.qubit) -> (!quantum.qubit, !quantum.result)
  quantum.return %q0_3, %q1_1 : !quantum.qubit, !quantum.qubit
}
```

The SSA use-def chains in this region are **isomorphic to the QCO-IR DAG**:
data-dependency edges are value uses; the `Collapsing` trait on
`quantum.measure`/`quantum.reset` marks the control-dependency barriers. This is
why converting Rust QCO-IR to MLIR is graph relabeling, not re-analysis.

## 7. Phase 2 integration path

What Phase 2 adds on top of this spec (see the ADR for the "why" and the
non-negotiable constraints):

1. **TableGen definitions** (§8) generating the C++ op classes.
2. **A pass manager** hosting Phase 3 optimization passes over `quantum.circuit`
   regions.
3. **A conversion framework**: `quantum` → QIR/LLVM dialect lowering, replacing
   the textual emitter in `qir.rs` with a dialect conversion (the mnemonic →
   intrinsic table in [`qir_lowering.md`](qir_lowering.md) becomes the rewrite
   patterns).
4. **Op verification** encoding Rust invariants I1–I7 as `verify()` methods.
5. **The `src/ir/mlir_compat.rs` seam**: Rust↔MLIR marshalling lives here and
   nowhere else, so the rest of the crate stays MLIR-free.

## 8. Implied TableGen structure

A future contributor can begin the C++ port from this section alone. The Rust
types dictate the following `.td` skeleton:

```tablegen
// QuantumDialect.td
def Quantum_Dialect : Dialect {
  let name = "quantum";
  let cppNamespace = "::oqci::quantum";
}

// QuantumTypes.td   —— from src/ir/types.rs newtypes
def Quantum_QubitType  : TypeDef<Quantum_Dialect, "Qubit">  { let mnemonic = "qubit"; }
def Quantum_ResultType : TypeDef<Quantum_Dialect, "Result"> { let mnemonic = "result"; }

// QuantumOps.td      —— from src/ir/qc.rs Instruction + qco.rs boundaries
def Quantum_GateOp : Op<Quantum_Dialect, "gate", [Pure]> {
  let arguments = (ins StrAttr:$gate,                    // GateKind::mnemonic()
                       F64ArrayAttr:$params,             // GateKind::params()
                       Variadic<Quantum_QubitType>:$qubits);
  let results   = (outs Variadic<Quantum_QubitType>:$out);
  let hasVerifier = 1;                                   // encodes I3, I4
}
def Quantum_MeasureOp : Op<Quantum_Dialect, "measure", [Collapsing]> {
  let arguments = (ins Quantum_QubitType:$qubit);
  let results   = (outs Quantum_QubitType:$out, Quantum_ResultType:$result);
}
def Quantum_ResetOp : Op<Quantum_Dialect, "reset", [Collapsing]> {
  let arguments = (ins Quantum_QubitType:$qubit);
  let results   = (outs Quantum_QubitType:$out);
}
def Quantum_CircuitOp : Op<Quantum_Dialect, "circuit", [IsolatedFromAbove,
                          SingleBlockImplicitTerminator<"ReturnOp">]> {
  let arguments = (ins SymbolNameAttr:$sym_name);
  let regions   = (region SizedRegion<1>:$body);
}
def Quantum_ReturnOp : Op<Quantum_Dialect, "return", [Terminator]> {
  let arguments = (ins Variadic<Quantum_QubitType>:$operands);
}
```

Mapping back to the source of truth: `GateOp` ⟵ `Instruction::Gate`,
`MeasureOp` ⟵ `Instruction::Measure`, `ResetOp` ⟵ `Instruction::Reset`,
`CircuitOp`/`ReturnOp` ⟵ `QcoCircuit` + `Input`/`Output` boundary nodes,
`QubitType`/`ResultType` ⟵ `QubitId`/`ClbitId`, the `$params` attr ⟵ `Angle`
list. The `Collapsing` trait is the MLIR encoding of QCO-IR's `DepKind::Control`
barrier (`ir_spec.md` §4.2). The `hasVerifier` on `GateOp` is where Rust
invariants I3/I4 move.
