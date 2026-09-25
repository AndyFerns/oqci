import { useMemo } from "react";
import ReactFlow, { Background, type Edge, type Node, Position } from "reactflow";
import "reactflow/dist/style.css";
import type { GraphView } from "../state/types";

interface Props {
  graph: GraphView;
}

const COL_WIDTH = 160;
const ROW_HEIGHT = 70;

/** Parses a node label like `"op:2"`, `"in:%q0"`, `"out:%c1"` — exactly the
 * labels `oqci::cli::snapshot::graph_of` produces — into a stable id and a
 * display string, without reinterpreting what they mean. */
function parseLabel(label: string): { id: string; text: string; kind: "op" | "in" | "out" } {
  const [prefix, rest] = label.split(":", 2);
  const kind = prefix === "in" ? "in" : prefix === "out" ? "out" : "op";
  return { id: label, text: kind === "op" ? `#${rest}` : rest, kind };
}

export function DagGraph({ graph }: Props) {
  const { nodes, edges } = useMemo(() => {
    const opColumn = new Map<string, number>();
    graph.layers.forEach((layer, col) => {
      layer.forEach((index) => opColumn.set(`op:${index}`, col));
    });

    const rowByWire = new Map<string, number>();
    let nextRow = 0;
    const rowFor = (wireKey: string) => {
      if (!rowByWire.has(wireKey)) rowByWire.set(wireKey, nextRow++);
      return rowByWire.get(wireKey)!;
    };

    const nodeIds = new Set<string>();
    graph.edges.forEach((e) => {
      nodeIds.add(e.from);
      nodeIds.add(e.to);
    });

    const flowNodes: Node[] = Array.from(nodeIds).map((id) => {
      const { text, kind } = parseLabel(id);
      const isBoundary = kind !== "op";
      const wireForRow = isBoundary ? text : (graph.edges.find((e) => e.from === id || e.to === id)?.wire ?? id);
      const row = rowFor(wireForRow);
      const col = kind === "in" ? -1 : kind === "out" ? graph.depth : (opColumn.get(id) ?? 0);

      return {
        id,
        position: { x: col * COL_WIDTH, y: row * ROW_HEIGHT },
        data: { label: text },
        sourcePosition: Position.Right,
        targetPosition: Position.Left,
        style: {
          background: isBoundary ? "var(--panel-bg-alt)" : "var(--gate)",
          color: isBoundary ? "var(--text-dim)" : "#0b0d12",
          border: isBoundary ? "1px dashed var(--border)" : "none",
          borderRadius: isBoundary ? 20 : 8,
          fontFamily: "var(--mono)",
          fontSize: 12,
          fontWeight: 600,
          width: isBoundary ? 60 : 44,
        },
      };
    });

    const flowEdges: Edge[] = graph.edges.map((e, i) => ({
      id: `e${i}-${e.from}-${e.to}`,
      source: e.from,
      target: e.to,
      label: e.kind === "control" ? "control" : undefined,
      animated: e.kind === "control",
      style: {
        stroke: e.kind === "control" ? "var(--multi)" : "var(--wire)",
        strokeDasharray: e.kind === "control" ? "4 3" : undefined,
      },
    }));

    return { nodes: flowNodes, edges: flowEdges };
  }, [graph]);

  return (
    <div style={{ height: 360, background: "var(--panel-bg)", borderRadius: 8 }}>
      <ReactFlow nodes={nodes} edges={edges} fitView proOptions={{ hideAttribution: true }}>
        <Background color="var(--border)" gap={24} />
      </ReactFlow>
    </div>
  );
}
