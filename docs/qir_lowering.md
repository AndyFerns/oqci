# QCO-IR → QIR Lowering

**Status:** normative for what `src/ir/qir.rs` emits today. **Boundary:** QIR
is an output artifact, not the execution path: a backend's executable
representation is a
separate artifact, built by a separate stage, because Stage C §5 forbids
treating emitted QIR as a guarantee that anything will run
(see [Backend Contract](backend_contract.md)). This document specifies the lowering implemented
in `src/ir/qir.rs`: the target format, where it is invoked, the op → QIR
mapping table, and its conformance caveats.

---

## 1. Target format

We emit **textual, LLVM-compatible QIR** in the classic typed-pointer form used
by the QIR specification examples:

- Opaque types `%Qubit = type opaque` and `%Result = type opaque`.
- Quantum intrinsics `__quantum__qis__*` and runtime calls `__quantum__rt__*`,
  emitted as `declare`d externs (deduplicated, sorted for deterministic output).
- A single entry-point function `define void @<name>() #0 { … ret void }`.
- Attributes on the entry-point function (attribute group `#0`):
  `"entry_point" "output_labeling_schema" "qir_profiles"="base_profile"
  "required_num_qubits"="N" "required_num_results"="M"`. No LLVM module flags
  (`!llvm.module.flags`) are emitted, and the `base_profile` label is applied
  to every module regardless of content; see §3.3.

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

### 1.4 Where QIR is emitted

`emit_qir` is not part of the compiler orchestrator: `compile::compile_circuit`
never calls it, and `CompilationArtifacts` carries no QIR. It is called
separately, on whichever circuit the caller holds:

| Caller | Circuit emitted |
|---|---|
| `oqci compile` (`src/cli/pipeline.rs`) | the source circuit, before optimization |
| `oqci optimize` | the optimized circuit |
| `oqci lower` | the lowered (target-legal) circuit |
| visualization server (`server/src/compile.rs`) | the optimized circuit |
| Python `qasm3_to_qir`, `qiskit_to_qir`, `cudaq_to_qir` (`python/src/lib.rs`) | the source circuit, after binding, with no optimization or lowering |

Only the `oqci lower` path can remove extended intrinsics (§3.1), because only
it has applied target lowering.

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
| `SX` | `__quantum__qis__sx__body` | q | **extended** |
| `SXdg` | `__quantum__qis__sxdg__body` | q | **extended** |
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
irregular names `cnot`, `s__adj`, `t__adj` follow QIR conventions. Membership
of the instruction set is not the same as being permitted by the Base Profile;
which of these operations the Base Profile admits has not been checked against
the specification.

## 3. Conformance caveats (intentional, documented)

Three caveats. The emitter itself performs no decomposition and no
reordering; it lowers exactly the circuit it is given.

### 3.1 Extended intrinsics

Gates with no member of the QIR *standard* instruction set (`id`, `p`, `u`,
`cy`, `swap`, `ccx`, `sx`, `sxdg`, and every `Opaque`) are emitted as declared
`__quantum__qis__*` externs. Every callee is declared, but no emitted module
has been checked by an LLVM or QIR tool (no `llvm-as` or QIR validator is part
of the build or of CI). One known defect: an `Opaque` gate's name is spliced
into the intrinsic name unchanged (`__quantum__qis__<name>__body`), and
validation only requires the name to be non-empty (I5), so a name containing,
for example, `-` or a space yields an identifier LLVM would reject.

Removing extended intrinsics is target lowering's job, not the emitter's:
lowering to a profile whose basis excludes them rewrites them with the
profile's decomposition rules ([`lowering.md`](lowering.md)). `linear-nisq`'s
basis is `{rz, sx, x, cx}`, so QIR emitted from a circuit lowered to it still
contains the extended `sx`; `ideal-simulator` accepts every registered gate,
so lowering to it removes none. The emitter does not decompose because doing so
would hide a choice of decomposition inside an output format.

### 3.2 Mid-circuit measurement

Strict QIR Base Profile expects all measurements at the end of the program. OQCI
circuits may contain mid-circuit measurement (a `Measure` before later gates on
the same or other qubits) and `Reset`. We emit operations in
program/topological order and do not reorder them.

No deferred-measurement pass exists. Absence of classical feed-forward (which
the IR guarantees, `ir_spec.md` §6) is necessary for deferring measurements but
not sufficient: a measurement cannot simply be moved to the end if a later gate
or reset acts on the measured qubit, or if the same classical bit is written
again, without introducing ancilla qubits — which would change
`required_num_qubits`. `linear-nisq` forbids mid-circuit measurement and reset,
so a circuit lowered to it has only terminal measurements; circuits emitted
from any other stage (§1.4) may not.

### 3.3 Profile label

Every module is labelled `"qir_profiles"="base_profile"` (`src/ir/qir.rs`),
including modules that contain extended intrinsics, mid-circuit measurement or
reset. The label is therefore a fixed string, not a conformance claim that has
been checked, and no validator has been run against any emitted module.

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
