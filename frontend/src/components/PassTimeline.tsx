import { useState } from "react";
import type { PassReplay } from "../state/types";
import { CircuitDiagram } from "./CircuitDiagram";
import "./Timeline.css";

interface Props {
  replay: PassReplay | null;
  numQubits: number;
  numClbits: number;
}

export function PassTimeline({ replay, numQubits, numClbits }: Props) {
  const [selected, setSelected] = useState(0);

  if (!replay) {
    return <div className="panel-empty">Waiting for a pass replay…</div>;
  }
  if (replay.degraded) {
    return (
      <div className="panel-degraded">
        Pass replay could not be cross-validated against the compiler ({replay.degraded_reason}). Showing
        nothing rather than a possibly-wrong reconstruction — the coarse pass table above is still accurate.
      </div>
    );
  }
  if (replay.steps.length === 0) {
    return <div className="panel-empty">No passes ran.</div>;
  }

  const step = replay.steps[Math.min(selected, replay.steps.length - 1)];

  return (
    <div className="timeline">
      <div className="timeline__stepper">
        {replay.steps.map((s, i) => (
          <button
            key={i}
            className={`timeline__step ${i === selected ? "timeline__step--active" : ""} ${!s.enabled ? "timeline__step--disabled" : ""}`}
            onClick={() => setSelected(i)}
            title={s.notes.join("; ")}
          >
            <span className="timeline__step-index">{i + 1}</span>
            <span className="timeline__step-id">{s.id}</span>
            {s.changed && <span className="timeline__step-badge">changed</span>}
            {!s.enabled && <span className="timeline__step-badge timeline__step-badge--skip">skipped</span>}
          </button>
        ))}
      </div>

      <div className="timeline__detail">
        <div className="timeline__metrics">
          <span>
            {step.before.op_count} → {step.after.op_count} ops
          </span>
          <span>
            depth {step.before.depth} → {step.after.depth}
          </span>
          <span>{step.duration_us}µs</span>
          {step.notes.length > 0 && <span className="timeline__notes">{step.notes.join("; ")}</span>}
        </div>

        <div className="timeline__diagrams">
          <CircuitDiagram
            instructions={step.instructions_before}
            numQubits={numQubits}
            numClbits={numClbits}
            title="before"
          />
          <CircuitDiagram
            instructions={step.instructions_after}
            numQubits={numQubits}
            numClbits={numClbits}
            title="after"
          />
        </div>
      </div>
    </div>
  );
}
