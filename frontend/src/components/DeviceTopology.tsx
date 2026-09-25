import { useMemo, useState } from "react";
import type { Topology } from "../state/types";
import "./DeviceTopology.css";

type Mode = "directed" | "symmetric" | "undirected";

interface Props {
  topology: Topology;
  /** Logical -> physical assignment to overlay as colored tokens, ascending
   * by logical qubit (the same shape `Layout::permutation()` /
   * `LoweringView.initial_layout`/`final_layout` already use). */
  layout?: number[] | null;
  title?: string;
}

const RADIUS = 120;
const CENTER = 150;
const NODE_R = 18;

function isSymmetric(edges: [number, number][], a: number, b: number): boolean {
  return edges.some(([x, y]) => x === a && y === b) && edges.some(([x, y]) => x === b && y === a);
}

export function DeviceTopology({ topology, layout, title }: Props) {
  const [mode, setMode] = useState<Mode>("directed");
  const n = topology.qubit_count;

  const positions = useMemo(() => {
    const pos: { x: number; y: number }[] = [];
    for (let i = 0; i < n; i++) {
      const angle = (2 * Math.PI * i) / n - Math.PI / 2;
      pos.push({ x: CENTER + RADIUS * Math.cos(angle), y: CENTER + RADIUS * Math.sin(angle) });
    }
    return pos;
  }, [n]);

  const visibleEdges = useMemo(() => {
    if (mode === "directed") return topology.edges;
    if (mode === "symmetric") {
      return topology.edges.filter(([a, b]) => isSymmetric(topology.edges, a, b));
    }
    // undirected: dedupe reciprocal pairs
    const seen = new Set<string>();
    const out: [number, number][] = [];
    for (const [a, b] of topology.edges) {
      const key = a < b ? `${a}-${b}` : `${b}-${a}`;
      if (!seen.has(key)) {
        seen.add(key);
        out.push([a, b]);
      }
    }
    return out;
  }, [topology.edges, mode]);

  const physicalOf = useMemo(() => {
    const map = new Map<number, number>();
    layout?.forEach((physical, logical) => map.set(physical, logical));
    return map;
  }, [layout]);

  return (
    <div className="device-topology">
      {title && <div className="circuit-diagram__title">{title}</div>}
      <div className="device-topology__modes">
        {(["directed", "symmetric", "undirected"] as Mode[]).map((m) => (
          <button key={m} className={`mode-btn ${mode === m ? "mode-btn--active" : ""}`} onClick={() => setMode(m)}>
            {m}
          </button>
        ))}
      </div>
      <svg width={CENTER * 2} height={CENTER * 2}>
        <defs>
          <marker id="arrow" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto">
            <path d="M0,0 L8,4 L0,8 Z" fill="var(--text-dim)" />
          </marker>
        </defs>
        {visibleEdges.map(([a, b], i) => {
          const pa = positions[a];
          const pb = positions[b];
          if (!pa || !pb) return null;
          // shorten the line so the arrowhead doesn't sit under the node
          const dx = pb.x - pa.x;
          const dy = pb.y - pa.y;
          const len = Math.hypot(dx, dy) || 1;
          const x2 = pb.x - (dx / len) * (NODE_R + 6);
          const y2 = pb.y - (dy / len) * (NODE_R + 6);
          return (
            <line
              key={`${a}-${b}-${i}`}
              x1={pa.x}
              y1={pa.y}
              x2={mode === "directed" ? x2 : pb.x}
              y2={mode === "directed" ? y2 : pb.y}
              stroke="var(--text-dim)"
              strokeWidth={1.5}
              markerEnd={mode === "directed" ? "url(#arrow)" : undefined}
            />
          );
        })}
        {positions.map((p, physical) => {
          const logical = physicalOf.get(physical);
          return (
            <g key={physical}>
              <circle
                cx={p.x}
                cy={p.y}
                r={NODE_R}
                className={logical !== undefined ? "topology-node topology-node--occupied" : "topology-node"}
              />
              <text x={p.x} y={p.y + 4} textAnchor="middle" className="topology-node__label">
                {logical !== undefined ? `L${logical}` : `#${physical}`}
              </text>
              {logical !== undefined && (
                <text x={p.x} y={p.y - NODE_R - 6} textAnchor="middle" className="topology-node__sublabel">
                  #{physical}
                </text>
              )}
            </g>
          );
        })}
      </svg>
    </div>
  );
}
