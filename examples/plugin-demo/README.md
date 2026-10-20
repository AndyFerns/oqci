# `oqci-plugin-demo` — extending OQCI without touching it

A demonstration of the extension boundary in
[`docs/extensions.md`](../../docs/extensions.md)
(`final-deliverables-spec.md` §18, Definition of Done item 9). It adds one of
each kind of component through the public API alone. The crate depends on
`oqci` as any third party would, so it cannot reach any of the compiler's
crate-private items.

| Component | What it is | Spec |
|---|---|---|
| `HzhToX` | an optimization pass that rewrites `H; Z; H` on one qubit into `X`. It is placed before `single-qubit-fusion`, which would otherwise absorb the triple first. | §18.2 |
| `Qtoy` | a line-based source language (`.qtoy` files), read through `CircuitBuilder` and `map_gate` like every built-in frontend | §18.1 |
| `demo_ring` | a synthetic six-qubit CZ-native ring that supplies its own profile, lowering, execution interface and cost model | §18.3 |

```bash
cargo run -p oqci-plugin-demo -- passes
cargo run -p oqci-plugin-demo -- lower my.qtoy --backend demo-ring --verify
cargo test -p oqci-plugin-demo
```

`tests/no_core_modification.rs` compiles a `qtoy` program end to end. The demo pass optimizes it, it is lowered onto the demo backend, and the compiler's own checker verifies the semantics of every transformation, the extension pass included. The test also checks two things about provenance. It names exactly the three extensions with their crate version. And the registry refuses any id that would be ambiguous.

`demo-ring` describes no real device. Its profile says so in its own capabilities (`synthetic-not-a-real-device`).
