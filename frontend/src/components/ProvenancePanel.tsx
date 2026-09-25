import type { Provenance } from "../state/types";
import "./InfoPanel.css";

export function ProvenancePanel({ provenance }: { provenance: Provenance | undefined }) {
  if (!provenance) return <div className="panel-empty">No executable prepared yet.</div>;

  const rows: [string, string][] = [
    ["circuit", provenance.circuit],
    ["compiler version", provenance.compiler_version],
    ["git commit", provenance.git_commit],
    ["backend", provenance.backend_id],
    ["target profile", provenance.profile_id],
    ["cost model", `${provenance.cost_model_id}@${provenance.cost_model_version}`],
    ["pass pipeline", provenance.pass_pipeline.join(" → ") || "(none)"],
    ["lowering steps", provenance.lowering_steps.join(" → ")],
    ["decomposition rules", provenance.decomposition_rules.join(", ") || "(none)"],
    ["initial layout", `[${provenance.initial_layout.join(", ")}]`],
    ["final layout", `[${provenance.final_layout.join(", ")}]`],
    ["swaps inserted", String(provenance.swaps_inserted)],
    ["shots", String(provenance.shots)],
    ["seed", provenance.seed === undefined || provenance.seed === null ? "(none)" : String(provenance.seed)],
  ];

  return (
    <div className="info-panel">
      {rows.map(([k, v]) => (
        <div key={k} className="info-panel__row">
          <span className="info-panel__key">{k}</span>
          <span className="info-panel__value">{v}</span>
        </div>
      ))}
    </div>
  );
}
