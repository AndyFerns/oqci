# OQCI Visualization Frontend

The browser UI for [`oqci-server`](../server/README.md) — live circuit
diagrams, a QCO-IR dependency graph, a pass-by-pass optimization timeline,
and a lowering timeline (with individual SWAPs and individual decomposition
rule firings), all driven by the WebSocket contract documented in
[`docs/visualization.md`](../docs/visualization.md).

This app makes no compilation decision. Everything it renders comes from the
server, which itself only calls the real, unmodified `oqci` compiler.

## Layout

| Path | What it is |
|---|---|
| `src/state/types.ts` | TypeScript mirror of the server's JSON contract (`oqci::cli::snapshot` + `server/src/events.rs`). Keep in lockstep with those two files. |
| `src/state/pipelineStore.ts` | Zustand store — the latest `PipelineReport` plus the current sequence's replay events. |
| `src/ws/client.ts` | The WebSocket connection. |
| `src/components/CircuitDiagram.tsx` | Hand-rolled SVG piano-roll renderer, laid out from the compiler's own `QcoCircuit::layers()` data (never recomputed client-side). |
| `src/components/DagGraph.tsx` | `react-flow` view of the QCO-IR dependency graph. |
| `src/components/PassTimeline.tsx` | Scrub through each optimization pass. |
| `src/components/LoweringTimeline.tsx` | Scrub through layout/routing/decomposition, down to individual SWAPs and rule firings. |
| `src/components/DeviceTopology.tsx` | The target device's coupling graph, with the current logical→physical layout overlaid. |

## Running

```bash
npm install
npm run dev
```

Requires `oqci-server` running (`cargo run -p oqci-server -- --root . --backend simulator-nisq` from the repo root) — see [`server/README.md`](../server/README.md).

```bash
npm run build   # dist/, servable by oqci-server --static-dir
```
