# QCO-IR → QIR Lowering

**Status:** Phase 0 (stable). **Boundary:** QIR emission is the end of Phase 0;
there is no backend execution. This document specifies the lowering implemented
in `src/ir/qir.rs`: the target format, the op → QIR mapping table, and the two
documented conformance caveats.

---

## 1. Target format

We emit **textual, LLVM-compatible QIR** in the classic typed-pointer form used
by the QIR specification examples:

- Opaque types `%Qubit = type opaque` and `%Result = type opaque`.
- Quantum intrinsics `__quantum__qis__*` and runtime calls `__quantum__rt__*`,
  emitted as `declare`d externs (deduplicated, sorted for deterministic output).
- A single entry-point function `define void @<name>() #0 { … ret void }`.
- Base-Profile module flags on the entry attribute group:
  `"entry_point" "qir_profiles"="base_profile" "required_num_qubits"="N"
  "required_num_results"="M"`.

### 1.1 Static qubit/result addressing

QIR Base Profile uses statically-numbered qubits and results. A `QubitId(k)`
lowers to `%Qubit* inttoptr (i64 k to %Qubit*)` and a `ClbitId(k)` to
`%Result* inttoptr (i64 k to %Result*)`. `required_num_qubits` is the circuit's
`num_qubits`; `required_num_results` is its `num_clbits`.

### 1.2 Angle encoding — exact hex doubles

Angle parameters are emitted as LLVM **hexadecimal `double` literals**:
`0x` followed by the 16 hex digits of the IEEE-754 bit pattern
(`format!("0x{:016X}", x.to_bits())`). LLVM accepts this form exactly for every
finite `f64`, avoiding the pitfall where a rounded decimal literal is rejected or
silently altered. Example: `π` → `double 0x400921FB54442D18`.

### 1.3 Emission order

Operations are emitted in the deterministic topological order from
`QcoCircuit::topological_ops()` (`ir_spec.md` §4.3). Result-recording calls
(`__quantum__rt__result_record_output`) are appended after the operation body,
one per measured classical bit, in ascending result id.

## 2. Op → QIR mapping table

`quantum.gate` ⟶ `call void @__quantum__qis__<name>__body(<params?>, <qubits>)`,
parameters (as `double`) first, then qubit operands. `quantum.measure` and
`quantum.reset` map to their dedicated intrinsics.

| IR op / `GateKind` | QIR intrinsic | Args (in order) | Standard set? |
|--------------------|---------------|-----------------|---------------|
| `I` | `__quantum__qis__id__body` | q | extended |
| `X` | `__quantum__qis__x__body` | q | standard |
| `Y` | `__quantum__qis__y__body` | q | standard |
| `Z` | `__quantum__qis__z__body` | q | standard |
| `H` | `__quantum__qis__h__body` | q | standard |
| `S` | `__quantum__qis__s__body` | q | standard |
| `Sdg` | `__quantum__qis__s__adj` | q | standard |
| `T` | `__quantum__qis__t__body` | q | standard |
| `Tdg` | `__quantum__qis__t__adj` | q | standard |
| `Rx(θ)` | `__quantum__qis__rx__body` | θ, q | standard |
| `Ry(θ)` | `__quantum__qis__ry__body` | θ, q | standard |
| `Rz(θ)` | `__quantum__qis__rz__body` | θ, q | standard |
| `P(λ)` | `__quantum__qis__p__body` | λ, q | **extended** |
| `U{θ,φ,λ}` | `__quantum__qis__u__body` | θ, φ, λ, q | **extended** |
| `Cx` | `__quantum__qis__cnot__body` | ctrl, tgt | standard |
| `Cy` | `__quantum__qis__cy__body` | ctrl, tgt | **extended** |
| `Cz` | `__quantum__qis__cz__body` | ctrl, tgt | standard |
| `Swap` | `__quantum__qis__swap__body` | a, b | **extended** |
| `Ccx` | `__quantum__qis__ccx__body` | c0, c1, tgt | **extended** |
| `Opaque{name}` | `__quantum__qis__<name>__body` | params, qubits | **extended** |
| `Measure{q, c}` | `__quantum__qis__mz__body` | q, result(c) | standard |
| `Reset{q}` | `__quantum__qis__reset__body` | q | standard |
| (per measured clbit) | `__quantum__rt__result_record_output` | result(c), `i8* null` | runtime |

"standard" = part of the QIR specification's quantum instruction set; the
irregular names `cnot`, `s__adj`, `t__adj` follow QIR conventions.

## 3. Conformance caveats (intentional, documented)

Two honesty caveats. Both are consequences of Phase 0's scope (no optimization
or decomposition passes) and both are addressable by a Phase 3 pass without any
change to this lowering's structure.

### 3.1 Extended intrinsics

Gates with no member of the QIR *standard* instruction set (`id`, `p`, `u`,
`cy`, `swap`, `ccx`, and every `Opaque`) are emitted as declared
`__quantum__qis__*` externs. The emitted module is **valid LLVM IR** (every
callee is declared) and structurally valid QIR; a runtime that does not provide
these intrinsics would need a **decomposition pass** (Phase 3) to rewrite them
into the standard set — e.g. `swap → 3× cnot`, `ccx → the standard T/H/CX
decomposition`, `p(λ) → rz(λ)` up to an unobservable global phase.

**Rationale for not decomposing now.** Decomposition is a semantics-preserving
*transformation* — precisely the kind of optimization/rewrite pass that Phase 0
explicitly excludes. Baking it into the lowering would (a) smuggle a pass into a
phase that is supposed to have none, and (b) hard-code one decomposition choice
before the pass framework exists to make it configurable. Emitting a declared
extended intrinsic keeps the lowering total and defers the choice cleanly.

### 3.2 Mid-circuit measurement

Strict QIR Base Profile expects all measurements at the end of the program. OQCI
circuits may contain mid-circuit measurement (a `Measure` before later gates on
the same or other qubits). We emit operations in program/topological order, so
such a module is valid LLVM IR and runs correctly under a runtime that permits
mid-circuit measurement, but a strict Base-Profile linter would flag it.

**Rationale.** Making every circuit strictly Base-Profile-conformant requires a
**deferred-measurement pass** (push measurements to the end, valid only without
classical feed-forward — which Phase 0 guarantees). That is again a Phase 3
transformation. We keep the emitter faithful to the input and label the profile
honestly, rather than silently reordering.

## 4. Worked example — Bell state

QC-IR: `H q0 ; CX q0,q1 ; measure q0→c0 ; measure q1→c1`. Emitted QIR:

```llvm
; QIR module for circuit `bell`
%Qubit = type opaque
%Result = type opaque

declare void @__quantum__qis__cnot__body(%Qubit*, %Qubit*)
declare void @__quantum__qis__h__body(%Qubit*)
declare void @__quantum__qis__mz__body(%Qubit*, %Result*)
declare void @__quantum__rt__result_record_output(%Result*, i8*)

define void @bell() #0 {
entry:
  call void @__quantum__qis__h__body(%Qubit* inttoptr (i64 0 to %Qubit*))
  call void @__quantum__qis__cnot__body(%Qubit* inttoptr (i64 0 to %Qubit*), %Qubit* inttoptr (i64 1 to %Qubit*))
  call void @__quantum__qis__mz__body(%Qubit* inttoptr (i64 0 to %Qubit*), %Result* inttoptr (i64 0 to %Result*))
  call void @__quantum__qis__mz__body(%Qubit* inttoptr (i64 1 to %Qubit*), %Result* inttoptr (i64 1 to %Result*))
  call void @__quantum__rt__result_record_output(%Result* inttoptr (i64 0 to %Result*), i8* null)
  call void @__quantum__rt__result_record_output(%Result* inttoptr (i64 1 to %Result*), i8* null)
  ret void
}

attributes #0 = { "entry_point" "output_labeling_schema" "qir_profiles"="base_profile" "required_num_qubits"="2" "required_num_results"="2" }
```

## 5. Phase 2 note

Under MLIR (Phase 2), this textual emitter is replaced by a dialect conversion
from `quantum` to the QIR/LLVM dialect. The mapping table in §2 becomes the set
of rewrite patterns; the extended-intrinsic and deferred-measurement caveats
become opt-in conversion/decomposition passes. Nothing in the table changes —
only the mechanism that applies it. See
[`mlir_dialect.md`](mlir_dialect.md) §7 and the ADR.
