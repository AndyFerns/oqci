// Mirrors the Rust JSON contract exactly:
//   - PipelineReport & co.:      src/cli/snapshot.rs (oqci core crate)
//   - PassReplay/LoweringReplay: server/src/events.rs (oqci-server)
// Field names and shapes must stay in lockstep with those two files — see
// docs/visualization.md.

export type ParamView =
  | { kind: "concrete"; radians: number }
  | { kind: "symbol"; name: string };

export interface InstructionView {
  index: number;
  op: "gate" | "measure" | "reset";
  gate?: string;
  params: ParamView[];
  qubits: number[];
  clbit?: number;
  text: string;
}

export interface EdgeView {
  from: string;
  to: string;
  wire: string;
  kind: "data" | "control";
}

export interface GraphView {
  op_count: number;
  depth: number;
  max_parallel_width: number;
  layers: number[][];
  edges: EdgeView[];
}

export interface ResourceReport {
  num_qubits: number;
  num_clbits: number;
  op_count: number;
  one_qubit_count: number;
  two_qubit_count: number;
  measure_count: number;
  reset_count: number;
  gate_counts: Record<string, number>;
  depth: number;
  max_parallel_width: number;
}

export interface StageSnapshot {
  stage: string;
  metrics?: ResourceReport;
  instructions?: InstructionView[];
  graph?: GraphView;
  qir?: string;
  unavailable?: string;
}

export interface PassRecordView {
  id: string;
  description: string;
  enabled: boolean;
  changed: boolean;
  before: ResourceReport;
  after: ResourceReport;
  duration_us: number;
  notes: string[];
}

export interface DiffEntryView {
  marker: string;
  text: string;
}

// Violation and Cost are structurally open (their exact Rust shape carries
// several `#[serde(tag=...)]` variants and backend-defined fields) — render
// them generically rather than over-typing.
export type Violation = Record<string, unknown>;
export type Cost = Record<string, unknown>;

export interface TargetReportView {
  profile: string;
  backend_id: string;
  qubit_count: number;
  edge_count: number;
  supported_operations: string[];
  legal: boolean;
  violations: Violation[];
  cost_model: string;
  cost_model_configuration: Record<string, string>;
  cost: Cost;
}

export interface LoweringStepView {
  id: string;
  op_count: number;
  detail: string;
}

export interface LoweringView {
  backend: string;
  profile: string;
  initial_layout: number[];
  final_layout: number[];
  swaps_inserted: number;
  orientations_repaired: number;
  rules_applied: string[];
  steps: LoweringStepView[];
  legal: boolean;
  violations: Violation[];
}

export interface ExecutableOp {
  op: string;
  qubits: number[];
  params: number[];
  clbit?: number;
}

export interface Provenance {
  circuit: string;
  compiler_version: string;
  git_commit: string;
  backend_id: string;
  profile_id: string;
  cost_model_id: string;
  cost_model_version: string;
  cost_model_configuration: Record<string, string>;
  pass_pipeline: string[];
  lowering_steps: string[];
  decomposition_rules: string[];
  initial_layout: number[];
  final_layout: number[];
  swaps_inserted: number;
  shots: number;
  seed?: number;
}

export interface Executable {
  backend_id: string;
  num_qubits: number;
  num_clbits: number;
  ops: ExecutableOp[];
  settings: { shots: number; seed?: number; memory: boolean };
  provenance: Provenance;
}

export interface ExecutableView {
  backend_id: string;
  num_qubits: number;
  num_clbits: number;
  operations: string[];
  has_measurement: boolean;
  executable: Executable;
}

export interface PipelineReport {
  source_path: string;
  frontend: string;
  circuit_name: string;
  unbound_parameters: string[];
  stages: StageSnapshot[];
  passes?: PassRecordView[];
  diff?: DiffEntryView[];
  target?: TargetReportView;
  lowering?: LoweringView;
  executable?: ExecutableView;
}

// --- Fine-grained replay events (server-owned, not part of the core JSON
// contract, but built from the same InstructionView/ResourceReport types) --

export interface PassStepEvent {
  pass_index: number;
  id: string;
  enabled: boolean;
  changed: boolean;
  before: ResourceReport;
  after: ResourceReport;
  instructions_before: InstructionView[];
  instructions_after: InstructionView[];
  duration_us: number;
  notes: string[];
}

export interface PassReplay {
  steps: PassStepEvent[];
  degraded: boolean;
  degraded_reason?: string;
}

export interface SwapEvent {
  sequence: number;
  physical_a: number;
  physical_b: number;
  layout_after: number[];
}

export interface RuleFiringEvent {
  sequence: number;
  scope: string;
  source_index: number;
  source_mnemonic: string;
  source_qubits: number[];
  expanded_into: InstructionView[];
  rules_applied: string[];
}

export interface LoweringStepReplay {
  id: string;
  op_count: number;
  detail: string;
  instructions: InstructionView[];
  swap_events: SwapEvent[];
  rule_firings: RuleFiringEvent[];
}

export interface LoweringReplay {
  steps: LoweringStepReplay[];
  degraded: boolean;
  degraded_reason?: string;
}

// --- Target profiles (for the device topology panel) -----------------------

export interface Topology {
  qubit_count: number;
  edges: [number, number][];
}

export interface BasisProfile {
  id: string;
  version: string;
  backend_id: string;
  topology: Topology;
  supported_operations: string[];
  decomposition_rules: string[];
  cost_model_id: string;
  [key: string]: unknown;
}

// --- Wire envelope -----------------------------------------------------------

export type ServerMessage =
  | { kind: "pipeline_report"; sequence: number; report: PipelineReport }
  | { kind: "pass_replay"; sequence: number; replay: PassReplay }
  | { kind: "lowering_replay"; sequence: number; replay: LoweringReplay }
  | { kind: "compile_error"; sequence: number; message: string };

export type ClientMessage =
  | { kind: "watch_file"; path: string }
  | { kind: "compile_source"; source: string; name: string };
