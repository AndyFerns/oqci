//! Physical qubit connectivity.
//!
//! Stage D §6 requires the target profile to expose topology so mapping and
//! routing can consume it, and §7 is blunt about the shape that takes:
//!
//! > If a backend treats a two-qubit interaction as directed, the target model
//! > must represent that explicitly. Do not assume that an undirected edge
//! > means both ordered interactions are equally native.
//!
//! So edges here are **directed**. An `(a, b)` edge means "a two-qubit
//! operation with `a` as control and `b` as target is native"; it says nothing
//! about `(b, a)`. A backend where both orderings are native declares both
//! edges — [`Topology::add_undirected`] and the built-in constructors do exactly
//! that, explicitly, rather than leaving it implied.
//!
//! That asymmetry is not hypothetical: on real superconducting hardware a
//! reversed CNOT costs surrounding basis changes. Encoding it as a missing
//! edge means routing finds out from the topology instead of from a comment.

use std::collections::BTreeSet;

use serde::Serialize;

/// A physical qubit on the target device, as distinct from a logical
/// [`crate::ir::QubitId`] in a circuit.
///
/// The two are separate types on purpose: conflating "qubit 3 in the program"
/// with "qubit 3 on the chip" is precisely the bug that layout and routing
/// exist to prevent, and Stage D §6 lists logical and physical qubit ids as
/// distinct required concepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct PhysicalQubit(pub u32);

impl PhysicalQubit {
    /// The raw index this reference wraps.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for PhysicalQubit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#q{}", self.0)
    }
}

/// A device's physical qubits and its directed coupling map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Topology {
    qubit_count: u32,
    /// Directed `(control, target)` pairs. Sorted and deduplicated, so two
    /// topologies built in different orders compare equal and serialize
    /// identically — profiles are snapshotted for reproducibility (Stage D §8).
    edges: BTreeSet<(u32, u32)>,
}

impl Topology {
    /// A topology with no couplings at all — every qubit isolated.
    #[must_use]
    pub fn disconnected(qubit_count: u32) -> Self {
        Topology {
            qubit_count,
            edges: BTreeSet::new(),
        }
    }

    /// Every ordered pair of distinct qubits is native.
    ///
    /// The "no connectivity constraints" case: a simulator, or any target
    /// where routing is a no-op.
    #[must_use]
    pub fn all_to_all(qubit_count: u32) -> Self {
        let mut topology = Topology::disconnected(qubit_count);
        for a in 0..qubit_count {
            for b in 0..qubit_count {
                if a != b {
                    topology.edges.insert((a, b));
                }
            }
        }
        topology
    }

    /// A line: `0 ↔ 1 ↔ 2 ↔ …`, each neighbour pair native in both directions.
    #[must_use]
    pub fn linear(qubit_count: u32) -> Self {
        let mut topology = Topology::disconnected(qubit_count);
        for a in 0..qubit_count.saturating_sub(1) {
            topology.add_undirected(PhysicalQubit(a), PhysicalQubit(a + 1));
        }
        topology
    }

    /// Adds a single directed coupling: `control → target` becomes native,
    /// and the reverse direction does **not**.
    pub fn add_directed(&mut self, control: PhysicalQubit, target: PhysicalQubit) -> &mut Self {
        self.edges.insert((control.index(), target.index()));
        self
    }

    /// Adds both directions of a coupling.
    ///
    /// This is a convenience for genuinely symmetric hardware, not a default:
    /// it inserts two explicit directed edges rather than introducing an
    /// "undirected" notion the rest of the model would have to interpret.
    pub fn add_undirected(&mut self, a: PhysicalQubit, b: PhysicalQubit) -> &mut Self {
        self.add_directed(a, b).add_directed(b, a)
    }

    /// Number of physical qubits.
    #[must_use]
    pub fn qubit_count(&self) -> u32 {
        self.qubit_count
    }

    /// `true` if `qubit` exists on this device.
    #[must_use]
    pub fn contains(&self, qubit: PhysicalQubit) -> bool {
        qubit.index() < self.qubit_count
    }

    /// Whether a two-qubit operation with this operand order is native.
    ///
    /// Order matters. `supports(a, b)` and `supports(b, a)` are independent
    /// questions unless the profile declared both directions.
    #[must_use]
    pub fn supports(&self, control: PhysicalQubit, target: PhysicalQubit) -> bool {
        self.edges.contains(&(control.index(), target.index()))
    }

    /// The qubits reachable from `qubit` as a control, ascending.
    #[must_use]
    pub fn neighbors(&self, qubit: PhysicalQubit) -> Vec<PhysicalQubit> {
        self.edges
            .iter()
            .filter(|(from, _)| *from == qubit.index())
            .map(|(_, to)| PhysicalQubit(*to))
            .collect()
    }

    /// Every directed coupling, ascending.
    #[must_use]
    pub fn edges(&self) -> Vec<(PhysicalQubit, PhysicalQubit)> {
        self.edges
            .iter()
            .map(|(a, b)| (PhysicalQubit(*a), PhysicalQubit(*b)))
            .collect()
    }

    /// Number of directed couplings. A symmetric line of `n` qubits has
    /// `2(n-1)` of them, not `n-1`.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// `true` if every declared coupling has its reverse declared too.
    ///
    /// Reported rather than assumed — see the module docs.
    #[must_use]
    pub fn is_symmetric(&self) -> bool {
        self.edges
            .iter()
            .all(|(a, b)| self.edges.contains(&(*b, *a)))
    }

    /// Couplings that reference a qubit outside `0..qubit_count`.
    ///
    /// Used by profile validation so a malformed coupling map is caught when
    /// the profile is built, not when a circuit is checked against it.
    pub(crate) fn out_of_range_edges(&self) -> Vec<(PhysicalQubit, PhysicalQubit)> {
        self.edges
            .iter()
            .filter(|(a, b)| *a >= self.qubit_count || *b >= self.qubit_count)
            .map(|(a, b)| (PhysicalQubit(*a), PhysicalQubit(*b)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const Q0: PhysicalQubit = PhysicalQubit(0);
    const Q1: PhysicalQubit = PhysicalQubit(1);
    const Q2: PhysicalQubit = PhysicalQubit(2);

    #[test]
    fn disconnected_has_no_couplings() {
        let topology = Topology::disconnected(3);
        assert_eq!(topology.qubit_count(), 3);
        assert_eq!(topology.edge_count(), 0);
        assert!(!topology.supports(Q0, Q1));
    }

    #[test]
    fn all_to_all_connects_every_ordered_pair() {
        let topology = Topology::all_to_all(3);
        assert_eq!(topology.edge_count(), 6, "3 * 2 ordered pairs");
        assert!(topology.supports(Q0, Q1));
        assert!(topology.supports(Q1, Q0));
        assert!(!topology.supports(Q0, Q0), "no self-coupling");
    }

    #[test]
    fn linear_connects_neighbours_in_both_directions() {
        let topology = Topology::linear(3);
        assert_eq!(topology.edge_count(), 4, "2 links, 2 directions each");
        assert!(topology.supports(Q0, Q1) && topology.supports(Q1, Q0));
        assert!(topology.supports(Q1, Q2) && topology.supports(Q2, Q1));
        assert!(!topology.supports(Q0, Q2), "not adjacent on a line");
    }

    #[test]
    fn a_directed_edge_does_not_imply_its_reverse() {
        // The Stage D §7 rule, as an executable assertion.
        let mut topology = Topology::disconnected(2);
        topology.add_directed(Q0, Q1);
        assert!(topology.supports(Q0, Q1));
        assert!(!topology.supports(Q1, Q0));
        assert!(!topology.is_symmetric());
    }

    #[test]
    fn undirected_declares_both_directions_explicitly() {
        let mut topology = Topology::disconnected(2);
        topology.add_undirected(Q0, Q1);
        assert_eq!(topology.edge_count(), 2);
        assert!(topology.is_symmetric());
    }

    #[test]
    fn neighbors_lists_outgoing_couplings_only() {
        let mut topology = Topology::disconnected(3);
        topology.add_directed(Q0, Q1).add_directed(Q2, Q0);
        assert_eq!(topology.neighbors(Q0), vec![Q1]);
        assert_eq!(topology.neighbors(Q1), vec![]);
    }

    #[test]
    fn membership_is_bounded_by_qubit_count() {
        let topology = Topology::linear(2);
        assert!(topology.contains(Q1));
        assert!(!topology.contains(Q2));
    }

    #[test]
    fn edges_are_deduplicated_and_order_independent() {
        let mut one = Topology::disconnected(2);
        one.add_directed(Q0, Q1).add_directed(Q0, Q1);
        let mut two = Topology::disconnected(2);
        two.add_directed(Q0, Q1);
        assert_eq!(one, two, "profiles must snapshot reproducibly");
    }

    #[test]
    fn out_of_range_edges_are_detectable() {
        let mut topology = Topology::disconnected(2);
        topology.add_directed(Q0, PhysicalQubit(7));
        assert_eq!(topology.out_of_range_edges(), vec![(Q0, PhysicalQubit(7))]);
    }
}
