# ADR: No Frontend in Phase 0

**Status:** Accepted. **Applies to:** Phase 0 (IR core). **Supersedes:** none.

## Context

OQCI is an LLVM-inspired, multi-level quantum compiler: frontends (OpenQASM,
Qiskit, Cirq, CUDA-Q) → QC-IR → QCO-IR → passes → QIR/LLVM. It is tempting to
build a frontend parser first, because a frontend makes the system feel usable
immediately and gives concrete circuits to test against.

Phase 0 deliberately builds **no** frontend parsers or source-language adapters.
QIR emission is the far boundary; frontends are Phase 1.

## Decision

**Do not build any frontend (OpenQASM parser, Qiskit/Cirq/CUDA-Q adapter, or any
surface-syntax reader) until QC-IR and QCO-IR are stable.** Validate the IR
against the abstract operational semantics of quantum circuits first, through the
builder API and the QC-IR → QCO-IR → QIR pipeline, independent of any concrete
surface syntax.

## Rationale (the verbatim scope rationale, preserved)

> Building a frontend before the IR is finalized risks silently biasing IR design
> toward one source language's syntax/semantics. The IR must be validated against
> the abstract operational semantics of quantum circuits first, independent of any
> concrete surface syntax. Frontends come in Phase 1, once QC-IR/QCO-IR are stable
> and this bias risk is retired.

### Why the bias is real and hard to detect

A frontend does not just parse — it *chooses which IR shapes are easy to
produce*. If OpenQASM were the first frontend, QC-IR would tend to grow OpenQASM's
register model, its gate-modifier syntax, its `gphase`, its classical-bit
semantics. Those choices would then look "natural" in the IR and go unquestioned,
even though they are one language's accidents. The bias is silent precisely
because it arrives as "this made the parser simpler," not as an explicit design
argument. By fixing the IR against language-agnostic operational semantics first,
every later frontend must map *onto* a neutral IR, exposing genuine mismatches as
frontend problems rather than quietly reshaping the core.

### Why "just one small parser to test with" is a trap

Tests do not need a parser: the `CircuitBuilder` API constructs any circuit
directly and more precisely than a parser would, and the integration suite
(`tests/pipeline.rs`) already covers identity, Bell, GHZ-3, mid-circuit
measurement, and every validation error path. A parser added "just for testing"
would (a) acquire users, (b) acquire a de-facto spec, and (c) become the very
bias vector this ADR exists to prevent — all before the IR is stable.

## Consequences

- **Positive:** the IR is designed against quantum-circuit semantics, not a
  source language; multiple frontends in Phase 1 can be judged by how cleanly
  they map onto a neutral core; no throwaway parser accrues hidden influence.
- **Negative / accepted cost:** no "paste OpenQASM, get QIR" demo yet; circuits
  are constructed programmatically via the builder. This is acceptable because
  Phase 0's audience is the IR itself, not end users.

## For future contributors

Do not "helpfully" add a parser early. If you need example circuits, use
`CircuitBuilder`. A frontend PR should land in Phase 1, **after** QC-IR/QCO-IR
are declared stable, and should be reviewed as a mapping onto the existing IR —
never as a reason to change the IR's core shape. If a frontend reveals a genuine
gap, raise it as an IR change with its own rationale (per the project's
"every non-trivial decision gets a paragraph" rule), not as a parser convenience.

## Related

- [`ir_spec.md`](ir_spec.md) — the operational semantics the IR is validated
  against (esp. §3.4, §6).
- [`architecture_decision_mlir_phase2.md`](architecture_decision_mlir_phase2.md)
  — the companion ADR on keeping the MLIR port cheap.
