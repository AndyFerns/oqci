//! Where a circuit's logical qubits sit on the device.
//!
//! Stage D §6 lists "logical qubit IDs, physical qubit IDs, initial layout,
//! layout updates, … final layout/reporting" as *distinct* required concepts,
//! and this module keeps them distinct. Until now the compiler has had only an
//! implicit identity layout — [`crate::target::check`] reads logical qubit `n`
//! as physical qubit `n` because there was nothing better to read. That
//! assumption is now a named strategy ([`TrivialLayout`]) rather than an
//! unstated convention.
//!
//! # A layout is injective, not bijective
//!
//! A circuit usually has fewer qubits than the device, so the mapping is
//! logical → physical and injective: every logical qubit has exactly one
//! physical home, and physical qubits with no tenant are ancillas that stay in
//! `|0>`. Nothing may ever map two logical qubits to the same physical one,
//! which is why [`Layout::from_pairs`] rejects that rather than trusting its
//! caller.
//!
//! # Layout choice cannot make a circuit wrong
//!
//! Worth stating, because it bounds how much damage a bug here can do.
//! Routing repairs whatever connectivity a layout leaves broken, and the
//! verification step re-checks the result against the profile regardless. A
//! bad layout therefore costs SWAPs; it does not produce an illegal or
//! incorrect circuit. [`DenseLayout`] is a heuristic in exactly that sense,
//! and is allowed to be one.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::ir::{Circuit, Instruction, QubitId};
use crate::target::{CouplingMode, PhysicalQubit, Topology};

/// A layout that could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LayoutError {
    /// Two logical qubits were assigned the same physical qubit.
    #[error("logical qubits {first} and {second} were both mapped to {physical}")]
    NotInjective {
        /// The lower-indexed logical qubit.
        first: QubitId,
        /// The logical qubit that collided with it.
        second: QubitId,
        /// The contested physical qubit.
        physical: PhysicalQubit,
    },
    /// A logical qubit was mapped off the device.
    #[error("logical qubit {logical} was mapped to {physical}, but the device has {qubit_count}")]
    PhysicalOutOfRange {
        /// The logical qubit.
        logical: QubitId,
        /// Its out-of-range destination.
        physical: PhysicalQubit,
        /// The device's qubit count.
        qubit_count: u32,
    },
    /// A logical qubit below the highest assigned one was left unmapped.
    #[error("logical qubit {logical} has no physical assignment")]
    Unmapped {
        /// The logical qubit with no home.
        logical: QubitId,
    },
    /// The circuit needs more qubits than the device has.
    #[error("circuit needs {needed} qubits, device has {available}")]
    DeviceTooSmall {
        /// Qubits the circuit declares.
        needed: u32,
        /// Qubits the device has.
        available: u32,
    },
}

/// An injective assignment of logical qubits to physical ones.
///
/// Both directions are stored and kept in step, because routing asks both
/// questions constantly — "where does logical `q` live?" when emitting an
/// operation, and "what lives on physical `p`?" when swapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Indexed by logical qubit.
    to_physical: Vec<PhysicalQubit>,
    /// Indexed by physical qubit; `None` for an unoccupied device qubit.
    to_logical: Vec<Option<QubitId>>,
}

/// Serializes as the forward mapping plus the device width.
///
/// Hand-written rather than derived for two reasons. The inverse table is
/// redundant — it is recoverable from the forward one — so emitting it would
/// put the same fact on the wire twice, and a consumer could then read a
/// self-contradictory layout. And `QubitId` has no `Serialize`: the IR types
/// are deliberately serde-free so their shape stays a compiler contract
/// rather than acquiring a wire format by accident, the same rule
/// `src/target/legality.rs` and `src/cli/snapshot.rs` follow.
impl Serialize for Layout {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut out = serializer.serialize_struct("Layout", 2)?;
        out.serialize_field("device_qubits", &self.device_qubits())?;
        out.serialize_field("logical_to_physical", &self.permutation())?;
        out.end()
    }
}

impl Layout {
    /// Logical `n` on physical `n` — the assumption `check` has always made,
    /// now written down.
    ///
    /// # Errors
    ///
    /// [`LayoutError::DeviceTooSmall`] if the circuit has more qubits than
    /// the device.
    pub fn trivial(num_qubits: u32, device_qubits: u32) -> Result<Self, LayoutError> {
        if num_qubits > device_qubits {
            return Err(LayoutError::DeviceTooSmall {
                needed: num_qubits,
                available: device_qubits,
            });
        }
        Layout::from_pairs(
            (0..num_qubits).map(|q| (QubitId(q), PhysicalQubit(q))),
            device_qubits,
        )
    }

    /// Builds a layout from explicit assignments, validating it.
    ///
    /// # Errors
    ///
    /// [`LayoutError::NotInjective`] if two logical qubits share a physical
    /// one, [`LayoutError::PhysicalOutOfRange`] if an assignment leaves the
    /// device, or [`LayoutError::Unmapped`] if the assignments are not
    /// contiguous from logical qubit zero.
    pub fn from_pairs(
        pairs: impl IntoIterator<Item = (QubitId, PhysicalQubit)>,
        device_qubits: u32,
    ) -> Result<Self, LayoutError> {
        let mut to_logical: Vec<Option<QubitId>> = vec![None; device_qubits as usize];
        let mut assigned: BTreeMap<u32, PhysicalQubit> = BTreeMap::new();

        for (logical, physical) in pairs {
            if physical.index() >= device_qubits {
                return Err(LayoutError::PhysicalOutOfRange {
                    logical,
                    physical,
                    qubit_count: device_qubits,
                });
            }
            if let Some(existing) = to_logical[physical.index() as usize] {
                return Err(LayoutError::NotInjective {
                    first: existing.min(logical),
                    second: existing.max(logical),
                    physical,
                });
            }
            to_logical[physical.index() as usize] = Some(logical);
            assigned.insert(logical.index(), physical);
        }

        // Logical qubits are `0..n`, so the assignments must be too: a gap
        // would mean a qubit in the circuit has nowhere to live.
        let mut to_physical = Vec::with_capacity(assigned.len());
        for logical in 0..assigned.len() as u32 {
            let physical = *assigned.get(&logical).ok_or(LayoutError::Unmapped {
                logical: QubitId(logical),
            })?;
            to_physical.push(physical);
        }

        Ok(Layout {
            to_physical,
            to_logical,
        })
    }

    /// Number of logical qubits this layout places.
    #[must_use]
    pub fn len(&self) -> usize {
        self.to_physical.len()
    }

    /// Whether this layout places no qubits at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.to_physical.is_empty()
    }

    /// Number of physical qubits on the device this layout was built for.
    #[must_use]
    pub fn device_qubits(&self) -> u32 {
        self.to_logical.len() as u32
    }

    /// Where a logical qubit currently lives.
    #[must_use]
    pub fn physical(&self, logical: QubitId) -> Option<PhysicalQubit> {
        self.to_physical.get(logical.index() as usize).copied()
    }

    /// What currently lives on a physical qubit, if anything.
    #[must_use]
    pub fn logical(&self, physical: PhysicalQubit) -> Option<QubitId> {
        self.to_logical
            .get(physical.index() as usize)
            .copied()
            .flatten()
    }

    /// Exchanges whatever occupies two physical qubits — what a routing SWAP
    /// does to the mapping.
    ///
    /// Handles one or both qubits being unoccupied: swapping an ancilla with
    /// a tenant moves the tenant, which is exactly what the corresponding
    /// `Swap` instruction does to the state.
    ///
    /// Injectivity is preserved by construction, since this permutes the
    /// occupancy table — no two logical qubits can come to share a physical
    /// one.
    pub fn swap_physical(&mut self, a: PhysicalQubit, b: PhysicalQubit) {
        if a == b {
            return;
        }
        let (ia, ib) = (a.index() as usize, b.index() as usize);
        self.to_logical.swap(ia, ib);
        if let Some(logical) = self.to_logical[ia] {
            self.to_physical[logical.index() as usize] = a;
        }
        if let Some(logical) = self.to_logical[ib] {
            self.to_physical[logical.index() as usize] = b;
        }
        debug_assert!(self.is_injective(), "swap_physical broke injectivity");
    }

    /// Whether every logical qubit has a distinct physical home, and the two
    /// directions agree.
    ///
    /// Always true for a `Layout` built through this module's constructors.
    /// Asserted in debug builds after each update, so a future change that
    /// breaks the invariant is caught where it happens rather than several
    /// stages downstream.
    #[must_use]
    pub fn is_injective(&self) -> bool {
        self.to_physical.iter().enumerate().all(|(logical, p)| {
            p.index() < self.device_qubits()
                && self.to_logical[p.index() as usize] == Some(QubitId(logical as u32))
        })
    }

    /// The assignment as `logical index -> physical index`, ascending by
    /// logical qubit. Used for reporting, and by the equivalence harness to
    /// relate a routed circuit's wires back to the original's.
    #[must_use]
    pub fn permutation(&self) -> Vec<u32> {
        self.to_physical.iter().map(|p| p.index()).collect()
    }

    /// The highest physical qubit this layout occupies, if any.
    ///
    /// Lowering sizes its output register from this rather than from the
    /// device's full width: a two-qubit program on a 32-qubit simulator
    /// should not become a 32-qubit circuit.
    #[must_use]
    pub fn highest_occupied(&self) -> Option<PhysicalQubit> {
        self.to_physical.iter().copied().max()
    }
}

impl std::fmt::Display for Layout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let body: Vec<String> = self
            .to_physical
            .iter()
            .enumerate()
            .map(|(logical, physical)| format!("%q{logical}->{physical}"))
            .collect();
        write!(f, "{}", body.join(", "))
    }
}

/// How an initial layout is chosen.
///
/// §8.6 requires the mapper to "consult the selected target profile", which is
/// why the topology is a parameter rather than something a strategy infers.
pub trait LayoutStrategy: Send + Sync {
    /// Stable identifier, recorded in lowering output for reproducibility.
    fn id(&self) -> &'static str;

    /// Chooses where each logical qubit starts.
    ///
    /// # Errors
    ///
    /// [`LayoutError`] if no valid assignment exists — in practice only
    /// [`LayoutError::DeviceTooSmall`].
    fn plan(&self, circuit: &Circuit, topology: &Topology) -> Result<Layout, LayoutError>;
}

/// Logical `n` on physical `n`.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrivialLayout;

impl LayoutStrategy for TrivialLayout {
    fn id(&self) -> &'static str {
        "trivial"
    }

    fn plan(&self, circuit: &Circuit, topology: &Topology) -> Result<Layout, LayoutError> {
        Layout::trivial(circuit.num_qubits(), topology.qubit_count())
    }
}

/// Seats interacting logical qubits near each other.
///
/// A heuristic, and only a heuristic: it reduces expected SWAPs and can never
/// make a circuit illegal, because routing repairs whatever it leaves broken.
/// Deterministic — every ordering below breaks ties on the lower index, so the
/// same circuit and topology always produce the same layout.
///
/// The algorithm: order logical qubits by how much two-qubit work they do,
/// then place each on the free physical qubit minimizing its total hop
/// distance to already-placed partners, weighted by how often they interact.
/// The first qubit goes on the best-connected physical qubit.
#[derive(Debug, Clone, Copy, Default)]
pub struct DenseLayout;

impl LayoutStrategy for DenseLayout {
    fn id(&self) -> &'static str {
        "dense"
    }

    fn plan(&self, circuit: &Circuit, topology: &Topology) -> Result<Layout, LayoutError> {
        if circuit.num_qubits() > topology.qubit_count() {
            return Err(LayoutError::DeviceTooSmall {
                needed: circuit.num_qubits(),
                available: topology.qubit_count(),
            });
        }
        let interactions = interaction_counts(circuit);

        // Logical qubits, busiest first, ties on the lower index.
        let mut order: Vec<u32> = (0..circuit.num_qubits()).collect();
        let weight = |q: u32| -> usize {
            interactions
                .iter()
                .filter(|((a, b), _)| *a == q || *b == q)
                .map(|(_, n)| *n)
                .sum()
        };
        order.sort_by_key(|q| (std::cmp::Reverse(weight(*q)), *q));

        let mut placed: BTreeMap<u32, PhysicalQubit> = BTreeMap::new();
        let mut taken = vec![false; topology.qubit_count() as usize];

        for logical in order {
            let Some(best) = (0..topology.qubit_count())
                .map(PhysicalQubit)
                .filter(|p| !taken[p.index() as usize])
                .min_by_key(|candidate| {
                    // Cost of seating `logical` here: hops to each already
                    // placed partner, weighted by interaction count. A
                    // partner in another component is penalized heavily
                    // rather than treated as free.
                    let cost: usize = placed
                        .iter()
                        .map(|(other, other_physical)| {
                            let pair = (logical.min(*other), logical.max(*other));
                            let count = interactions.get(&pair).copied().unwrap_or(0);
                            if count == 0 {
                                return 0;
                            }
                            let hops = topology
                                .distance(*candidate, *other_physical, CouplingMode::Undirected)
                                .unwrap_or(UNREACHABLE_PENALTY);
                            count.saturating_mul(hops)
                        })
                        .sum();
                    // On an empty board this is what decides: prefer the
                    // best-connected qubit.
                    let degree = topology
                        .adjacent(*candidate, CouplingMode::Undirected)
                        .len();
                    (cost, std::cmp::Reverse(degree), candidate.index())
                })
            else {
                // Unreachable: the fit was checked above, so a free physical
                // qubit always exists.
                return Err(LayoutError::DeviceTooSmall {
                    needed: circuit.num_qubits(),
                    available: topology.qubit_count(),
                });
            };
            taken[best.index() as usize] = true;
            placed.insert(logical, best);
        }

        Layout::from_pairs(
            placed.into_iter().map(|(l, p)| (QubitId(l), p)),
            topology.qubit_count(),
        )
    }
}

/// Distance stand-in for a partner in another connected component.
///
/// Large enough to dominate any real distance on a device, without being
/// `usize::MAX`, which would overflow when multiplied by an interaction count.
const UNREACHABLE_PENALTY: usize = 1 << 20;

/// How often each unordered pair of logical qubits interacts.
///
/// Keyed on `(min, max)`, so `cx q0,q1` and `cx q1,q0` count toward the same
/// pair: layout cares about proximity, not direction. Direction is routing's
/// and orientation repair's problem.
fn interaction_counts(circuit: &Circuit) -> BTreeMap<(u32, u32), usize> {
    let mut counts = BTreeMap::new();
    for instruction in circuit.instructions() {
        let Instruction::Gate { qubits, .. } = instruction else {
            continue;
        };
        // Every pair of operands, so a three-qubit gate contributes three.
        for (i, a) in qubits.iter().enumerate() {
            for b in &qubits[i + 1..] {
                let key = (a.index().min(b.index()), a.index().max(b.index()));
                *counts.entry(key).or_insert(0) += 1;
            }
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::CircuitBuilder;

    const P0: PhysicalQubit = PhysicalQubit(0);
    const P1: PhysicalQubit = PhysicalQubit(1);
    const P2: PhysicalQubit = PhysicalQubit(2);

    fn line_circuit() -> Circuit {
        // q0 and q2 interact heavily; q1 barely at all.
        let mut b = CircuitBuilder::new("t");
        let q = b.alloc_qubits(3);
        b.cx(q[0], q[2]).cx(q[0], q[2]).cx(q[2], q[0]).h(q[1]);
        b.build().unwrap()
    }

    #[test]
    fn a_trivial_layout_is_the_identity() {
        let layout = Layout::trivial(3, 5).unwrap();
        assert_eq!(layout.physical(QubitId(2)), Some(P2));
        assert_eq!(layout.logical(P2), Some(QubitId(2)));
        assert_eq!(layout.permutation(), vec![0, 1, 2]);
        assert_eq!(layout.device_qubits(), 5);
        assert!(layout.is_injective());
    }

    #[test]
    fn unoccupied_device_qubits_are_ancillas() {
        let layout = Layout::trivial(2, 4).unwrap();
        assert_eq!(layout.len(), 2);
        assert_eq!(layout.logical(PhysicalQubit(3)), None);
        assert_eq!(layout.highest_occupied(), Some(P1));
    }

    #[test]
    fn two_logical_qubits_cannot_share_a_physical_one() {
        let err = Layout::from_pairs([(QubitId(0), P1), (QubitId(1), P1)], 3).unwrap_err();
        assert_eq!(
            err,
            LayoutError::NotInjective {
                first: QubitId(0),
                second: QubitId(1),
                physical: P1,
            }
        );
    }

    #[test]
    fn a_layout_off_the_device_is_rejected() {
        let err = Layout::from_pairs([(QubitId(0), PhysicalQubit(9))], 3).unwrap_err();
        assert!(matches!(err, LayoutError::PhysicalOutOfRange { .. }));
    }

    #[test]
    fn a_gap_in_the_logical_qubits_is_rejected() {
        // Logical 1 has no home, so the mapping is not usable for a circuit
        // declaring two qubits.
        let err = Layout::from_pairs([(QubitId(0), P0), (QubitId(2), P2)], 3).unwrap_err();
        assert_eq!(
            err,
            LayoutError::Unmapped {
                logical: QubitId(1)
            }
        );
    }

    #[test]
    fn a_circuit_larger_than_the_device_is_rejected() {
        let err = Layout::trivial(6, 4).unwrap_err();
        assert_eq!(
            err,
            LayoutError::DeviceTooSmall {
                needed: 6,
                available: 4
            }
        );
    }

    #[test]
    fn swapping_exchanges_two_tenants() {
        let mut layout = Layout::trivial(3, 3).unwrap();
        layout.swap_physical(P0, P2);
        assert_eq!(layout.physical(QubitId(0)), Some(P2));
        assert_eq!(layout.physical(QubitId(2)), Some(P0));
        assert_eq!(layout.logical(P0), Some(QubitId(2)));
        assert!(layout.is_injective());
    }

    #[test]
    fn swapping_with_an_ancilla_moves_the_tenant() {
        // The case a `to_physical`-only implementation gets wrong: physical 2
        // has no logical qubit, so there is nothing to swap *back*.
        let mut layout = Layout::trivial(2, 3).unwrap();
        layout.swap_physical(P1, P2);
        assert_eq!(layout.physical(QubitId(1)), Some(P2));
        assert_eq!(layout.logical(P1), None);
        assert_eq!(layout.logical(P2), Some(QubitId(1)));
        assert!(layout.is_injective());
    }

    #[test]
    fn swapping_a_qubit_with_itself_changes_nothing() {
        let mut layout = Layout::trivial(2, 2).unwrap();
        let before = layout.clone();
        layout.swap_physical(P1, P1);
        assert_eq!(layout, before);
    }

    #[test]
    fn swaps_compose_into_a_three_cycle() {
        // Two swaps sharing a qubit produce a non-involutive permutation —
        // the only kind that distinguishes a layout from its inverse.
        let mut layout = Layout::trivial(3, 3).unwrap();
        layout.swap_physical(P0, P1);
        layout.swap_physical(P1, P2);
        assert_eq!(layout.permutation(), vec![2, 0, 1]);

        let mut undone = layout.clone();
        undone.swap_physical(P1, P2);
        undone.swap_physical(P0, P1);
        assert_eq!(undone.permutation(), vec![0, 1, 2], "swaps are reversible");
    }

    #[test]
    fn the_trivial_strategy_reports_its_identity() {
        let topology = Topology::linear(5);
        let layout = TrivialLayout.plan(&line_circuit(), &topology).unwrap();
        assert_eq!(TrivialLayout.id(), "trivial");
        assert_eq!(layout.permutation(), vec![0, 1, 2]);
    }

    #[test]
    fn the_dense_strategy_seats_interacting_qubits_adjacently() {
        // On a line, the trivial layout puts q0 and q2 two hops apart even
        // though they do all the two-qubit work. Dense should not.
        let topology = Topology::linear(3);
        let layout = DenseLayout.plan(&line_circuit(), &topology).unwrap();

        let (a, b) = (
            layout.physical(QubitId(0)).unwrap(),
            layout.physical(QubitId(2)).unwrap(),
        );
        assert_eq!(
            topology.distance(a, b, CouplingMode::Undirected),
            Some(1),
            "the busiest pair should be adjacent, got {layout}"
        );
        assert!(layout.is_injective());
    }

    #[test]
    fn the_dense_strategy_is_deterministic() {
        let topology = Topology::linear(4);
        let circuit = line_circuit();
        assert_eq!(
            DenseLayout.plan(&circuit, &topology).unwrap(),
            DenseLayout.plan(&circuit, &topology).unwrap()
        );
    }

    #[test]
    fn a_disconnected_topology_still_yields_a_layout() {
        // Layout never fails on connectivity — routing is what refuses an
        // unroutable circuit, and it needs a layout to explain why.
        let topology = Topology::disconnected(4);
        let layout = DenseLayout.plan(&line_circuit(), &topology).unwrap();
        assert!(layout.is_injective());
    }

    #[test]
    fn interaction_counts_ignore_operand_order() {
        let counts = interaction_counts(&line_circuit());
        assert_eq!(
            counts.get(&(0, 2)),
            Some(&3),
            "cx 0,2 twice and cx 2,0 once"
        );
        assert_eq!(counts.get(&(0, 1)), None);
    }

    #[test]
    fn a_three_qubit_gate_contributes_every_pair() {
        let mut b = CircuitBuilder::new("t");
        let q = b.alloc_qubits(3);
        b.ccx(q[0], q[1], q[2]);
        let counts = interaction_counts(&b.build().unwrap());
        assert_eq!(counts.get(&(0, 1)), Some(&1));
        assert_eq!(counts.get(&(0, 2)), Some(&1));
        assert_eq!(counts.get(&(1, 2)), Some(&1));
    }

    #[test]
    fn a_layout_renders_readably() {
        assert_eq!(
            Layout::trivial(2, 2).unwrap().to_string(),
            "%q0->#q0, %q1->#q1"
        );
    }
}
