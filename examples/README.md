# `examples/` — Sample OpenQASM 3 Programs

Small, runnable programs used as onboarding material for the `oqci` CLI and
as fixtures for [`../tests/cli.rs`](../tests/cli.rs). If you add a file here,
that test file's `every_example_compiles` case picks it up automatically —
but a new fixture used by a *specific* test still needs that test written
explicitly.

| File | Demonstrates |
|---|---|
| [`bell.qasm`](bell.qasm) | The minimal two-qubit entangled pair. The first thing to run: `oqci compile examples/bell.qasm`. |
| [`ghz3.qasm`](ghz3.qasm) | A three-qubit GHZ state with a deliberately redundant `x q[2]; h q[1]; x q[2];` sequence — the two `x` gates cancel despite the unrelated gate between them, because cancellation is a QCO-IR-adjacency question, not a textual one. Run `oqci optimize examples/ghz3.qasm --diff` to see it happen. |
| [`parameterized.qasm`](parameterized.qasm) | A VQE-style circuit with two `input`-declared symbolic parameters and a pair of same-axis rotations for `rotation-merge` to fold. QIR emission is deliberately unavailable until you supply `--bind theta=… --bind phi=…`. |

## Try it

```bash
oqci compile examples/bell.qasm
oqci optimize examples/ghz3.qasm --diff
oqci compile examples/parameterized.qasm --bind theta=1.5708 --bind phi=0.7854
oqci watch examples/bell.qasm --mode optimize
```

See [`../docs/cli.md`](../docs/cli.md) for the full command reference, and
[`../docs/openqasm_frontend.md`](../docs/openqasm_frontend.md) for exactly
which OpenQASM 3 constructs these files (and the frontend generally) may use.
