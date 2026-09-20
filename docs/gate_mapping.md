# Gate Mapping — Source Names → `GateKind`

Status: normative for the OpenQASM 3 frontend and the Qiskit adapter
Implemented by: `src/frontend/gate_map.rs`

Both frontends resolve gate names through **one** table, so they agree on what
a mnemonic means by construction rather than by coincidence. OpenQASM 3's
`stdgates.inc` names and Qiskit's `Instruction.name` strings coincide across
the entire registered set, which is what makes a single table possible.

Matching is **case-insensitive**: OpenQASM spells the built-in controlled-X as
`CX`, Qiskit as `cx`.

## Registered gates

| Source name(s) | `GateKind` | Params | Qubits |
|---|---|---|---|
| `id`, `i` | `I` | 0 | 1 |
| `x` | `X` | 0 | 1 |
| `y` | `Y` | 0 | 1 |
| `z` | `Z` | 0 | 1 |
| `h` | `H` | 0 | 1 |
| `s` | `S` | 0 | 1 |
| `sdg` | `Sdg` | 0 | 1 |
| `t` | `T` | 0 | 1 |
| `tdg` | `Tdg` | 0 | 1 |
| `sx` | `SX` | 0 | 1 |
| `sxdg` | `SXdg` | 0 | 1 |
| `rx` | `Rx(θ)` | 1 | 1 |
| `ry` | `Ry(θ)` | 1 | 1 |
| `rz` | `Rz(θ)` | 1 | 1 |
| `p`, `u1`, `phase` | `P(λ)` | 1 | 1 |
| `u`, `u3` | `U { θ, φ, λ }` | 3 | 1 |
| `u2` | `U { π/2, φ, λ }` | 2 | 1 |
| `cx`, `cnot` | `Cx` | 0 | 2 |
| `cy` | `Cy` | 0 | 2 |
| `cz` | `Cz` | 0 | 2 |
| `swap` | `Swap` | 0 | 2 |
| `ccx`, `toffoli` | `Ccx` | 0 | 3 |

Three mappings deserve their justification recorded:

- **`u1(λ) → P(λ)`.** The legacy `u1` gate is `diag(1, e^{iλ})`, which is
  exactly the phase gate. They are the same operator, not an approximation.
- **`u2(φ, λ) → U(π/2, φ, λ)`.** This is `u2`'s definition, so the frontend
  supplies the fixed θ rather than inventing a separate variant.
- **`sx`/`sxdg` → `SX`/`SXdg`.** These names previously matched nothing in the
  table and fell through to the `Opaque` escape hatch below. They now resolve to
  registered variants, so they are optimizable and simulable like any other
  registered gate. The IR change and its rationale are recorded in
  [`architecture_decision_sx_basis_gate.md`](architecture_decision_sx_basis_gate.md);
  both frontends pick the change up automatically, since Qiskit already spells
  them `sx`/`sxdg`.

Qubit-operand counts in the last column are *informational*: arity is enforced
by QC-IR validation ([`IrError::GateArityMismatch`]), not by this table.
Parameter counts **are** enforced here, as `FrontendError::ParamArity`.

## Everything else: the opaque escape hatch

A name not in the table is **not an error**. It becomes:

```rust
GateKind::Opaque { name, params }
```

This is deliberate, and it is the rule set out in Stage A §4: `GateKind`
remains a closed enum plus one escape hatch, and **a frontend must never grow
the enum to accommodate a source language**. A gate common enough to deserve a
registered variant gets one through an explicit IR change with its own
rationale — never as a side effect of parser convenience.

Opaque gates lower to declared `__quantum__qis__<name>__body` externs; see
[`qir_lowering.md`](qir_lowering.md) for what that does and does not promise.

## Parameters

Every parameter slot accepts either form of [`Param`](ir_spec.md):

- a concrete angle, in radians;
- a symbolic parameter, which must be bound before lowering.

How each frontend produces a symbol differs — OpenQASM from an `input`
declaration, Qiskit from an unbound `Parameter` — but both land on
`Param::Symbol`, and both refuse compound expressions over one. See
[`openqasm_frontend.md`](openqasm_frontend.md) and
[`qiskit_adapter.md`](qiskit_adapter.md).
