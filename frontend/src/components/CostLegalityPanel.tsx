import type { TargetReportView } from "../state/types";
import "./InfoPanel.css";

export function CostLegalityPanel({ target }: { target: TargetReportView | undefined }) {
  if (!target) return <div className="panel-empty">No target selected.</div>;

  const scalar = target.cost["scalar_score"];
  const weights = Object.entries(target.cost_model_configuration);

  return (
    <div className="info-panel">
      <div className="info-panel__row">
        <span className="info-panel__key">profile</span>
        <span className="info-panel__value">{target.profile}</span>
      </div>
      <div className="info-panel__row">
        <span className="info-panel__key">legality</span>
        <span className={`info-panel__value ${target.legal ? "info-panel__value--ok" : "info-panel__value--bad"}`}>
          {target.legal ? "legal" : `${target.violations.length} violation(s)`}
        </span>
      </div>

      {!target.legal && (
        <ul className="violation-list">
          {target.violations.map((v, i) => (
            <li key={i} className="violation-list__item">
              {JSON.stringify(v)}
            </li>
          ))}
        </ul>
      )}

      <div className="info-panel__row">
        <span className="info-panel__key">cost model</span>
        <span className="info-panel__value">{target.cost_model}</span>
      </div>

      {/* A scalar is never shown without the weights that produced it —
          Stage E §6/§7's discipline, applied at the presentation layer too. */}
      {typeof scalar === "number" && (
        <div className="info-panel__row">
          <span className="info-panel__key">scalar score</span>
          <span className="info-panel__value">
            {scalar}
            {weights.length > 0 && (
              <span className="info-panel__weights">
                {" "}
                (from {weights.map(([k, v]) => `${k}=${v}`).join(", ")})
              </span>
            )}
          </span>
        </div>
      )}

      <div className="cost-breakdown">
        {Object.entries(target.cost)
          .filter(([k]) => k !== "scalar_score")
          .map(([k, v]) => (
            <div key={k} className="cost-breakdown__item">
              <span className="cost-breakdown__key">{k}</span>
              <span className="cost-breakdown__value">{typeof v === "object" ? JSON.stringify(v) : String(v)}</span>
            </div>
          ))}
      </div>
    </div>
  );
}
