# ADR: The IR Contract (C1–C8)

**Status:** Accepted, 2026-09-30. **Applies to:** the Rust IR type design
(`src/ir/`) and every stage that consumes it.
**Supersedes:** the binding-constraint table of
[`architecture_decision_mlir_phase2.md`](architecture_decision_mlir_phase2.md),
which now refers here.

## Context

The constraints C1–C8 were first written to keep a future MLIR port mechanical.
Under the project framing
([`core_architecture/framing-and-research-question.md`](core_architecture/framing-and-research-question.md)),
their primary job is different: they are what makes OQCI's IR *strongly
validated*. Every circuit that exists has passed one total check, and every
later stage — passes, lowering, QIR emission — can rely on that check instead
of repeating it.

MLIR portability is still a benefit of the same constraints, and is recorded
as such below, but it is no longer the reason they exist.

## The guarantee

A `Circuit` value is a witness that invariants **I1–I8**
([`ir_spec.md`](ir_spec.md) §3.3) hold. Construction is the only way to obtain
one, validation is total, and a violation is reported as a typed `IrError`,
never a panic. C1–C8 are the design constraints that keep this guarantee true
and checkable.

## Constraints

Changing any of these requires an explicit architecture decision that amends
this ADR, together with the corresponding specification and test updates.

| # | Constraint | What it guarantees | Where it is enforced / tested |
|---|---|---|---|
| C1 | **`GateKind` is a closed enum plus a single `Opaque` escape hatch.** | Every registered gate has known semantics, so matching over gates is exhaustive: a new variant cannot be added without every pass, rule table, lowering step and emitter being made to handle it. `Opaque` is the one clearly marked path for gates OQCI cannot reason about, and every consumer treats it as inert. Only an explicit ADR adds a variant ([`architecture_decision_sx_basis_gate.md`](architecture_decision_sx_basis_gate.md) is the one so far). | `src/ir/types.rs`; exhaustive `match`es across `src/`; `the_simulator_supports_every_registered_gate` (`src/target/builtin.rs`) |
| C2 | **`GateKind` carries parameters only; qubit operands live on the instruction/node.** | Parameters and operands are validated by separate rules (I7/I8 vs I1/I3/I4) and never entangled. | `Instruction::Gate { kind, qubits }` (`src/ir/qc.rs`) |
| C3 | **`QubitId` and `ClbitId` are distinct newtypes.** | Using a classical bit as a qubit, or the reverse, is a compile error rather than a runtime check. I1 and I2 range-check each against its own register. | `src/ir/types.rs`; `tests/validation.rs` (I1, I2) |
| C4 | **`Angle` is the dedicated concrete parameter type, and gate parameters are `Param` = `Concrete(Angle) \| Symbol(String)`. Neither reverts to a bare `f64`.** | A concrete angle and an unbound symbol are distinguishable everywhere (Stage F §2), so no stage can silently evaluate or invent a symbolic value. Finiteness (I7) and non-empty symbol names (I8) are checked on exactly one type. | `src/ir/param.rs`; `tests/validation.rs` (I7, I8); `bind_parameters` re-validates (`src/ir/bind.rs`) |
| C5 | **All construction goes through `CircuitBuilder`; there is no public struct mutation.** | There is exactly one place a circuit can come into existence, so there is exactly one place invariants are checked. Every stage that produces a new circuit — a pass, parameter binding, target lowering, a frontend — builds it through the builder and is validated again. | `Circuit`'s fields are private and it has no `Deserialize` (`src/ir/qc.rs`) |
| C6 | **Validation is centralized and total over I1–I8, with no panics.** | Malformed input from any frontend or API caller yields a typed `IrError`. This also binds any future in-compiler *semantic* validator: it must report an outcome — including "inconclusive" — rather than panic. | `CircuitBuilder::build` (`src/ir/qc.rs`); `tests/validation.rs`; `tests/error_model.rs` |
| C7 | **QCO-IR is a wire-threaded, value-semantics DAG with explicit `Input`/`Output` boundaries and `Data`/`Control` edge kinds.** | Which operations may be reordered is explicit: a `Control` edge out of a measurement or reset is a barrier no rewrite crosses. The conversion is deterministic and semantics-preserving ([`ir_spec.md`](ir_spec.md) §5). | `src/ir/qco.rs`, `src/ir/convert.rs`; `tests/qco_structure.rs`, `tests/roundtrip.rs` |
| C8 | **`src/ir/mlir_compat.rs` is the sole MLIR seam; the rest of `ir/` stays MLIR-free.** | The IR's semantics cannot come to depend on external compiler infrastructure. | `src/ir/mlir_compat.rs` contains no code; no MLIR dependency exists |

## MLIR portability, as a secondary benefit

The same constraints keep a future MLIR port mechanical
([`mlir_dialect.md`](mlir_dialect.md),
[`architecture_decision_mlir_phase2.md`](architecture_decision_mlir_phase2.md)):

- **C1**: registered variants map to `quantum.gate` with a registered `gate`
  mnemonic; `Opaque` maps to `quantum.gate` with an unregistered mnemonic
  (`mlir_dialect.md` §4.1).
- **C2**: parameters become attributes, qubits become SSA operands
  (`mlir_dialect.md` §3).
- **C3**: the two newtypes become `!quantum.qubit` and `!quantum.result`.
- **C4**: a concrete parameter becomes a `FloatAttr`; a symbolic parameter's
  mapping is specified in `mlir_dialect.md` §2.
- **C5, C6**: the builder mirrors `OpBuilder`, and each invariant becomes an
  op `verify()` rule.
- **C7**: on qubit wires, the DAG is isomorphic to SSA use-def chains, and
  `Control` edges become the `Collapsing` trait. Classical-bit wires have no
  mapping yet (`mlir_dialect.md` §6).
- **C8**: confines the marshalling to one module.

## Consequences

- **Positive:** every stage after construction can trust its input, which is
  what makes the passes, lowering and QIR emission checkable in isolation; and
  the constraints have a justification that holds whether or not MLIR is ever
  adopted.
- **Negative / accepted cost:** the constraints are a standing discipline.
  This ADR must be consulted before changing a core IR type, and a gate that
  OQCI cannot write a matrix for must stay `Opaque`.

## Related

- [`ir_spec.md`](ir_spec.md) — the invariants I1–I8 and the DAG model.
- [`architecture_decision_no_frontend.md`](architecture_decision_no_frontend.md)
  — why frontends map onto the IR and never reshape it.
- [`architecture_decision_sx_basis_gate.md`](architecture_decision_sx_basis_gate.md)
  — the one recorded amendment to the gate set under C1.
