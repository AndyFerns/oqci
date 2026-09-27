# CUDA-Q Frontend — Supported Subset

Status: normative
Implemented by: `src/frontend/cudaq/`
Verified by: unit tests in `src/frontend/cudaq/`, `tests/cudaq_frontend.rs`,
`tests/cli.rs`, `python/tests/test_frontends.py`
Entry points: `parse_cudaq`, `parse_cudaq_named`, `Frontend::CudaQ`

This document defines **exactly** what the CUDA-Q frontend accepts.
`final-deliverables-spec.md` §5.5 asks that a CUDA-Q adapter "map into the same
QC-IR contract" as every other frontend. This one does: it builds the same
`Circuit` the OpenQASM and Qiskit frontends build, through the same
`CircuitBuilder`, with gate names resolved by the same shared table
([`gate_mapping.md`](gate_mapping.md)).

It does **not** implement CUDA-Q. It reads a documented subset of CUDA-Q's
Python kernel syntax, and refuses everything else with a diagnostic naming
the construct and its line.

## What it reads

A CUDA-Q program is a Python module. The quantum program is the one function
decorated `@cudaq.kernel` (or `@cudaq.kernel()`). Everything else in the file
(imports, `cudaq.sample(...)`, `if __name__ == "__main__":` blocks, argument
parsing) is **host code**. It runs on a classical machine and is not part of
any circuit.

The frontend cuts the kernel out of the file by line and indentation *before*
tokenizing anything. Host code is therefore never parsed, and it can be
arbitrary Python: f-strings, dicts, `**`, anything. A file that runs under
CUDA-Q itself compiles here unchanged; see
[`../examples/ghz3_cudaq.py`](../examples/ghz3_cudaq.py).

It reads **source text**, the way the OpenQASM frontend does. It does not
import or introspect a live kernel object. The consequences:

- neither Python nor CUDA-Q is needed to compile a kernel;
- the frontend is deterministic and tested by ordinary `cargo test`;
- a kernel reaches the whole pipeline (optimization, lowering, execution
  preparation, `oqci` CLI commands and the visualizer) through
  `Frontend::CudaQ`.

The CLI and the visualization server select this frontend for any file ending
in `.py`. From Python, pass `frontend="cudaq"` to `oqci.compile`.

## Pipeline

```text
file → extract kernel → lexer → parser → translate → CircuitBuilder → Circuit
       (by indentation)  (Python   (flat      (flat qubit   (validation)
                          lines)    grammar)   space)
```

As in the OpenQASM frontend, the translator performs **no** validation QC-IR
already performs. Duplicate operands, and non-finite angles such as `1 / 0`,
surface from `CircuitBuilder::build` as `FrontendError::Ir`.

## Where the subset comes from

§33.4 forbids implementing a vendor's syntax from memory, so every accepted
operation below is one NVIDIA's CUDA-Q documentation shows in **Python**
kernel syntax. These pages were used, all "latest", retrieved 2026-09-26:

- `specification/cudaq/kernels.html`: kernel declaration, `cudaq.qvector`,
  `cudaq.qubit`, and argument types;
- `api/default_ops.html`: the operation set, argument order ("the gate
  parameters come first"), `.adj`, `.ctrl` including the list form for
  multiple controls, and broadcasting of single-qubit operations over a
  register;
- `using/examples/quantum_operations.html`: the worked examples, including
  per-qubit `mz(qvector[0]); mz(qvector[1])`.

Some operations CUDA-Q very likely supports are nonetheless **refused**,
because those pages do not show their Python spelling: `reset`, bare
`sdg`/`tdg`, and variadic controls such as `x.ctrl(c0, c1, t)`. Widening the
subset means checking the pages again (or pinning a CUDA-Q release and testing
against it), not guessing.

## Supported

### Kernel and arguments

```python
@cudaq.kernel
def ansatz(theta: float, phi: float):
    ...
```

- Exactly one `@cudaq.kernel` function per file.
- Arguments must be annotated `float`. Each one becomes a **symbolic angle**
  (`Param::Symbol`), bound before execution exactly as an OpenQASM `input` is
  (`--bind theta=0.5` on the CLI, `bindings=` from Python).
- A return annotation (`-> None`) is allowed and ignored.

### Allocation

| Statement | Allocates |
|---|---|
| `q = cudaq.qvector(N)` | `N` qubits; `N` must fold to a positive integer |
| `a = cudaq.qubit()` | one qubit |

Every allocation is laid out on one flat qubit space in allocation order, as
OpenQASM registers are.

### Operations

| Source | QC-IR |
|---|---|
| `h(q)`, `x(q)`, `y(q)`, `z(q)`, `s(q)`, `t(q)` | `H`, `X`, `Y`, `Z`, `S`, `T` |
| `s.adj(q)`, `t.adj(q)` | `Sdg`, `Tdg` |
| `h.adj`, `x.adj`, `y.adj`, `z.adj` | the gate itself (each is its own adjoint) |
| `rx(θ, q)`, `ry(θ, q)`, `rz(θ, q)` | `Rx`, `Ry`, `Rz` |
| `r1(λ, q)` | `P` (`r1` is a spelling of the phase gate) |
| `rx.adj(θ, q)` and the other rotations | the rotation by `−θ`. **Concrete angles only**: negating a symbol is arithmetic QC-IR cannot represent |
| `u3(θ, φ, λ, q)` | `U` |
| `swap(a, b)` | `Swap` |
| `x.ctrl(c, t)` | `Cx` |
| `x.ctrl([c0, c1], t)` | `Ccx` |
| `y.ctrl(c, t)`, `z.ctrl(c, t)` | `Cy`, `Cz` |
| `mz(q)` | `Measure` |

A qubit operand is `q[i]` with a constant, non-negative integer index, or the
name of a `cudaq.qubit()`. A **single-qubit** operation, and `mz`, may also
take a whole register, which applies it to every qubit (CUDA-Q's documented
broadcast). Controls and two-qubit operands must be single qubits.

### Measurement destinations

**Classical bit `i` receives qubit `i`.** Every `mz` produces its own
`Measure { qubit, target }` with an explicit destination. The circuit has as
many classical bits as qubits whenever it measures anything, and none
otherwise.

So `mz(q[0]); mz(q[1]); mz(q[2])` and `mz(q)` compile to the same three
measurements. `tests/cudaq_frontend.rs` pins both to byte-identical QIR with
the OpenQASM `c = measure q;`.

### Angle expressions

Numeric literals, `math.pi` / `numpy.pi` / `np.pi`, unary `+`/`-`, `+ - * /`,
and parentheses, folded to a number at compile time. A bare `float` argument
is a symbol; **any arithmetic on one** (`theta / 2`, `-theta`) is refused.
The OpenQASM frontend applies the same rule to compound `input` expressions.

### Also accepted

- Comments, blank lines, a docstring, and `pass`.
- Statements split across lines inside brackets, or with a trailing `\`.
- `;` between statements.

## Not supported

Each of these is refused by name, never skipped:

| Construct | Why |
|---|---|
| `for`, `while`, `if`/`elif`/`else`, `match` | Static circuits only (Stage F). Unroll loops by hand. |
| `q[i]` with a non-constant `i` | Needs a loop or runtime value. |
| Negative indices, slices (`q[0:2]`) | Not in the verified subset. |
| Classical variables (`angle = 0.5`) | Write the angle inline. |
| `b = mz(q)` | A measurement result feeds classical logic. Call `mz` as a statement. |
| `mx`, `my` | Only computational-basis measurement. |
| `reset`, bare `sdg`/`tdg` | Not shown in the documentation the subset is verified against (write `s.adj` / `t.adj`). |
| Controlled gates other than `x` (1–2 controls), `y`, `z` (1 control), e.g. `rx.ctrl`, `h.ctrl` | QC-IR has no such gate. |
| Calling another kernel or any unknown function | Kernel composition is out of scope. |
| Non-`float` arguments (`int`, `list[float]`, `cudaq.qview`, …) | Only symbolic angles are representable. |
| More than one `@cudaq.kernel` in a file, a decorator with arguments, stacked decorators | Ambiguous entry point. |
| `return`, nested `def`, `lambda`, `with`, `try`, imports inside the kernel | Outside the subset. |
| Keyword arguments, `**`, `%`, comparisons | Positional calls and `+ - * /` only. |

## Diagnostics

Syntax errors carry a line and column in the original file. Unsupported
constructs and semantic errors name the construct and end with its line:

```text
error: unsupported construct: loops (`for`) are outside the supported subset — unroll them; OQCI compiles static circuits only (line 5)
```

## Running on CUDA-Q, and the old `cudaq-adapter/`

This frontend is the *reading* half of OQCI's CUDA-Q support. The *running*
half is `oqci.backends.cudaq`, an execution adapter that turns a prepared
`Executable` back into a kernel written in this same subset and samples it.
Because it writes this subset, its output is tested by feeding it back
through this frontend.

Both halves replace the root-level `cudaq-adapter/` prototype, which
carried its own private IR. That IR could not be optimized, lowered or
visualized. Its frontend also dropped all but the last of several per-qubit
`mz` calls, and it accepted non-finite angles. See
[`adapters.md`](adapters.md) for the execution adapter, the layout every
adapter follows, and where each prototype file went.
