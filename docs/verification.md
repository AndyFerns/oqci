# Verification and lint

This page covers two analyses that run inside the compiler:

- the **semantic checker** ([`src/verify/`](../src/verify/)) proves, or fails
  to prove, that each transformation preserved what the circuit computes;
- the **lint boundary** ([`src/analysis/lint.rs`](../src/analysis/lint.rs))
  reports patterns worth a reader's attention.

Neither one ever changes a circuit.

## Semantic verification (Stage G §3.1, "semantic, in-compiler")

### Turning it on

| Surface | How |
|---|---|
| CLI | `--verify` |
| Python | `oqci.compile(..., verify=True)` |
| Rust | `CompilerConfig::verify` |

It is off by default, because its cost is exponential in the width it checks.

When on, compilation checks three things:

- **binding**, by exact substitution;
- **every pass that ran**, extension passes included;
- **target lowering**, modulo initial and final layout.

### Verdicts

| Verdict | Meaning |
|---|---|
| `equivalent` (`identical`) | the pass returned the same instruction list |
| `equivalent` (`substitution`) | binding replaced exactly the bound symbols with exactly the bound values |
| `equivalent` (`operator`) | the two unitaries agree on every input, up to one global phase (the summed trace, modulus taken last) |
| `equivalent` (`initial_state`) | they agree on `\|0…0⟩`, the only state a program starts in |
| `equivalent` (`distribution`) | the distributions over the measured classical bits agree |
| `not_equivalent` | none of the above: an observable difference |
| `inconclusive` | not decided, with a typed reason (see below) |

The reasons for `inconclusive` are:

- `too_many_qubits`: the circuit is wider than `--verify-max-qubits`, which
  defaults to 8;
- `symbolic_parameter`;
- `opaque_gate`;
- `non_terminal_measurement`;
- `reset` of a used qubit.

A `not_equivalent` verdict stops compilation with
`CompileError::SemanticMismatch`, so no executable is produced.
`--verify-report-only` records the verdict and continues instead; that is for
investigating a failure, never for producing results.

An `inconclusive` verdict never fails compilation and is never counted as a
pass. The counts in the report give Stage G's inconclusive rate. QIR emission
is listed as `not checked`: nothing in the toolchain executes OQCI's QIR.

### How the comparison works

The comparison is exhaustive over the computational basis. It uses no random
input states, so it needs no seed. Lowering is compared modulo layout:

- the lowered circuit runs with each logical qubit placed on its initial
  physical qubit;
- it must reproduce the original's output read from the final physical qubits;
- every other physical qubit must be left in `|0⟩`.

Only the physical qubits the circuit actually touches are simulated, so a
small circuit on a 156-qubit device stays cheap to check.

### Independence, and why this is not a simulator

The kernel ([`src/verify/statevector.rs`](../src/verify/statevector.rs))
writes every gate's matrix from its definition. It never reads the
decomposition rules or the pass tables it audits. It shares its origin with
the test-only simulator in `tests/support/`. Qiskit's `quantum_info`
(`python/tests/test_rules.py`) remains the external check on the rules.

§32 rules out "a custom quantum simulator replacing established simulator
frameworks". This checker never samples, returns no counts, has no noise model
and no execution path, and is capped at a few qubits. Its only output is a
verdict comparing two circuits the compiler itself produced.

### What establishes that it works

- **`tests/verifier.rs`:**
  - property agreement with the test oracle on every rung of the ladder;
  - a deliberately wrong extension pass must stop compilation;
  - every example verifies on every built-in backend.
- **Mutation evidence:** in the benchmark suite ([`benchmarks.md`](benchmarks.md)),
  which records which of ten injected defects the checker catches.

## Lint (§13.5)

`oqci lint FILE [--backend ID | --target-file F | --target ID] [--optimized] [--deny warnings|errors]`.
The same rules are available through `oqci.lint(...)` and
`CompilerConfig::lint`. `oqci lint-rules` lists them.

| Rule | Severity | Fires on |
|---|---|---|
| `cancellable-pair` | warning | an adjacent gate/inverse pair the gate-cancellation pass would remove; it uses that pass's own matcher |
| `identity-operation` | warning / info | `id`, or a rotation by exactly zero or an exact multiple of the identity angle. Rotations that are the identity only up to global phase (`rz(2π) = −I`) are reported as info. |
| `unobservable-gate` | warning | in a circuit that measures, a gate no measured outcome depends on (backward liveness) |
| `unused-resource` | info / warning | a qubit never touched or a classical bit never written (info); a classical bit written twice, losing the first outcome (warning) |
| `target-unsupported` | error / info | with a target: an operation, measurement pattern or reset the target cannot run (error), or a gate lowering will decompose (info) |

Output is sorted deterministically. Extension rules register through
`Registry::register_lint_rule`. This is a small, modular rule set. It is not
a static-analysis research system, and nothing here claims one.
