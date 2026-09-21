//! Making two-qubit operations local, and making them face the right way.
//!
//! §8.7 requires routing to detect non-local two-qubit operations, select a
//! strategy, insert SWAPs, update the mapping, maintain correctness and report
//! the overhead. This module does all six, with one implementation of the
//! strategy: deterministic shortest path, no lookahead.
//!
//! # Insertion-only, in program order — and why that is the correctness proof
//!
//! [`ShortestPathRouter`] walks the instruction list in program order. It
//! deletes no operation, reorders no pair of original operations, and only
//! *inserts* `Swap`s between existing instructions.
//!
//! That single sentence discharges §33.14, the rule that no operation may be
//! optimized away across a measurement or reset barrier without proving the
//! transformation safe. Nothing is optimized away and nothing moves past
//! anything, so every `DepKind::Control` edge of the input DAG survives into
//! the output. There is no proof obligation left.
//!
//! A router with lookahead would produce shorter circuits and would owe that
//! proof. The simplicity here *is* the correctness argument, not a limitation
//! to apologise for — though the extra SWAPs are real, and
//! `docs/lowering.md` says so.
//!
//! # Operand positions, not `control()`/`target()`
//!
//! Routing reads `qubits[0]` and `qubits[1]` directly. [`Instruction::control`]
//! and [`Instruction::target`] answer only for `Cx`/`Cy`/`Cz`/`Ccx`; they
//! return nothing for `Swap` and for a two-qubit `Opaque`. A router keyed off
//! them would silently skip the very gates it inserts.
//! [`crate::target::check`] reads operand positions for the same reason, so
//! the two agree by construction.

use std::collections::BTreeSet;

use crate::ir::{Circuit, GateKind, Instruction, QubitId};
use crate::lowering::layout::Layout;
use crate::lowering::{LoweringError, UnroutableReason};
use crate::target::{BasisProfile, CouplingMode, PhysicalQubit};

/// How a reversed two-qubit operation is made native.
///
/// Stage D §7: "If a reverse interaction can be implemented by basis changes
/// or conjugation, encode the transformation in the target-specific lowering
/// rules rather than silently reversing operands."
///
/// Declared per operation, never inferred. `Cz` and `Swap` genuinely are
/// symmetric; `Cx` and `Cy` are not. Guessing wrong is silent — a swapped
/// control and target agree on every computational basis state and disagree
/// on superpositions — so anything unverified is [`OrientationPolicy::None`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrientationPolicy {
    /// The operation is unchanged by exchanging its operands, so repair is a
    /// pure relabelling with no added gates.
    Symmetric,
    /// Reversed by conjugating **both** operands with this one-qubit gate.
    ///
    /// For `Cx` the gate is `h`: `CX(b,a) = (H⊗H) CX(a,b) (H⊗H)`, exactly.
    ConjugateBoth(&'static str),
    /// No verified repair. An edge usable only in the wrong direction is then
    /// not usable at all, and routing says so rather than emitting something
    /// it cannot justify.
    None,
}

/// The repair available for an operation, if any.
///
/// Every entry is an identity checked against **two** independent
/// implementations of gate semantics: the project's own state-vector harness,
/// by
/// `tests/lowering.rs::the_declared_orientation_policies_are_the_identities_they_claim`,
/// and Qiskit's `quantum_info.Operator`, by
/// `python/tests/test_rules.py::test_the_orientation_repair_identity_holds`.
/// Both check the negative case too — that `Cx` is *not* symmetric — since
/// that is the reason a repair is needed at all.
#[must_use]
pub fn orientation_policy(kind: &GateKind) -> OrientationPolicy {
    match kind {
        // Exchanging the operands of a controlled-Z or a swap gives the same
        // matrix back.
        GateKind::Cz | GateKind::Swap => OrientationPolicy::Symmetric,
        GateKind::Cx => OrientationPolicy::ConjugateBoth("h"),
        _ => OrientationPolicy::None,
    }
}

/// How a router turned a circuit into a local one.
#[derive(Debug, Clone)]
pub struct RoutingOutput {
    /// Instructions over **physical** qubit indices.
    pub instructions: Vec<Instruction>,
    /// Where each logical qubit started.
    pub initial_layout: Layout,
    /// Where each logical qubit ended up, after every inserted `Swap`.
    pub final_layout: Layout,
    /// SWAPs inserted — §8.7's "report routing overhead".
    pub swaps_inserted: usize,
}

/// How connectivity repair is chosen.
pub trait RoutingStrategy: Send + Sync {
    /// Stable identifier, recorded in lowering output.
    fn id(&self) -> &'static str;

    /// Routes a circuit onto physical qubits.
    ///
    /// # Errors
    ///
    /// [`LoweringError::Unroutable`] when no sequence of SWAPs can make an
    /// operation's operands adjacent.
    fn route(
        &self,
        circuit: &Circuit,
        layout: Layout,
        profile: &BasisProfile,
        can_reverse: bool,
    ) -> Result<RoutingOutput, LoweringError>;
}

/// Deterministic shortest-path routing.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShortestPathRouter;

impl RoutingStrategy for ShortestPathRouter {
    fn id(&self) -> &'static str {
        "shortest-path"
    }

    fn route(
        &self,
        circuit: &Circuit,
        layout: Layout,
        profile: &BasisProfile,
        can_reverse: bool,
    ) -> Result<RoutingOutput, LoweringError> {
        let topology = profile.topology();
        let mode = routing_mode(can_reverse);
        let freeze_measured = !profile.measurement().mid_circuit_measurement;

        let initial_layout = layout.clone();
        let mut current = layout;
        let mut out: Vec<Instruction> = Vec::with_capacity(circuit.len());
        let mut swaps = 0usize;

        // Physical qubits that may no longer be touched by a `Swap`.
        //
        // When the device cannot measure mid-circuit, a measured wire must
        // stay untouched for the rest of the program. Without this, routing
        // could park another logical qubit on a measured wire, or route a
        // path straight through it — and `check` would then report
        // `MidCircuitMeasurementUnsupported` against the *measurement*,
        // blaming an instruction that was fine when it was written.
        let mut frozen: BTreeSet<PhysicalQubit> = BTreeSet::new();

        for (index, instruction) in circuit.instructions().iter().enumerate() {
            match instruction {
                Instruction::Gate { kind, qubits } if qubits.len() == 2 => {
                    let (mut a, mut b) = (
                        physical(&current, qubits[0], index)?,
                        physical(&current, qubits[1], index)?,
                    );

                    if !topology.couples(a, b, mode) {
                        // Routing *around* frozen wires, not filtering a
                        // single best path against them. On a ring two paths
                        // can be equally short, and discarding the first
                        // because it crosses a measured wire would refuse a
                        // circuit the second handles perfectly well.
                        let Some(path) = topology.shortest_path_avoiding(a, b, mode, &frozen)
                        else {
                            // Two failures with different fixes, so they get
                            // different reasons: an unreachable pair is a
                            // property of the device, while a frozen wire is a
                            // consequence of where this program measures. If a
                            // path exists once the freezing is ignored, the
                            // freezing is what blocked it.
                            let reason = if topology.shortest_path(a, b, mode).is_some() {
                                UnroutableReason::MeasurementFrozenWire
                            } else {
                                UnroutableReason::NoPath
                            };
                            return Err(LoweringError::Unroutable {
                                index,
                                from: a,
                                to: b,
                                reason,
                            });
                        };

                        // Walk the first operand along the path until it sits
                        // next to the second. The choice of *which* endpoint
                        // moves is part of the contract: moving the other one
                        // gives a different final layout for the same input.
                        for window in path.windows(2).take(path.len().saturating_sub(2)) {
                            let (from, to) = (window[0], window[1]);
                            out.push(swap_instruction(from, to, topology));
                            current.swap_physical(from, to);
                            swaps += 1;
                        }
                        a = physical(&current, qubits[0], index)?;
                        b = physical(&current, qubits[1], index)?;
                        debug_assert!(
                            topology.couples(a, b, mode),
                            "routing finished with non-adjacent operands"
                        );
                    }

                    out.push(Instruction::Gate {
                        kind: kind.clone(),
                        qubits: vec![QubitId(a.index()), QubitId(b.index())],
                    });
                }
                Instruction::Gate { kind, qubits } => {
                    // One-qubit gates need no repair; wider ones should not
                    // have survived arity reduction, and `lower` asserts that
                    // separately rather than silently accepting one here.
                    let mapped: Result<Vec<QubitId>, LoweringError> = qubits
                        .iter()
                        .map(|q| physical(&current, *q, index).map(|p| QubitId(p.index())))
                        .collect();
                    out.push(Instruction::Gate {
                        kind: kind.clone(),
                        qubits: mapped?,
                    });
                }
                Instruction::Measure { qubit, target } => {
                    let physical_qubit = physical(&current, *qubit, index)?;
                    if freeze_measured {
                        frozen.insert(physical_qubit);
                    }
                    // Classical bits are not remapped. Layout is a statement
                    // about qubits; a measurement's destination register is
                    // untouched, and `num_clbits` never changes.
                    out.push(Instruction::Measure {
                        qubit: QubitId(physical_qubit.index()),
                        target: *target,
                    });
                }
                Instruction::Reset { qubit } => {
                    let physical_qubit = physical(&current, *qubit, index)?;
                    out.push(Instruction::Reset {
                        qubit: QubitId(physical_qubit.index()),
                    });
                }
            }
        }

        Ok(RoutingOutput {
            instructions: out,
            initial_layout,
            final_layout: current,
            swaps_inserted: swaps,
        })
    }
}

/// Which graph SWAP paths may be planned on.
///
/// A `Swap` is three CNOTs in alternating directions, so it needs *both*
/// orientations of an edge no matter which operand order it is written in.
/// A one-way edge is therefore usable only when a reversed CNOT can be
/// repaired — which is a property of the target's rule closure, not of its
/// basis list, and so is decided by the caller.
fn routing_mode(can_reverse: bool) -> CouplingMode {
    if can_reverse {
        CouplingMode::Undirected
    } else {
        CouplingMode::Symmetric
    }
}

/// A `Swap` written with operands in an order the device declares.
///
/// `check` reads `qubits[0]` as the control of *any* two-qubit operation,
/// `Swap` included, so a swap emitted in the undeclared order is reported as
/// a connectivity violation even though the operation is symmetric. Emitting
/// it the right way round is cheaper than teaching `check` about symmetry,
/// and keeps `check` conservative.
fn swap_instruction(
    from: PhysicalQubit,
    to: PhysicalQubit,
    topology: &crate::target::Topology,
) -> Instruction {
    let (first, second) = if topology.supports(from, to) {
        (from, to)
    } else {
        (to, from)
    };
    Instruction::Gate {
        kind: GateKind::Swap,
        qubits: vec![QubitId(first.index()), QubitId(second.index())],
    }
}

fn physical(
    layout: &Layout,
    logical: QubitId,
    index: usize,
) -> Result<PhysicalQubit, LoweringError> {
    layout
        .physical(logical)
        .ok_or(LoweringError::UnmappedQubit {
            index,
            qubit: logical,
        })
}

/// Repairs two-qubit operations whose operand order the device does not
/// declare.
///
/// A single sweep, not a fixed point: the `Cx` this emits is natively
/// oriented by construction, so it can never need repairing again. The other
/// thing it emits is a one-qubit conjugation gate, which the single-qubit
/// cleanup phase then lowers. That is why the lowering schedule is a
/// sequence rather than a loop — see [`crate::lowering`].
///
/// # Errors
///
/// [`LoweringError::UnrepairableOrientation`] when an operation sits on an
/// edge declared only in the opposite direction and has no known repair.
pub fn repair_orientation(
    instructions: &[Instruction],
    profile: &BasisProfile,
) -> Result<(Vec<Instruction>, usize), LoweringError> {
    let topology = profile.topology();
    let mut out = Vec::with_capacity(instructions.len());
    let mut repaired = 0usize;

    for (index, instruction) in instructions.iter().enumerate() {
        let Instruction::Gate { kind, qubits } = instruction else {
            out.push(instruction.clone());
            continue;
        };
        if qubits.len() != 2 {
            out.push(instruction.clone());
            continue;
        }

        let (a, b) = (
            PhysicalQubit(qubits[0].index()),
            PhysicalQubit(qubits[1].index()),
        );
        if topology.supports(a, b) {
            out.push(instruction.clone());
            continue;
        }
        if !topology.supports(b, a) {
            // Not an orientation problem — the pair is not coupled at all.
            // Routing should have prevented this; `lower`'s verification step
            // reports it if it did not.
            out.push(instruction.clone());
            continue;
        }

        match orientation_policy(kind) {
            OrientationPolicy::Symmetric => {
                out.push(Instruction::Gate {
                    kind: kind.clone(),
                    qubits: vec![qubits[1], qubits[0]],
                });
                repaired += 1;
            }
            OrientationPolicy::ConjugateBoth(gate) => {
                let conjugation = crate::frontend::map_gate(gate, vec![]).map_err(|source| {
                    LoweringError::UnrepairableOrientation {
                        index,
                        mnemonic: kind.mnemonic().to_string(),
                        detail: source.to_string(),
                    }
                })?;
                for qubit in [qubits[0], qubits[1]] {
                    out.push(Instruction::Gate {
                        kind: conjugation.clone(),
                        qubits: vec![qubit],
                    });
                }
                out.push(Instruction::Gate {
                    kind: kind.clone(),
                    qubits: vec![qubits[1], qubits[0]],
                });
                for qubit in [qubits[0], qubits[1]] {
                    out.push(Instruction::Gate {
                        kind: conjugation.clone(),
                        qubits: vec![qubit],
                    });
                }
                repaired += 1;
            }
            OrientationPolicy::None => {
                return Err(LoweringError::UnrepairableOrientation {
                    index,
                    mnemonic: kind.mnemonic().to_string(),
                    detail: format!(
                        "the device declares {b}->{a} but not {a}->{b}, \
                         and no verified reversal exists for this operation"
                    ),
                });
            }
        }
    }
    Ok((out, repaired))
}
