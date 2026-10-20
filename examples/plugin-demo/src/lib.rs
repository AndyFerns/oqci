//! A demonstration OQCI extension (`final-deliverables-spec.md` §18).
//!
//! Three components, each added through the public extension API and nothing
//! else — this crate depends on `oqci` like any third party would, so it
//! cannot reach a single `pub(crate)` item:
//!
//! - [`HzhToX`], an optimization **pass** (§18.2): rewrites `H; Z; H` on one
//!   qubit into `X`, which is exact (`HZH = X`).
//! - [`Qtoy`], a **frontend** (§18.1): a tiny line-based language, read into
//!   QC-IR through the same `CircuitBuilder` and `map_gate` every built-in
//!   frontend uses.
//! - [`demo_ring`], a **backend** (§18.3): a synthetic six-qubit ring with a
//!   CZ-native basis, supplying its own profile, lowering, execution
//!   interface and cost model.
//!
//! [`register`] adds all three to a [`Registry`]. `src/main.rs` builds an
//! `oqci-demo` binary that is the `oqci` CLI plus these extensions.

use std::sync::Arc;

use oqci::backend::{
    self, Backend, BackendError, Executable, ExecutionResult, ExecutionSettings, Provenance,
};
use oqci::extension::{ExtensionError, ExtensionOrigin, FrontendPlugin, PassPlacement, Registry};
use oqci::frontend::{FrontendError, map_gate};
use oqci::ir::{Circuit, CircuitBuilder, ClbitId, GateKind, Instruction, Param, QubitId};
use oqci::lowering::Lowered;
use oqci::pass::{Pass, PassContext, PassError, PassOutput};
use oqci::target::{
    BasisProfile, BasisProfileBuilder, CostModel, MeasurementSupport, PhysicalQubit, Topology,
    WeightedCostModel, builtin, cost,
};

/// Who these extensions say they are, in every provenance record they enter.
#[must_use]
pub fn origin() -> ExtensionOrigin {
    ExtensionOrigin::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

/// Registers the demonstration pass, frontend and backend.
///
/// # Errors
///
/// [`ExtensionError`] if any id is already taken in `registry`.
pub fn register(registry: &mut Registry) -> Result<(), ExtensionError> {
    // Before fusion: `single-qubit-fusion` would otherwise absorb the H;Z;H
    // into a `U` first, and this pass would never see one.
    registry
        .register_pass(
            origin(),
            PassPlacement::BeforeFirst("single-qubit-fusion".to_string()),
            Arc::new(HzhToX),
        )?
        .register_frontend(origin(), Arc::new(Qtoy))?
        .register_backend(origin(), Arc::new(demo_ring()))?;
    Ok(())
}

// --- The pass ----------------------------------------------------------------

/// Rewrites `H; Z; H` on one qubit into `X`.
///
/// The three gates must be consecutive *on their wire*: no instruction in
/// between may touch that qubit (gates on other qubits may interleave). That
/// also keeps the rewrite from crossing a measurement or reset.
pub struct HzhToX;

impl Pass for HzhToX {
    fn id(&self) -> &'static str {
        "hzh-to-x"
    }

    fn description(&self) -> &'static str {
        "rewrite H; Z; H on one qubit into X (demo extension)"
    }

    fn run(&self, circuit: &Circuit, _context: &PassContext<'_>) -> Result<PassOutput, PassError> {
        let instructions = circuit.instructions();
        // For each qubit, the indices of the instructions touching it, in order.
        let mut on_wire: Vec<Vec<usize>> = vec![Vec::new(); circuit.num_qubits() as usize];
        for (index, instruction) in instructions.iter().enumerate() {
            for qubit in instruction.qubits() {
                on_wire[qubit.0 as usize].push(index);
            }
        }
        let single = |index: usize, want: &GateKind| {
            matches!(&instructions[index], Instruction::Gate { kind, qubits }
                if qubits.len() == 1 && kind == want)
        };

        let mut replace: Vec<Option<bool>> = vec![None; instructions.len()]; // Some(true): becomes X
        let mut rewrites = 0;
        for wire in &on_wire {
            let mut i = 0;
            while i + 2 < wire.len() {
                let (a, b, c) = (wire[i], wire[i + 1], wire[i + 2]);
                let free = [a, b, c].iter().all(|x| replace[*x].is_none());
                if free
                    && single(a, &GateKind::H)
                    && single(b, &GateKind::Z)
                    && single(c, &GateKind::H)
                {
                    replace[a] = Some(false);
                    replace[b] = Some(false);
                    replace[c] = Some(true);
                    rewrites += 1;
                    i += 3;
                } else {
                    i += 1;
                }
            }
        }
        if rewrites == 0 {
            return Ok(PassOutput::unchanged(circuit.clone()));
        }

        // Rebuilt through the public builder, so the result is re-validated
        // against I1-I8 like any built-in pass's output.
        let mut builder = CircuitBuilder::new(circuit.name());
        builder.alloc_qubits(circuit.num_qubits());
        builder.alloc_clbits(circuit.num_clbits());
        for (index, instruction) in instructions.iter().enumerate() {
            match (replace[index], instruction) {
                (Some(false), _) => {}
                (Some(true), Instruction::Gate { qubits, .. }) => {
                    builder.gate(GateKind::X, qubits.clone());
                }
                (_, Instruction::Gate { kind, qubits }) => {
                    builder.gate(kind.clone(), qubits.clone());
                }
                (_, Instruction::Measure { qubit, target }) => {
                    builder.measure(*qubit, *target);
                }
                (_, Instruction::Reset { qubit }) => {
                    builder.reset(*qubit);
                }
            }
        }
        Ok(PassOutput {
            circuit: builder
                .build()
                .map_err(|e| PassError::from_ir(self.id(), e))?,
            changed: true,
            notes: vec![format!("rewrote {rewrites} H;Z;H triple(s) into X")],
        })
    }
}

// --- The frontend ------------------------------------------------------------

/// `qtoy`, a line-based circuit language:
///
/// ```text
/// # a comment
/// qubits 2
/// bits 2
/// h 0
/// cx 0 1
/// rz(0.5) 1
/// measure 0 -> 0
/// ```
///
/// Gate names and parameter counts are resolved by `map_gate`, so `qtoy`
/// accepts exactly the registered gate set and nothing more. Its files end in
/// `.qtoy`.
pub struct Qtoy;

impl FrontendPlugin for Qtoy {
    fn id(&self) -> &str {
        "qtoy"
    }

    fn description(&self) -> &str {
        "a line-based demo language (demo extension)"
    }

    fn file_extensions(&self) -> &[&str] {
        &["qtoy"]
    }

    fn parse(&self, source: &str, name: &str) -> Result<Circuit, FrontendError> {
        let mut builder = CircuitBuilder::new(name);
        let (mut qubits, mut bits) = (None::<u32>, None::<u32>);

        for (number, raw) in source.lines().enumerate() {
            let line_no = u32::try_from(number + 1).unwrap_or(u32::MAX);
            let line = raw.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let syntax = |message: &str| FrontendError::syntax(message, line_no, 1);
            let mut words = line.split_whitespace();
            let head = words.next().expect("a non-empty line has a first word");
            let count = |words: &mut std::str::SplitWhitespace<'_>| -> Result<u32, FrontendError> {
                words
                    .next()
                    .and_then(|w| w.parse().ok())
                    .ok_or_else(|| syntax("expected a count"))
            };

            match head {
                "qubits" => {
                    let n = count(&mut words)?;
                    builder.alloc_qubits(n);
                    qubits = Some(n);
                }
                "bits" => {
                    let n = count(&mut words)?;
                    builder.alloc_clbits(n);
                    bits = Some(n);
                }
                "measure" => {
                    let qubit = index(words.next(), qubits, line_no)?;
                    if words.next() != Some("->") {
                        return Err(syntax("expected `measure Q -> C`"));
                    }
                    let clbit = index(words.next(), bits, line_no)?;
                    builder.measure(QubitId(qubit), ClbitId(clbit));
                }
                gate => {
                    let (name, params) = match gate.split_once('(') {
                        None => (gate, Vec::new()),
                        Some((name, rest)) => {
                            let inner = rest
                                .strip_suffix(')')
                                .ok_or_else(|| syntax("unclosed parameter list"))?;
                            let params = inner
                                .split(',')
                                .map(|p| {
                                    p.trim()
                                        .parse::<f64>()
                                        .map(Param::concrete)
                                        .map_err(|_| syntax("parameters must be numbers"))
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            (name, params)
                        }
                    };
                    let kind = map_gate(name, params)?;
                    if matches!(kind, GateKind::Opaque { .. }) {
                        return Err(FrontendError::unsupported(format!(
                            "gate `{name}` is not in the registered gate set"
                        )));
                    }
                    let operands = words
                        .map(|w| index(Some(w), qubits, line_no).map(QubitId))
                        .collect::<Result<Vec<_>, _>>()?;
                    builder.gate(kind, operands);
                }
            }
        }
        Ok(builder.build()?)
    }
}

fn index(word: Option<&str>, declared: Option<u32>, line: u32) -> Result<u32, FrontendError> {
    let value: u32 = word
        .and_then(|w| w.parse().ok())
        .ok_or_else(|| FrontendError::syntax("expected an index", line, 1))?;
    match declared {
        Some(n) if value < n => Ok(value),
        Some(n) => Err(FrontendError::semantic(format!(
            "index {value} is out of range for a register of {n} on line {line}"
        ))),
        None => Err(FrontendError::semantic(format!(
            "line {line} uses a register before declaring it"
        ))),
    }
}

// --- The backend -------------------------------------------------------------

/// The demonstration backend: a synthetic six-qubit CZ-native ring.
pub struct DemoRing {
    profile: BasisProfile,
    cost_model: WeightedCostModel,
}

/// Builds [`DemoRing`]. It describes no real device, and says so.
///
/// # Panics
///
/// Never: the profile is well-formed by construction, checked by a test.
#[must_use]
pub fn demo_ring() -> DemoRing {
    let mut topology = Topology::disconnected(6);
    for q in 0..6 {
        topology.add_undirected(PhysicalQubit(q), PhysicalQubit((q + 1) % 6));
    }
    let profile = BasisProfileBuilder::new("demo-ring", "1", "demo", topology)
        .operations(["rz", "sx", "x", "cz", "measure"])
        .measurement(MeasurementSupport {
            measurement: true,
            mid_circuit_measurement: false,
            reset: false,
        })
        .decomposition_rules(builtin::CZ_BASIS_RULES.iter().copied())
        .capability("synthetic-not-a-real-device")
        .cost_model("uniform")
        .build()
        .expect("the demo profile is well-formed");
    let cost_model = cost::resolve("uniform").expect("`uniform` is a built-in cost model");
    DemoRing {
        profile,
        cost_model,
    }
}

impl Backend for DemoRing {
    fn id(&self) -> &str {
        "demo-ring"
    }

    fn description(&self) -> &str {
        "synthetic 6-qubit CZ-native ring (demo extension); describes no real device"
    }

    fn profile(&self) -> &BasisProfile {
        &self.profile
    }

    fn cost_model(&self) -> &dyn CostModel {
        &self.cost_model
    }

    fn prepare(
        &self,
        lowered: &Lowered,
        settings: &ExecutionSettings,
        provenance: Provenance,
    ) -> Result<Executable, BackendError> {
        backend::validated(self, lowered)?;
        Executable::from_lowered(lowered, self.id(), settings.clone(), provenance)
    }

    fn execute(&self, _executable: &Executable) -> Result<ExecutionResult, BackendError> {
        Err(BackendError::ExecutionNotAvailableInProcess {
            backend: self.id().to_string(),
            reason: "the demo backend prepares executables only; run them with an execution \
                     adapter such as oqci.backends.aer"
                .to_string(),
        })
    }
}
