//! The executable representation a backend actually runs.
//!
//! # Why this is not QIR
//!
//! Stage C §5 is explicit: the textual QIR emitter "is useful as a
//! lowering/output artifact, but … does not itself guarantee IBM execution
//! compatibility", and the specification forbids documenting "QIR emitted =
//! executable". §33.12 goes further — do not claim hardware executability
//! without target-specific lowering and validation. So QIR and the executable
//! representation are deliberately different artifacts produced by different
//! stages, and an [`Executable`] can only be built from a [`Lowered`] circuit
//! that a target has already validated.
//!
//! # Why this is not OpenQASM 3 either
//!
//! Handing the adapter OpenQASM text would have been the obvious choice, and
//! it does not work: `qiskit.qasm3.loads` raises `MissingOptionalLibraryError`
//! unless the separate `qiskit_qasm3_import` package is installed, which it is
//! not in this project's environment. Adding a third-party parser to the trust
//! path between "circuit OQCI verified" and "circuit that runs" would also
//! mean a parser bug could silently change the program after verification.
//!
//! So the representation is a structured instruction list, and the adapter
//! replays it by calling one method per operation. Every [`crate::ir::GateKind`]
//! except `Opaque` maps one-to-one onto a `QuantumCircuit` method on the
//! Qiskit version this project pins — checked by introspection, not assumed
//! (§33.3). `Opaque` has no such mapping and is refused when the executable is
//! built, not at run time.

use serde::{Deserialize, Serialize};

use crate::ir::{GateKind, Instruction, Param};
use crate::lowering::Lowered;

use super::BackendError;
use super::result::Provenance;

/// One operation, flattened for transport.
///
/// The IR types stay serde-free — their shape is a compiler contract governed
/// by `docs/ir_spec.md`, and it should not acquire a wire format by accident.
/// This is that wire format, defined where it belongs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutableOp {
    /// The operation's mnemonic, as the adapter's method name.
    pub op: String,
    /// Physical qubit operands.
    pub qubits: Vec<u32>,
    /// Concrete parameters, in radians.
    ///
    /// Always concrete: an executable cannot carry a symbol, because no
    /// execution API accepts one. [`Executable::from_lowered`] refuses a
    /// circuit that still has any.
    pub params: Vec<f64>,
    /// The classical bit a measurement writes to.
    pub clbit: Option<u32>,
}

/// A circuit prepared for a specific backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Executable {
    /// Which backend this was prepared for. An executable is not portable,
    /// and saying so in the artifact prevents it being handed to the wrong
    /// one.
    pub backend_id: String,
    /// Physical qubits the circuit uses.
    pub num_qubits: u32,
    /// Classical bits it writes.
    pub num_clbits: u32,
    /// The operations, in order.
    pub ops: Vec<ExecutableOp>,
    /// Shots, seed and so on.
    pub settings: super::result::ExecutionSettings,
    /// Everything needed to cite a result produced from this.
    pub provenance: Provenance,
}

impl Executable {
    /// Builds an executable from a lowered, validated circuit.
    ///
    /// # Errors
    ///
    /// [`BackendError::NotExecutable`] if the circuit still contains a
    /// symbolic parameter or an opaque operation. Both are refusals rather
    /// than best-effort translations: a symbol has no value any execution API
    /// accepts, and an opaque gate has no known matrix, so guessing either
    /// would produce an artifact that runs and computes the wrong thing.
    pub fn from_lowered(
        lowered: &Lowered,
        backend_id: &str,
        settings: super::result::ExecutionSettings,
        provenance: Provenance,
    ) -> Result<Self, BackendError> {
        let mut ops = Vec::with_capacity(lowered.circuit.len());

        for (index, instruction) in lowered.circuit.instructions().iter().enumerate() {
            let op = match instruction {
                Instruction::Gate { kind, qubits } => {
                    if let GateKind::Opaque { name, .. } = kind {
                        return Err(BackendError::NotExecutable {
                            index,
                            detail: format!(
                                "opaque operation `{name}` has no known implementation"
                            ),
                        });
                    }
                    let mut params = Vec::new();
                    for param in kind.params() {
                        match param {
                            Param::Concrete(angle) => params.push(angle.radians()),
                            Param::Symbol(symbol) => {
                                return Err(BackendError::NotExecutable {
                                    index,
                                    detail: format!(
                                        "parameter `{symbol}` is still symbolic; \
                                         bind parameters before preparing for execution"
                                    ),
                                });
                            }
                        }
                    }
                    ExecutableOp {
                        op: kind.mnemonic().to_string(),
                        qubits: qubits.iter().map(|q| q.index()).collect(),
                        params,
                        clbit: None,
                    }
                }
                Instruction::Measure { qubit, target } => ExecutableOp {
                    op: "measure".to_string(),
                    qubits: vec![qubit.index()],
                    params: Vec::new(),
                    clbit: Some(target.index()),
                },
                Instruction::Reset { qubit } => ExecutableOp {
                    op: "reset".to_string(),
                    qubits: vec![qubit.index()],
                    params: Vec::new(),
                    clbit: None,
                },
            };
            ops.push(op);
        }

        Ok(Executable {
            backend_id: backend_id.to_string(),
            num_qubits: lowered.circuit.num_qubits(),
            num_clbits: lowered.circuit.num_clbits(),
            ops,
            settings,
            provenance,
        })
    }

    /// Whether this circuit measures anything.
    ///
    /// A circuit with no measurement produces no counts, which is worth
    /// catching before a backend runs it and returns an empty result that
    /// looks like a failure.
    #[must_use]
    pub fn has_measurement(&self) -> bool {
        self.ops.iter().any(|op| op.op == "measure")
    }

    /// The distinct operations this executable uses, ascending.
    ///
    /// Lets an adapter check it can implement everything *before* running
    /// anything, rather than failing part way through a circuit.
    #[must_use]
    pub fn operations(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.ops.iter().map(|op| op.op.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CircuitBuilder, GateKind, Param};
    use crate::lowering::{LoweringConfig, lower};
    use crate::target::builtin;

    fn provenance() -> Provenance {
        let (version, commit) = Provenance::compiler_identity();
        Provenance {
            circuit: "t".into(),
            compiler_version: version,
            git_commit: commit,
            backend_id: "simulator".into(),
            profile_id: "ideal-simulator@1".into(),
            cost_model_id: "uniform".into(),
            cost_model_version: "1".into(),
            cost_model_configuration: Default::default(),
            pass_pipeline: vec![],
            lowering_steps: vec![],
            decomposition_rules: vec![],
            initial_layout: vec![],
            final_layout: vec![],
            swaps_inserted: 0,
            shots: 10,
            seed: None,
        }
    }

    fn prepare(build: impl FnOnce(&mut CircuitBuilder)) -> Result<Executable, BackendError> {
        let mut b = CircuitBuilder::new("t");
        b.alloc_qubits(2);
        b.alloc_clbits(2);
        build(&mut b);
        let lowered = lower(
            &b.build().unwrap(),
            &builtin::ideal_simulator(),
            &LoweringConfig::default(),
        )
        .unwrap();
        Executable::from_lowered(&lowered, "simulator", Default::default(), provenance())
    }

    #[test]
    fn a_measured_circuit_becomes_a_replayable_op_list() {
        let executable = prepare(|b| {
            b.h(crate::ir::QubitId(0))
                .cx(crate::ir::QubitId(0), crate::ir::QubitId(1))
                .measure(crate::ir::QubitId(0), crate::ir::ClbitId(0));
        })
        .unwrap();

        assert_eq!(executable.operations(), vec!["cx", "h", "measure"]);
        assert!(executable.has_measurement());
        assert_eq!(executable.ops[2].clbit, Some(0));
        assert_eq!(executable.num_qubits, 2);
    }

    #[test]
    fn parameters_are_carried_as_concrete_radians() {
        let executable = prepare(|b| {
            b.rz(0.75, crate::ir::QubitId(0));
        })
        .unwrap();
        assert_eq!(executable.ops[0].op, "rz");
        assert!((executable.ops[0].params[0] - 0.75).abs() < 1e-12);
    }

    #[test]
    fn a_symbolic_parameter_is_refused_with_advice() {
        // An executable cannot carry a symbol: no execution API accepts one,
        // and substituting a value here would silently run a different
        // circuit from the one the user wrote.
        let err = prepare(|b| {
            b.rz(Param::symbol("theta"), crate::ir::QubitId(0));
        })
        .unwrap_err();
        assert!(
            err.to_string().contains("bind parameters"),
            "the error should say what to do: {err}"
        );
    }

    #[test]
    fn an_opaque_operation_is_refused_rather_than_passed_through() {
        let mut b = CircuitBuilder::new("t");
        b.alloc_qubits(2);
        b.gate(
            GateKind::Opaque {
                name: "iswap".into(),
                params: vec![],
            },
            [crate::ir::QubitId(0), crate::ir::QubitId(1)],
        );
        // Lower against a profile that tolerates it, so the refusal under
        // test is the executable's and not lowering's.
        let profile = crate::target::BasisProfileBuilder::new(
            "exotic",
            "1",
            "b",
            crate::target::Topology::all_to_all(2),
        )
        .operations(["iswap"])
        .cost_model("uniform")
        .build()
        .unwrap();
        let lowered = lower(&b.build().unwrap(), &profile, &LoweringConfig::default()).unwrap();

        let err = Executable::from_lowered(&lowered, "simulator", Default::default(), provenance())
            .unwrap_err();
        assert!(
            matches!(err, BackendError::NotExecutable { .. }),
            "got {err}"
        );
    }

    #[test]
    fn an_unmeasured_circuit_is_flagged_before_anything_runs() {
        let executable = prepare(|b| {
            b.h(crate::ir::QubitId(0));
        })
        .unwrap();
        assert!(!executable.has_measurement());
    }

    #[test]
    fn an_executable_round_trips_through_json() {
        let executable = prepare(|b| {
            b.h(crate::ir::QubitId(0))
                .measure(crate::ir::QubitId(0), crate::ir::ClbitId(0));
        })
        .unwrap();
        let json = serde_json::to_string(&executable).unwrap();
        let parsed: Executable = serde_json::from_str(&json).unwrap();
        assert_eq!(executable, parsed);
    }

    #[test]
    fn an_executable_names_the_backend_it_was_prepared_for() {
        // An executable is not portable between targets, and the artifact
        // says so rather than leaving it to convention.
        let executable = prepare(|b| {
            b.h(crate::ir::QubitId(0));
        })
        .unwrap();
        assert_eq!(executable.backend_id, "simulator");
    }
}
