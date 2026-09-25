import { useEffect, useMemo, useState } from "react";
import "./App.css";
import { CircuitDiagram } from "./components/CircuitDiagram";
import { CostLegalityPanel } from "./components/CostLegalityPanel";
import { DagGraph } from "./components/DagGraph";
import { DeviceTopology } from "./components/DeviceTopology";
import { LoweringTimeline } from "./components/LoweringTimeline";
import { PassTimeline } from "./components/PassTimeline";
import { ProvenancePanel } from "./components/ProvenancePanel";
import { usePipelineStore } from "./state/pipelineStore";
import type { BasisProfile } from "./state/types";
import { SERVER_HTTP, connectWatchFile, disconnect } from "./ws/client";

type Tab = "overview" | "passes" | "lowering" | "target" | "executable";

function App() {
  const { connected, connecting, watchedPath, report, passReplay, loweringReplay, error } = usePipelineStore();
  const [pathInput, setPathInput] = useState("examples/ghz3.qasm");
  const [tab, setTab] = useState<Tab>("overview");
  const [stageTab, setStageTab] = useState("qc-ir");
  const [profiles, setProfiles] = useState<BasisProfile[]>([]);
  const [routingLayout, setRoutingLayout] = useState<number[] | null>(null);

  useEffect(() => {
    fetch(`${SERVER_HTTP}/api/targets`)
      .then((r) => r.json())
      .then(setProfiles)
      .catch(() => setProfiles([]));
    return () => disconnect();
  }, []);

  const numQubits = report?.stages.find((s) => s.instructions)?.metrics?.num_qubits ?? 0;
  const numClbits = report?.stages.find((s) => s.instructions)?.metrics?.num_clbits ?? 0;

  const activeStage = useMemo(() => report?.stages.find((s) => s.stage === stageTab), [report, stageTab]);

  const profile = useMemo(() => {
    if (!report?.lowering) return undefined;
    const id = report.lowering.profile.split("@")[0];
    return profiles.find((p) => p.id === id);
  }, [report, profiles]);

  return (
    <div className="app">
      <header className="app__header">
        <h1>oqci — live compiler visualization</h1>
        <div className="app__connect">
          <input
            value={pathInput}
            onChange={(e) => setPathInput(e.target.value)}
            placeholder="path/to/circuit.qasm"
            className="app__path-input"
          />
          <button onClick={() => connectWatchFile(pathInput)} disabled={connecting}>
            {connecting ? "connecting…" : "watch"}
          </button>
          <span className={`status-dot ${connected ? "status-dot--on" : ""}`} title={connected ? "connected" : "disconnected"} />
        </div>
      </header>

      {watchedPath && (
        <div className="app__watched">
          watching <code>{watchedPath}</code>
          {report && (
            <>
              {" — "}
              <strong>{report.circuit_name}</strong> ({report.frontend})
              {report.unbound_parameters.length > 0 && (
                <span className="app__unbound"> · unbound: {report.unbound_parameters.join(", ")}</span>
              )}
            </>
          )}
        </div>
      )}

      {error && <div className="app__error">{error}</div>}

      {!report && !error && <div className="panel-empty app__waiting">Enter a path and click "watch" to begin.</div>}

      {report && (
        <>
          <nav className="app__tabs">
            {(["overview", "passes", "lowering", "target", "executable"] as Tab[]).map((t) => (
              <button key={t} className={`app__tab ${tab === t ? "app__tab--active" : ""}`} onClick={() => setTab(t)}>
                {t}
              </button>
            ))}
          </nav>

          <main className="app__main">
            {tab === "overview" && (
              <section className="panel">
                <div className="app__stage-tabs">
                  {report.stages.map((s) => (
                    <button
                      key={s.stage}
                      className={`app__stage-tab ${stageTab === s.stage ? "app__stage-tab--active" : ""}`}
                      onClick={() => setStageTab(s.stage)}
                    >
                      {s.stage}
                    </button>
                  ))}
                </div>

                {activeStage?.instructions && (
                  <CircuitDiagram
                    instructions={activeStage.instructions}
                    numQubits={activeStage.metrics?.num_qubits ?? numQubits}
                    numClbits={activeStage.metrics?.num_clbits ?? numClbits}
                    layers={report.stages.find((s) => s.stage === "qco-ir")?.graph?.layers}
                  />
                )}
                {activeStage?.graph && <DagGraph graph={activeStage.graph} />}
                {activeStage?.qir && <pre className="qir-text">{activeStage.qir}</pre>}
                {activeStage?.unavailable && <div className="panel-empty">{activeStage.unavailable}</div>}

                {report.diff && report.diff.length > 0 && (
                  <div className="diff-view">
                    <div className="info-panel__key">diff (source → optimized)</div>
                    {report.diff.map((d, i) => {
                      const kind = d.marker === "-" ? "removed" : d.marker === "+" ? "added" : "unchanged";
                      return (
                        <div key={i} className={`diff-line diff-line--${kind}`}>
                          {d.marker} {d.text}
                        </div>
                      );
                    })}
                  </div>
                )}
              </section>
            )}

            {tab === "passes" && (
              <section className="panel">
                <PassTimeline replay={passReplay} numQubits={numQubits} numClbits={numClbits} />
              </section>
            )}

            {tab === "lowering" && (
              <section className="panel panel--split">
                <div className="panel__col">
                  <LoweringTimeline
                    replay={loweringReplay}
                    numQubits={profile?.topology.qubit_count ?? numQubits}
                    numClbits={numClbits}
                    onLayoutChange={setRoutingLayout}
                  />
                </div>
                <div className="panel__col panel__col--narrow">
                  {profile ? (
                    <DeviceTopology
                      topology={profile.topology}
                      layout={routingLayout ?? report.lowering?.final_layout}
                      title={`${profile.id}@${profile.version}`}
                    />
                  ) : (
                    <div className="panel-empty">No target profile resolved for this backend.</div>
                  )}
                </div>
              </section>
            )}

            {tab === "target" && (
              <section className="panel">
                <CostLegalityPanel target={report.target} />
              </section>
            )}

            {tab === "executable" && (
              <section className="panel">
                <ProvenancePanel provenance={report.executable?.executable.provenance} />
              </section>
            )}
          </main>
        </>
      )}
    </div>
  );
}

export default App;
