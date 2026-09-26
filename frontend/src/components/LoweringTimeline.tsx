import { useEffect, useMemo, useState } from "react";
import type { LoweringReplay } from "../state/types";
import { CircuitDiagram } from "./CircuitDiagram";
import "./Timeline.css";

interface Props {
  replay: LoweringReplay | null;
  numQubits: number;
  numClbits: number;
  /** Called with the logical->physical layout after the currently-scrubbed
   * SWAP, so a sibling `DeviceTopology` panel can animate the same
   * trajectory the routing step just produced. */
  onLayoutChange?: (layout: number[] | null) => void;
}

export function LoweringTimeline({ replay, numQubits, numClbits, onLayoutChange }: Props) {
  const [stepIndex, setStepIndex] = useState(0);
  const [swapIndex, setSwapIndex] = useState(-1);

  const step = replay && !replay.degraded ? replay.steps[Math.min(stepIndex, replay.steps.length - 1)] : null;

  const highlight = useMemo(() => {
    if (!step) return undefined;
    const set = new Set<number>();
    step.rule_firings.forEach((f) => set.add(f.source_index));
    return set.size > 0 ? set : undefined;
  }, [step]);

  useEffect(() => {
    if (!onLayoutChange) return;
    if (step?.id === "routing" && swapIndex >= 0 && step.swap_events[swapIndex]) {
      onLayoutChange(step.swap_events[swapIndex].layout_after);
    } else {
      onLayoutChange(null);
    }
  }, [step, swapIndex, onLayoutChange]);

  if (!replay) {
    return <div className="panel-empty">Waiting for a lowering replay…</div>;
  }
  if (replay.degraded) {
    return (
      <div className="panel-degraded">
        Lowering replay could not be cross-validated against the compiler ({replay.degraded_reason}). Showing
        nothing rather than a possibly-wrong reconstruction — the coarse lowering summary above is still
        accurate.
      </div>
    );
  }
  if (!step) return null;

  return (
    <div className="timeline">
      <div className="timeline__stepper">
        {replay.steps.map((s, i) => (
          <button
            key={s.id}
            className={`timeline__step ${i === stepIndex ? "timeline__step--active" : ""}`}
            onClick={() => {
              setStepIndex(i);
              setSwapIndex(-1);
            }}
          >
            <span className="timeline__step-index">{i + 1}</span>
            <span className="timeline__step-id">{s.id}</span>
          </button>
        ))}
      </div>

      <div className="timeline__detail">
        <div className="timeline__metrics">
          <span>{step.op_count} ops</span>
          <span>{step.detail}</span>
        </div>

        {step.swap_events.length > 0 && (
          <div className="sub-timeline">
            <div className="sub-timeline__label">swaps inserted ({step.swap_events.length})</div>
            <div className="sub-timeline__row">
              <button
                className={`sub-timeline__dot ${swapIndex === -1 ? "sub-timeline__dot--active" : ""}`}
                onClick={() => setSwapIndex(-1)}
              >
                initial
              </button>
              {step.swap_events.map((s, i) => (
                <button
                  key={s.sequence}
                  className={`sub-timeline__dot ${swapIndex === i ? "sub-timeline__dot--active" : ""}`}
                  onClick={() => setSwapIndex(i)}
                  title={`swap #q${s.physical_a} <-> #q${s.physical_b}`}
                >
                  #{i + 1}
                </button>
              ))}
            </div>
            {swapIndex >= 0 && (
              <div className="sub-timeline__detail">
                swap physical qubits #q{step.swap_events[swapIndex].physical_a} ↔ #q
                {step.swap_events[swapIndex].physical_b} — layout now [
                {step.swap_events[swapIndex].layout_after.join(", ")}]
              </div>
            )}
          </div>
        )}

        {step.rule_firings.length > 0 && (
          <div className="sub-timeline">
            <div className="sub-timeline__label">rule firings ({step.rule_firings.length})</div>
            <ul className="rule-list">
              {step.rule_firings.map((f) => (
                <li key={f.sequence} className="rule-list__item">
                  <span className="rule-list__source">
                    {f.source_mnemonic}({f.source_qubits.map((q) => `q${q}`).join(",")})
                  </span>
                  <span className="rule-list__arrow">→</span>
                  <span className="rule-list__expansion">{f.expanded_into.map((e) => e.text).join("; ")}</span>
                  <span className="rule-list__rules">{f.rules_applied.join(", ")}</span>
                </li>
              ))}
            </ul>
          </div>
        )}

        <CircuitDiagram instructions={step.instructions} numQubits={numQubits} numClbits={numClbits} highlight={highlight} />
      </div>
    </div>
  );
}
