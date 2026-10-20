# QCO-IR → QIR Emission

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

We emit **textual, LLVM-compatible QIR 1.0**, structured as the Base Profile
specifies. The specification checked is `qir-alliance/qir-spec` at tag `1.0`,
commit `e872897d107516d889718b5b6f5fee909dd64238`,
`specification/profiles/Base_Profile.md`. Every module contains:

- An entry point that takes no arguments and returns `i64`:
  `define i64 @<name>() #0 { … ret i64 0 }`, exit code `0` meaning success.
- An `entry` block calling `__quantum__rt__initialize(null)`, a `body` block,
  and an `output` block that records results and returns. A module labeled
  `base_profile` (§3.3) also has a `measurements` block between `body` and
  `output`, as the Base Profile requires: `body` holds only non-irreversible
  calls, `measurements` only irreversible ones. A `custom` module keeps every
  operation, measurement and reset included, in `body` in topological order.
- `__quantum__qis__mz__body` declared with attribute group
  `#1 = { "irreversible" }`, and its result parameter marked `writeonly`, in
  the declaration and at every call site.
- Output recording: one `__quantum__rt__result_record_output` call per
  measured classical bit, in ascending order, each passing a non-null pointer
  to its own null-terminated global label (`@0 = internal constant [3 x i8]
  c"c0\00"`, …). Labels are the classical bit's name, `c<index>`.
- Entry-point attributes (group `#0`): `"entry_point"`,
  `"output_labeling_schema"="oqci_clbit"`, `"qir_profiles"`, and
  `"required_num_qubits"` / `"required_num_results"` (§1.1).
- The four module flags the Base Profile requires:

  ```llvm
  !0 = !{i32 1, !"qir_major_version", i32 1}
  !1 = !{i32 7, !"qir_minor_version", i32 0}
  !2 = !{i32 1, !"dynamic_qubit_management", i1 false}
  !3 = !{i32 1, !"dynamic_result_management", i1 false}
  ```

- Quantum intrinsics `__quantum__qis__*` and runtime functions
  `__quantum__rt__*`, `declare`d once each and sorted, for deterministic
  output.

### 1.1 Static qubit/result addressing

Qubits and results are identified by constant integers cast to pointers
(static management, which is why the two `dynamic_*_management` flags are
false). `QubitId(k)` becomes `inttoptr (i64 k to …)` in qubit position and
`ClbitId(k)` the same in result position.

`required_num_qubits` is the highest qubit index the entry point uses, plus
one; `required_num_results` is the highest measured classical bit, plus one.
Both count what the program *uses*, as the Base Profile's attribute
definitions ask — not the declared register size, which for a lowered circuit
is the device's width.

### 1.2 Pointer style

Opaque pointers (`ptr`) are the default: they are what LLVM 17 and later
print and what PyQIR 0.12.5 emits. Typed pointers — `%Qubit*`, `%Result*`,
`i8*` labels passed through `getelementptr inbounds`, with `%Qubit` and
`%Result` declared as opaque types — reproduce the specification's LLVM 13
examples, which is the spelling older LLVM releases (LLVM 14, for example)
default to. The choice changes spelling only, never structure
(`ir::PointerStyle`). Which tools accept which style is recorded in §4 from
observed runs, not assumed.

### 1.3 Angle encoding and emission order

Angle parameters are emitted as LLVM **hexadecimal `double` literals**:
`0x` followed by the 16 hex digits of the IEEE-754 bit pattern
(`format!("0x{:016X}", x.to_bits())`). LLVM accepts this form exactly for every
finite `f64`, avoiding the pitfall where a rounded decimal literal is rejected or
silently altered. Example: `π` → `double 0x400921FB54442D18`.

Operations are emitted in the deterministic topological order from
`QcoCircuit::topological_ops()` (`ir_spec.md` §4.3). In a `base_profile`
module, measurements move to the `measurements` block in that same relative
order; this reorders nothing observable, because a Base-labeled circuit never
touches a qubit after measuring it (§3.3).

### 1.4 Where QIR is emitted

The compiler orchestrator emits one canonical QIR artifact per compilation,
from the most-lowered circuit available (`CompilationArtifacts::qir`, see
[`compiler.md`](compiler.md#qirartifact)): the lowered circuit when a backend
was selected and lowering ran, the optimized circuit otherwise. Every tool
shows that artifact; none emits QIR of its own:

| Caller | QIR shown |
|---|---|
| `oqci compile`, `optimize`, `lower`, `prepare`, `watch` (`src/cli/pipeline.rs`) | **the canonical artifact.** Shown with its label and source stage; see [`cli.md`](cli.md#the-qir-stage). |
| visualization server (`server/src/compile.rs`) | **the canonical artifact**, with its label and source stage shown above the text. |
| Python `oqci.compile` (the `qir` entry), `qasm3_to_qir`, `qiskit_to_qir`, `cudaq_to_qir` (`python/src/lib.rs`) | **the canonical artifact.** The `*_to_qir` helpers run the pipeline (optimization, and lowering when given a `backend`) and return its text. |

Only a compilation with a backend can remove instructions outside the Base set
(§3.1), because only it applies target lowering; `qir-base` is the backend that
removes all of them. `ir::emit_qir` and `ir::emit_qir_with` remain public for
library callers that hold a circuit and want QIR text directly; they always
label the module `custom` (§3.3).

## 2. Op → QIR mapping table

`quantum.gate` ⟶ `call void @__quantum__qis__<name>__body(<params?>, <qubits>)`,
parameters (as `double`) first, then qubit operands. `quantum.measure` and
`quantum.reset` map to their dedicated intrinsics.

| IR op / `GateKind` | QIR intrinsic | Args (in order) | In PyQIR 0.12.5's QIS? |
|--------------------|---------------|-----------------|---------------|
| `I` | `__quantum__qis__id__body` | q | no |
| `X` | `__quantum__qis__x__body` | q | yes |
| `Y` | `__quantum__qis__y__body` | q | yes |
| `Z` | `__quantum__qis__z__body` | q | yes |
| `H` | `__quantum__qis__h__body` | q | yes |
| `S` | `__quantum__qis__s__body` | q | yes |
| `Sdg` | `__quantum__qis__s__adj` | q | yes |
| `T` | `__quantum__qis__t__body` | q | yes |
| `Tdg` | `__quantum__qis__t__adj` | q | yes |
| `SX` | `__quantum__qis__sx__body` | q | **no** |
| `SXdg` | `__quantum__qis__sxdg__body` | q | **no** |
| `Rx(θ)` | `__quantum__qis__rx__body` | θ, q | yes |
| `Ry(θ)` | `__quantum__qis__ry__body` | θ, q | yes |
| `Rz(θ)` | `__quantum__qis__rz__body` | θ, q | yes |
| `P(λ)` | `__quantum__qis__p__body` | λ, q | **no** |
| `U{θ,φ,λ}` | `__quantum__qis__u__body` | θ, φ, λ, q | **no** |
| `Cx` | `__quantum__qis__cnot__body` | ctrl, tgt | yes |
| `Cy` | `__quantum__qis__cy__body` | ctrl, tgt | **no** |
| `Cz` | `__quantum__qis__cz__body` | ctrl, tgt | yes |
| `Swap` | `__quantum__qis__swap__body` | a, b | yes |
| `Ccx` | `__quantum__qis__ccx__body` | c0, c1, tgt | yes |
| `Opaque{name}` | `__quantum__qis__<name>__body` | params, qubits | **no** |
| `Measure{q, c}` | `__quantum__qis__mz__body` (`irreversible`) | q, `writeonly` result(c) | yes |
| `Reset{q}` | `__quantum__qis__reset__body` | q | yes (never in a Base module, §3.3) |
| (entry) | `__quantum__rt__initialize` | `null` | runtime |
| (per measured clbit) | `__quantum__rt__result_record_output` | result(c), label | runtime |

**Why the last column is PyQIR and not "the standard set".** QIR 1.0 defines
no normative quantum instruction set: `specification/Instruction_Set.md` says
backends are not required to support every instruction and each declares its
own. An earlier revision of this table called some rows "standard" and others
"extended"; that classification had no source in the specification. The
reference used instead is the instruction set PyQIR 0.12.5 exposes
(`pyqir/qis.py`), which is pinned and checkable, and which the `qir-base`
target profile is built on ([`target_model.md`](target_model.md)). The
irregular names `cnot`, `s__adj`, `t__adj` are the ones PyQIR uses.

## 3. Conformance caveats (intentional, documented)

Three caveats. The emitter itself performs no decomposition; apart from
moving measurements into their own block in a Base-labeled module (§1.3), it
emits exactly the circuit it is given.

### 3.1 Instructions outside PyQIR's instruction set

Gates outside PyQIR 0.12.5's QIS (`id`, `p`, `u`, `cy`, `sx`, `sxdg`, and
every `Opaque`) are emitted as declared `__quantum__qis__*` externs; a consumer
must supply them. Every callee is declared. An `Opaque` gate's name is kept verbatim in its
intrinsic name (`__quantum__qis__<name>__body`); validation only requires it to
be non-empty (I5), so when the result falls outside LLVM's unquoted identifier
syntax (`[-a-zA-Z$._0-9]`, not starting with a digit) — a space, say — the
symbol is emitted quoted, `@"__quantum__qis__my gate__body"`, with `"`, `\` and
non-printable bytes written as `\XX` hex escapes. Nothing is mangled, so two
distinct gates cannot collide on one symbol.

Removing them is target lowering's job, not the emitter's: lowering to a
profile whose basis excludes them rewrites them with the profile's
decomposition rules ([`lowering.md`](lowering.md)). Lowering onto the
`qir-base` target removes all of them except `Opaque`; `linear-nisq`'s basis
is `{rz, sx, x, cx}`, so QIR emitted from a circuit lowered to it still
contains `sx`; `ideal-simulator` accepts every registered gate, so lowering to
it removes none. The emitter does not decompose because doing so would hide a
choice of decomposition inside an output format.

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

A module is labeled `"qir_profiles"="base_profile"` **if and only if** its
circuit is legal against the `qir-base` target profile
([`target_model.md`](target_model.md)): every gate in the Base instruction set
(PyQIR 0.12.5's, §2), measurements terminal, no reset, no unbound parameter.
Otherwise it is labeled `"custom"`, which claims no profile — the label PyQIR
itself uses for modules outside a profile.

The decision is made by the compiler orchestrator, which emits the canonical
QIR artifact for each compilation
([`compiler.md`](compiler.md#qirartifact)). A strict mode
(`QirProfileRequirement::Base`) refuses, with `CompileError::QirProfile`,
instead of emitting `custom`.

The IR layer cannot make the claim on its own, so its public emitters,
`ir::emit_qir` and `ir::emit_qir_with`, always write `custom`. The Base form
comes only from a crate-private emitter the orchestrator calls after the
check. That emitter also refuses, with `IrError::NotBaseProfile`, a circuit
that resets a qubit or uses one after measuring it, so even a wrong caller
cannot produce a Base-labeled module that breaks those rules.

To get a Base-labeled module for a circuit that uses `sx`, `sxdg`, `id`, `p`,
`u` or `cy`, compile it for the `qir-base` backend: lowering rewrites those
gates into the Base instruction set first. A circuit with mid-circuit
measurement or reset can never be Base-labeled (§3.2).

## 4. Validation

What has been checked, by what, and what each check does and does not prove.
Every claim here names the tool and version; none is inferred.

| Check | By | What it proves | Status |
|---|---|---|---|
| Base Profile structure (i64 entry point, the four blocks in order, `initialize` first, only unitaries in `body`, only `mz` in `measurements`, no reset, irreversible `mz` with `writeonly` result, non-null unique labels, attributes, the four module flags) on every example × every backend × both pointer styles | `tests/qir_conformance.rs` (OQCI's own test) | That OQCI's output matches OQCI's reading of `Base_Profile.md` at commit `e872897`. **Not independent evidence**: the same authors wrote the emitter and the check. | Runs in `cargo test`, on every CI platform. |
| Semantics of emission: each module's entry block is decoded back into a circuit (from the text alone, sharing no code with the emitter) and must have the source's operator, column by column over every basis state, and its qubit-to-result measurement map. Run on every example, a circuit using every gate kind with operands in both orders, and 128 random circuits, in both pointer styles | `tests/qir_semantics.rs` (OQCI's own test) | That emission neither reorders operands nor changes a gate or angle, up to OQCI's own decoding of its intrinsics. Not execution by a QIR consumer, which nothing in the toolchain does. Added after the Stage G mutant `qir-cx-operands-swapped` survived every structural check. | Runs in `cargo test`, on every CI platform. |
| Parse + `Module.verify()`, and read back `qir_major_version` = 1, `qir_minor_version` = 0, `dynamic_*_management` = false, one entry point with readable `required_num_*` | PyQIR **0.12.5** (`scripts/validate_qir.py`) | The module is well-formed LLVM IR to PyQIR's LLVM, and its QIR metadata is readable with the intended values. Not Base Profile conformance. | **Observed passing** locally on 2026-09-30: 26/26 opaque and 26/26 typed modules (the corpus above plus one module of quoted `Opaque` names). As a negative control, the validator rejects a module lacking the QIR 1.0 module flags and entry-point attributes, as pre-0.5.0 output did. Also observed passing in CI job `qir-validate`, run [39384077389](https://github.com/AndyFerns/oqci/actions/runs/39384077389). |
| `llvm-as` on every module | LLVM **18** (`llvm-as-18`, ubuntu-24.04), both pointer styles | The text is valid LLVM IR for that LLVM release. | CI job `qir-validate`. **Observed passing** in CI on 2026-10-20, run [39384077389](https://github.com/AndyFerns/oqci/actions/runs/39384077389) (commit `d70c44a`, `llvm-as-18` from the ubuntu-24.04 archive), both pointer styles. Not observed locally: no `llvm-as` is installed on the development machine. |
| `llvm-as` on typed-pointer modules | LLVM **14** (`llvm-as-14`, ubuntu-22.04) | The typed spelling is valid IR for an LLVM that defaults to typed pointers, as the specification's examples do. | CI job `qir-typed-llvm14`. **Observed passing** in the same CI run (`llvm-as-14` from the ubuntu-22.04 archive). |

Interoperability evidence is consumer-side (`final-deliverables-spec.md`
§14.3): a named tool at a pinned version accepted the artifact. No tool has yet
*executed* an OQCI QIR module; that is Stage G territory.

The corpus can be regenerated and checked locally:

```bash
OQCI_QIR_CORPUS_DIR=qir-corpus cargo test --test qir_conformance
pip install -r python/requirements-qir.txt
python scripts/validate_qir.py qir-corpus/opaque
python scripts/validate_qir.py qir-corpus/typed
```

## 5. Worked example — Bell state

QC-IR: `H q0 ; CX q0,q1 ; measure q0→c0 ; measure q1→c1`. Emitted QIR:

```llvm
; QIR module for circuit `bell`
; Generated by OQCI: QIR 1.0 (qir-spec tag 1.0, commit e872897), profile custom, opaque pointers.
source_filename = "bell"

@0 = internal constant [3 x i8] c"c0\00"
@1 = internal constant [3 x i8] c"c1\00"

define i64 @bell() #0 {
entry:
  call void @__quantum__rt__initialize(ptr null)
  br label %body

body:
  call void @__quantum__qis__h__body(ptr inttoptr (i64 0 to ptr))
  call void @__quantum__qis__cnot__body(ptr inttoptr (i64 0 to ptr), ptr inttoptr (i64 1 to ptr))
  call void @__quantum__qis__mz__body(ptr inttoptr (i64 0 to ptr), ptr writeonly inttoptr (i64 0 to ptr))
  call void @__quantum__qis__mz__body(ptr inttoptr (i64 1 to ptr), ptr writeonly inttoptr (i64 1 to ptr))
  br label %output

output:
  call void @__quantum__rt__result_record_output(ptr inttoptr (i64 0 to ptr), ptr @0)
  call void @__quantum__rt__result_record_output(ptr inttoptr (i64 1 to ptr), ptr @1)
  ret i64 0
}

declare void @__quantum__qis__cnot__body(ptr, ptr)
declare void @__quantum__qis__h__body(ptr)
declare void @__quantum__qis__mz__body(ptr, ptr writeonly) #1
declare void @__quantum__rt__initialize(ptr)
declare void @__quantum__rt__result_record_output(ptr, ptr)

attributes #0 = { "entry_point" "output_labeling_schema"="oqci_clbit" "qir_profiles"="custom" "required_num_qubits"="2" "required_num_results"="2" }
attributes #1 = { "irreversible" }

!llvm.module.flags = !{!0, !1, !2, !3}

!0 = !{i32 1, !"qir_major_version", i32 1}
!1 = !{i32 7, !"qir_minor_version", i32 0}
!2 = !{i32 1, !"dynamic_qubit_management", i1 false}
!3 = !{i32 1, !"dynamic_result_management", i1 false}
```

This is the `custom` form: the measurements stay in `body`. The same circuit
emitted as `base_profile` (§3.3) has them in a `measurements` block between
`body` and `output`.

## 6. If MLIR is adopted

MLIR is optional (Stage B). If it is adopted, this textual emitter could be
replaced by a dialect conversion from `quantum` to the QIR/LLVM dialect. The mapping table in §2 becomes the set
of rewrite patterns; the extended-intrinsic and deferred-measurement caveats
become opt-in conversion/decomposition passes. Nothing in the table changes —
only the mechanism that applies it. See
[`mlir_dialect.md`](mlir_dialect.md) §7 and the ADR.
