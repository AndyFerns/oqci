# ADR — Adding `SX` / `SXdg` to the Registered Gate Set

Status: **ACCEPTED**
Date: 2026-09-20
Supersedes: nothing. Amends the closed-gate-set invariant in
[`core_architecture/stage-a-rust-native-ir-foundation.md`](core_architecture/stage-a-rust-native-ir-foundation.md) §4.

## Context

Stage A §4 binds `GateKind` as "a closed enum plus one `Opaque` escape hatch
**until an explicit architecture decision changes this**", and
`final-deliverables-spec.md` §33.5 forbids silently expanding the supported gate
set. This document is that explicit decision; it exists so the change is a
recorded choice rather than a convenience taken during implementation.

The forcing issue is the target model. Stage D and Stage E both name IBM-style
gate-model NISQ hardware as the initial research target, and that family's native
one-qubit basis is `{rz, sx, x}` — the √X gate is not incidental to it, it is one
of the two pulses the hardware actually implements. OQCI's registered set had no
way to name it.

## The problem with the alternative

The cheaper option was to leave `GateKind` closed and let basis decomposition
emit `Opaque { name: "sx" }`. That needs no IR change, and it was the right
answer while nothing consumed it. It stops being the right answer once
decomposition actually targets an IBM-style basis, because `Opaque` is
deliberately inert throughout the compiler:

- **Optimization passes refuse to touch it.** `gate-cancellation` excludes
  `Opaque` because OQCI cannot know what an unknown gate does, so it cannot know
  that doing it twice does nothing (see [`pass_manager.md`](pass_manager.md)).
  A circuit decomposed into the IBM basis would therefore arrive at the
  optimizer *already opaque* — the passes would be structurally unable to
  optimize the very circuits the research targets.
- **QIR lowering emits it as an extended intrinsic.** Fine for a genuinely
  unknown gate; wrong for one whose semantics are exactly known.
- **The state-vector equivalence harness cannot simulate it**, so any
  decomposition producing `sx` would fall outside the property-based correctness
  check that makes the passes trustworthy.

In short, routing `sx` through `Opaque` would have bought a smaller diff today
at the cost of making the target-lowering work unverifiable and unoptimizable.

## Decision

Add two variants to `GateKind`:

| Variant | Mnemonic | Matrix | Arity | Params |
|---|---|---|---|---|
| `SX` | `sx` | `½·[[1+i, 1−i], [1−i, 1+i]]` | 1 | 0 |
| `SXdg` | `sxdg` | `½·[[1−i, 1+i], [1+i, 1−i]]` | 1 | 0 |

`SX` is the principal square root of `X`: `SX · SX = X` exactly. `SXdg` is its
adjoint, so `SX · SXdg = SXdg · SX = I` exactly — an exact identity, not an
up-to-phase one.

Equivalently `SX = e^{iπ/4}·Rx(π/2)`, which is *why* it gets its own variant
rather than being folded into `Rx`: the two differ by a global phase, and
`Rx(π/2)` is not what the hardware calls native.

### Consequences across the codebase

- **Gate set.** `GateKind` gains two no-parameter, arity-1 variants. It remains
  closed; `Opaque` remains the only escape hatch. This is the first and so far
  only amendment to the Stage A §4 invariant.
- **Frontends.** `sx` and `sxdg` map through the shared table in
  [`gate_mapping.md`](gate_mapping.md), so both OpenQASM 3 and Qiskit resolve
  them identically — Qiskit names them `sx`/`sxdg` already.
- **Cancellation.** `SX`↔`SXdg` join the mutual-inverse table alongside
  `S`↔`Sdg` and `T`↔`Tdg`. Note that `SX` is **not** self-inverse: `SX; SX` is
  `X`, not the identity, so it must not be added to the involution list.
- **QIR.** `sx` is outside the QIR standard instruction set, so it lowers to a
  declared extended intrinsic `__quantum__qis__sx__body`, exactly as `p`, `u`,
  `cy`, `swap` and `ccx` already do. See [`qir_lowering.md`](qir_lowering.md).
- **Verification.** Both gates get matrices in the test-only state-vector
  simulator and enter the `proptest` generator, so the cancellation rule is
  covered by the same equivalence property as every other rewrite.
- **MLIR.** Two more registered ops in the eventual `quantum` dialect; the
  op↔Rust correspondence stays mechanical.

## What this decision does *not* license

It authorizes exactly these two variants. It is **not** a general licence to add
a gate whenever a backend names one. The bar remains: a new variant needs a
recorded decision explaining why `Opaque` is insufficient, and the gate's
semantics must be exact enough to write its matrix down and verify it. A gate
OQCI cannot simulate cannot be optimized safely, and belongs in `Opaque`.

## Alternatives rejected

| Option | Why not |
|---|---|
| Keep `sx` as `Opaque` | Blocks optimization and equivalence-checking of exactly the circuits the IBM-target research depends on (above). |
| Rewrite `sx` → `Rx(π/2)` during decomposition | Differs by a global phase, so a *controlled* version of the circuit would mean something different. OQCI tracks no global phase (there is no `gphase` op), so the rewrite is not sound in general. |
| Open `GateKind` to arbitrary named gates | Discards exhaustive matching across every pass and the QIR lowering table — the reason the closed-enum-plus-`Opaque` model was chosen in Stage A. |
