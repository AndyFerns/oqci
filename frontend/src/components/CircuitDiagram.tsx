import { useMemo } from "react";
import type { InstructionView } from "../state/types";
import "./CircuitDiagram.css";

interface Props {
  instructions: InstructionView[];
  numQubits: number;
  numClbits?: number;
  /** ASAP layers from the compiler's own `QcoCircuit::layers()` (via
   * `GraphView.layers`), grouping program indices into provably-independent
   * columns. When omitted, instructions are laid out one per column in
   * program order — a safe, always-available fallback. */
  layers?: number[][];
  /** Program indices to visually emphasize (e.g. the instructions a scrubbed
   * pass step just changed). */
  highlight?: Set<number>;
  title?: string;
}

const ROW_HEIGHT = 44;
const COL_WIDTH = 64;
const LEFT_MARGIN = 84;
const TOP_MARGIN = 24;
const BOX = 30;

function columnOf(index: number, layers: number[][] | undefined): number {
  if (!layers) return index;
  for (let c = 0; c < layers.length; c++) {
    if (layers[c].includes(index)) return c;
  }
  return index;
}

function gateColor(op: string, gate?: string): string {
  if (op === "measure") return "var(--meter)";
  if (op === "reset") return "var(--reset)";
  if (gate === "swap") return "var(--swap)";
  if (["cx", "cy", "cz", "ccx"].includes(gate ?? "")) return "var(--multi)";
  return "var(--gate)";
}

export function CircuitDiagram({ instructions, numQubits, numClbits = 0, layers, highlight, title }: Props) {
  const columns = useMemo(
    () => instructions.map((inst) => columnOf(inst.index, layers)),
    [instructions, layers],
  );
  const colCount = columns.length > 0 ? Math.max(...columns) + 1 : 1;
  const width = LEFT_MARGIN + colCount * COL_WIDTH + 24;
  const clbitRow = numQubits;
  const rowCount = numQubits + (numClbits > 0 ? 1 : 0);
  const height = TOP_MARGIN * 2 + rowCount * ROW_HEIGHT;

  const yOf = (row: number) => TOP_MARGIN + row * ROW_HEIGHT + ROW_HEIGHT / 2;
  const xOf = (col: number) => LEFT_MARGIN + col * COL_WIDTH + COL_WIDTH / 2;

  return (
    <div className="circuit-diagram">
      {title && <div className="circuit-diagram__title">{title}</div>}
      <svg width={width} height={height} className="circuit-diagram__svg">
        {Array.from({ length: numQubits }).map((_, q) => (
          <g key={`wire-${q}`}>
            <line x1={LEFT_MARGIN} y1={yOf(q)} x2={width - 12} y2={yOf(q)} className="wire" />
            <text x={8} y={yOf(q) + 4} className="wire-label">
              q{q}
            </text>
          </g>
        ))}
        {numClbits > 0 && (
          <g>
            <line x1={LEFT_MARGIN} y1={yOf(clbitRow) - 2} x2={width - 12} y2={yOf(clbitRow) - 2} className="wire wire--classical" />
            <line x1={LEFT_MARGIN} y1={yOf(clbitRow) + 2} x2={width - 12} y2={yOf(clbitRow) + 2} className="wire wire--classical" />
            <text x={8} y={yOf(clbitRow) + 4} className="wire-label">
              c
            </text>
          </g>
        )}

        {instructions.map((inst, i) => {
          const col = columns[i];
          const x = xOf(col);
          const isHighlighted = highlight?.has(inst.index) ?? false;
          const rows = inst.qubits.length > 0 ? inst.qubits : [0];
          const minY = Math.min(...rows.map(yOf));
          const maxY = Math.max(...rows.map(yOf));
          const color = gateColor(inst.op, inst.gate);

          return (
            <g key={`${inst.index}-${i}`} className={isHighlighted ? "op op--highlight" : "op"}>
              {rows.length > 1 && <line x1={x} y1={minY} x2={x} y2={maxY} className="op__connector" />}

              {inst.op === "measure" ? (
                <>
                  <line x1={x} y1={yOf(inst.qubits[0])} x2={x} y2={yOf(clbitRow)} className="op__connector op__connector--classical" />
                  <rect x={x - BOX / 2} y={yOf(inst.qubits[0]) - BOX / 2} width={BOX} height={BOX} rx={4} className="op__box" style={{ fill: color }} />
                  <text x={x} y={yOf(inst.qubits[0]) + 4} className="op__label" textAnchor="middle">
                    M
                  </text>
                </>
              ) : rows.length === 1 ? (
                <>
                  <rect x={x - BOX / 2} y={yOf(rows[0]) - BOX / 2} width={BOX} height={BOX} rx={4} className="op__box" style={{ fill: color }} />
                  <text x={x} y={yOf(rows[0]) + 4} className="op__label" textAnchor="middle">
                    {inst.op === "reset" ? "|0⟩" : (inst.gate ?? "?").toUpperCase().slice(0, 3)}
                  </text>
                </>
              ) : (
                rows.map((r) => <circle key={r} cx={x} cy={yOf(r)} r={7} className="op__node" style={{ fill: color }} />)
              )}

              {rows.length > 1 && (
                <text x={x} y={minY - 10} className="op__multi-label" textAnchor="middle">
                  {inst.gate ?? "?"}
                </text>
              )}
            </g>
          );
        })}
      </svg>
    </div>
  );
}
