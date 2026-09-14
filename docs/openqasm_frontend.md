# OpenQASM 3 Frontend — Supported Subset

Status: normative
Implemented by: `src/frontend/openqasm/`
Entry points: `parse_openqasm3`, `parse_openqasm3_named`

This document defines **exactly** what the OpenQASM 3 frontend accepts.
`final-deliverables-spec.md` §5.2 requires the supported subset to be
documented precisely and forbids claiming full OpenQASM 3 coverage. OQCI does
not implement full OpenQASM 3, and this page is the honest boundary.

Everything outside the subset is **refused** with a diagnostic naming the
construct. Nothing is silently ignored. That matters most for control flow: a
frontend that skipped an `if` would compile the branch as though it were
unconditional, changing the program's meaning without saying so.

## Pipeline

```text
source → lexer → parser → AST → translate → CircuitBuilder → Circuit
         (positions)      (narrow)  (symbol table)   (validation)
```

Each stage is independently testable, and the translator performs **no**
validation QC-IR already performs — gate arity, duplicate operands and
out-of-range bits surface from `CircuitBuilder::build` as
`FrontendError::Ir`.

## Supported

### Header and includes

```qasm
OPENQASM 3.0;              // optional; recorded, not enforced
include "stdgates.inc";    // accepted and recorded, never loaded
```

The standard gate names are built in (see [`gate_mapping.md`](gate_mapping.md)),
so the include is honoured by having the names already available rather than
by reading the file.

### Declarations

```qasm
qubit[4] q;    bit[4] c;     // sized registers
qubit anc;     bit flag;     // single, unindexed
```

Registers are allocated into QC-IR's flat, dense index space **in declaration
order**. QC-IR has no named sub-registers, so the frontend keeps the name →
range mapping itself; `q[2]` in the second declared register resolves to
whatever flat index that is.

Re-declaring a name is a semantic error.

### Parameters

```qasm
input float[64] theta;
input angle theta2;
```

An `input` declaration introduces a **symbolic** parameter (`Param::Symbol`).
Only `float` and `angle` are accepted. Symbolic circuits are valid QC-IR; they
must pass through `bind_parameters` before lowering (see
[`ir_spec.md`](ir_spec.md)).

### Gate calls

```qasm
h q[0];                 // no parameters
rz(pi/2) q[0];          // parameterized
cx q[0], q[1];          // multi-qubit
ry(theta) q[0];         // symbolic parameter
```

**Broadcast** over whole registers is supported in these shapes:

| Form | Meaning |
|---|---|
| every operand indexed | one application |
| one whole register, single-qubit gate | one application per element |
| all operands whole registers of equal width `n` | `n` applications, element-wise |

Mixing indexed and whole-register operands in one call (`cx q[0], r;`) is
**refused** rather than guessed at, and registers of differing widths cannot be
broadcast together.

### Measurement and reset

```qasm
measure q[0] -> c[0];   // arrow form
c[0] = measure q[0];    // assignment form
c = measure q;          // whole-register, element-wise
reset q[0];
```

Both measurement spellings produce identical IR. Broadcast measurement
requires equal widths on both sides.

### Parameter expressions

Numeric literals, the constants `pi`/`π`, `tau`/`τ` and `euler`, unary `-`, and
binary `+ - * /` with standard precedence and parentheses. These are
**constant-folded at parse time** to a concrete angle, so `rz(pi/2)` emits bit
for bit what `rz(1.5707963267948966)` emits.

### Comments

Line (`//`) and block (`/* … */`).

## Not supported

Each of these is refused with `FrontendError::Unsupported` naming the
construct:

| Construct | Why |
|---|---|
| `if`, `else`, `while`, `for` | Dynamic/classical control flow. Stage F defers it explicitly; QC-IR has no representation for a conditional operation. |
| `def`, `defcal`, `gate … { }`, `extern` | Subroutines and user-defined gates. Would require an inlining or opaque-expansion design that does not exist yet. |
| `output` declarations | No corresponding QC-IR concept. |
| `barrier`, `delay`, `box` | Scheduling annotations; QC-IR has no barrier, and silently dropping one could mislead a later scheduling pass. |
| `array` declarations, non-`float`/`angle` `input` | Outside the static, parameter-only data model of Stage F. |
| `qreg` / `creg` | OpenQASM **2** syntax. Refused with a message pointing at `qubit[n]` / `bit[n]`. |
| Compound expressions over a symbolic parameter (`2*theta`) | QC-IR represents a *symbol*, not a symbolic expression. Folding one would silently discard the arithmetic. |

Unrecognised **gate names** are not in this list: they map to
`GateKind::Opaque` (see [`gate_mapping.md`](gate_mapping.md)).

## Diagnostics

| Variant | Raised for |
|---|---|
| `FrontendError::Syntax { message, line, column }` | Malformed source. Carries a 1-based position. |
| `FrontendError::Semantic` | Undeclared register, out-of-range index, duplicate declaration, mismatched broadcast widths. |
| `FrontendError::Unsupported` | Any construct in the table above. |
| `FrontendError::ParamArity` | Wrong number of gate parameters (e.g. `rz()`). |
| `FrontendError::Ir` | A QC-IR invariant the built circuit violates. |

## Guarantee

The frontend is a **mapping onto the existing IR**, never a second definition
of it. `tests/openqasm_pipeline.rs` asserts this the strict way: a parsed Bell
state, GHZ-3, and folded-angle rotation must emit **byte-identical** QIR to the
equivalent hand-built `CircuitBuilder` circuit. If a future change made the
frontend reshape semantics, those equivalence tests fail.
