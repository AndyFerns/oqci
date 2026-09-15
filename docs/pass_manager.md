# Pass Manager and Optimization Passes

Status: normative
Implemented by: `src/pass/`, `src/analysis/`
Verified by: `tests/pass_equivalence.rs`

## The pass manager

`final-deliverables-spec.md` §7 requires explicit registration and ordering,
enable/disable capability, deterministic execution, per-pass metadata,
before/after hooks, error propagation, and ablation support.

```rust
pub trait Pass: Send + Sync {
    fn id(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn run(&self, circuit: &Circuit) -> Result<PassOutput, PassError>;
}
```

A pass is a `Circuit → Circuit` function. Passes that need dependency
structure call `qc_to_qco` internally and replay the result through
`CircuitBuilder`.

That replay is not incidental. Because every pass's output passes through
`CircuitBuilder::build`, **a pass cannot emit a circuit that violates a QC-IR
invariant** — it gets an `IrError` instead, reported as `PassError::Ir` naming
the pass. A buggy pass fails loudly rather than corrupting the IR for
everything downstream.

`QcoCircuit`'s mutation methods stay crate-internal (Stage A §4: "construction
remains centralized"), so this design also avoids widening that hole for
passes.

### Selection and ablation

```rust
PassSelection::All
PassSelection::only(["gate-cancellation"])
PassSelection::all_except(["rotation-merge"])   // an ablation run
```

A disabled pass still produces a `PassRecord` with `enabled: false`, so an
ablation report shows what was deliberately left out rather than leaving a
silent gap.

### Metadata

Every pass produces a `PassRecord`: before/after `ResourceReport`, whether it
changed anything, how long it took, and its own notes. The before/after
metrics come from [`analysis::analyze`](#analysis) — the same function
`oqci analyze` calls — so the compiler's account of a circuit and the tool's
cannot disagree.

## The default pipeline

```text
canonicalize → gate-cancellation → rotation-merge → canonicalize → schedule
```

The ordering is deliberate:

- `canonicalize` first, so later passes see a normalized circuit. An explicit
  identity gate between two `H`s would otherwise hide a cancellable pair.
- `canonicalize` **again** after `rotation-merge`, because merging
  `Rz(a); Rz(-a)` produces `Rz(0)`, which canonicalization already removes
  exactly.
- `schedule` last, and read-only: it measures what actually came out.

This is a fixed, finite order rather than iteration to convergence. §7 asks
that ordering be treated as meaningful, and a finite pipeline cannot fail to
terminate. (Individual peephole passes *do* iterate internally to a fixed
point — each round strictly removes instructions, so they terminate too.)

## Adjacency: what "adjacent" means

Both peephole passes ask the same question, answered once in
`src/pass/adjacency.rs`: is operation B the immediate successor of A **on
every wire A touches**?

Program-order adjacency would be wrong in both directions:

| Circuit | Program order says | QCO-IR says | Correct? |
|---|---|---|---|
| `H q0; H q1; H q0` | not adjacent | adjacent — nothing touches `q0` between them | cancel ✓ |
| `H q0; X q0; H q0` | not adjacent | not adjacent | keep ✓ |
| `X q0; measure q0; X q0` | not adjacent | not adjacent — the measurement is a node on the wire | keep ✓ |

The measurement barrier needs no special case: a collapsing operation is
itself a node on the wire, so it breaks the unanimity. This is how §33.14
("do not optimize away operations across measurement/reset/control barriers
without proving the transformation safe") is satisfied mechanically rather
than by care.

## Pass reference

### `canonicalize` — remove provable no-ops

Removes exactly two things:

- `GateKind::I` — the identity matrix, literally.
- `Rx`/`Ry`/`Rz`/`P` with a **concrete** parameter of exactly `0.0`.

**Not** implemented, deliberately: rewrites like `P(π) → Z` or
`U(0,0,λ) → P(λ)`. Those hold only up to a global phase this IR does not
track (there is no `gphase` operation), so applying them would quietly change
what a *controlled* version of the circuit means.

The zero-angle rule requires exact `0.0`, not "close to zero" — deciding how
much floating-point error is acceptable in someone else's circuit is not a
compiler pass's call.

### `gate-cancellation` — remove adjacent inverse pairs

| Gates | Rule |
|---|---|
| `X Y Z H Cx Cy Cz Swap Ccx` | self-inverse |
| `S`↔`Sdg`, `T`↔`Tdg` | mutual inverses |
| `Rx Ry Rz P` | inverse iff both parameters concrete and exactly negating |
| `U`, `Opaque`, `I` | **never cancelled** |

Operand lists must match **in order**: `Cx q0,q1` is not the inverse of
`Cx q1,q0`, since that exchanges control and target.

Why the exclusions:

- **`U`** — its inverse is `U(-θ, -λ, -φ)`, note the swapped φ/λ. That is
  exactly the kind of rule that is easy to get subtly wrong and impossible to
  notice afterwards. It stays out until `tests/pass_equivalence.rs`
  demonstrates it across random angles.
- **`Opaque`** — OQCI does not know what the gate does, so it cannot know
  that doing it twice does nothing.
- **Rotations** cancel only on exact negation. `θ` and `2π − θ` describe the
  same rotation but are not recognised, and no epsilon is applied.

### `rotation-merge` — combine same-axis rotations

`Rz(a); Rz(b) → Rz(a+b)`, likewise `Rx`, `Ry`, and the phase gate `P`.

Merged **only when both parameters are concrete**. Merging `Rz(θ); Rz(φ)`
would need a parameter meaning "θ + φ", and `Param` represents a symbol, not
an expression (`ir_spec.md` §1.1) — folding them would silently lose a term.
Stage F §6 requires the same from the other direction: passes must preserve
parameter semantics and must not evaluate symbolic parameters prematurely.

**This pass is the whole of OQCI's "gate fusion".** §8.3 (Gate Fusion) and
§8.4 (Rotation Merging) are listed separately in the spec; this single pass
implements the additive-parameter fusion both describe, and nothing beyond
it. There is no fusion of arbitrary consecutive gates into a synthesized
unitary, no two-qubit block collapsing, no Euler-angle recombination of `U`
gates. That is §8.3's own instruction applied honestly — "Do not claim
arbitrary unitary synthesis unless it is actually implemented" — not an
oversight.

### `schedule` — report parallelism

Computes the ASAP layering (`QcoCircuit::layers`) and reports depth and
maximum parallel width. It **never reorders anything**.

§8.5 warns against reordering operations merely because they touch different
qubits. Reordering only becomes meaningful once a target's constraints make
one schedule better than another — Stage D/Phase 3 work. Until then, a
reordering pass would be choosing between schedules on no evidence.

## Analysis

`src/analysis/` is the single source of truth for measurement:

- `analyze(&Circuit) -> ResourceReport` — gate counts (total, one-qubit,
  multi-qubit, per-mnemonic), measurement and reset counts, register widths,
  depth, max parallel width. Satisfies §13.1–13.3 for the operations
  currently modeled.
- `diff_circuits(&before, &after) -> CircuitDiff` — an LCS alignment of two
  instruction lists, computed by comparing the circuits rather than by
  trusting a pass's own account of itself. `O(n·m)`, which is fine at the
  sizes this project targets.

Target-native gate counts and routing overhead are also named in §13 but need
a target profile that does not exist yet. They are **absent** rather than
reported as zero.

## Correctness verification

`tests/pass_equivalence.rs` is what makes these rewrites trustworthy. It runs
`proptest`-generated circuits through each pass and asserts the state vector
is unchanged **up to global phase** (`|⟨ψ_before|ψ_after⟩| ≈ 1`).

Semantics come from an independent dense-matrix simulator
(`tests/support/statevector.rs`, test-only, `num-complex` as a
dev-dependency), *not* from the tables the passes consult — so a mis-signed
angle in a rewrite rule cannot hide behind a matching mistake in the check.

The harness has been verified to fail when a rule is broken: inverting the
sign in `rotation-merge`'s angle addition makes four of its tests fail,
including the randomized property. A property test that cannot fail proves
nothing.

## Not implemented

Per `final-deliverables-spec.md`'s Critical Rule — a feature is not
implemented merely because a module or diagram names it — the following from
§8 are **absent**, pending the target model (Stage D):

| Spec | Status |
|---|---|
| §8.6 Qubit mapping | not implemented — needs a target topology |
| §8.7 Routing / SWAP insertion | not implemented — needs connectivity data |
| §8.8 Basis decomposition | not implemented — needs a basis profile |
| §8.3 general gate fusion | only additive-parameter fusion, as described above |

Also absent: a fixed-point pass scheduler (the pipeline order is fixed and
finite), pass plugins (§18.2), and any target-aware scheduling.
