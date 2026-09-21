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
//!
//! # Three graphs, one coupling map
//!
//! Because edges are directed, "are these two qubits connected?" is not one
//! question but three, and [`CouplingMode`] makes the caller say which:
//!
//! - [`CouplingMode::Directed`] — is this operation native *as written*?
//! - [`CouplingMode::Symmetric`] — can a `SWAP` run here unaided? A `SWAP` is
//!   three `CX`s in alternating directions, so it needs both orientations no
//!   matter which operand order it is written in.
//! - [`CouplingMode::Undirected`] — could an interaction happen here at all,
//!   given that a reversed `CX` is repairable by conjugation?
//!
//! The last one is a *conditional* graph: it is only the right routing graph
//! when that repair is genuinely expressible on the target. This module does
//! not decide that — `crate::lowering` derives it from the decomposition-rule
//! closure, because "the profile lists `h`" is the wrong test (a profile can
//! reach `h` through a rule without listing it).

use std::collections::{BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

/// A physical qubit on the target device, as distinct from a logical
/// [`crate::ir::QubitId`] in a circuit.
///
/// The two are separate types on purpose: conflating "qubit 3 in the program"
/// with "qubit 3 on the chip" is precisely the bug that layout and routing
/// exist to prevent, and Stage D §6 lists logical and physical qubit ids as
/// distinct required concepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
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

/// Which graph a connectivity query should run on.
///
/// Three genuinely different questions get asked of the same coupling map,
/// and conflating them is the entire bug class this enum exists to prevent.
/// Naming the graph at the call site is the point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CouplingMode {
    /// Exactly the declared edges. "Is this operation native *as written*?" —
    /// what [`crate::target::check`] asks.
    Directed,
    /// Only pairs where **both** orientations are declared. "Can a `SWAP`
    /// run here with no help?" — a `SWAP` is three `CX`s in alternating
    /// directions, so it needs both regardless of the operand order it is
    /// written in.
    Symmetric,
    /// Pairs where **either** orientation is declared. "Could an interaction
    /// happen here at all, given that a reversed `CX` is repairable by
    /// conjugation?" Only a valid routing graph when that repair is actually
    /// expressible on the target — see `crate::lowering`, which derives that
    /// from the decomposition-rule closure rather than assuming it.
    Undirected,
}

/// A device's physical qubits and its directed coupling map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    ///
    /// Equivalent to [`Topology::adjacent`] under [`CouplingMode::Directed`],
    /// and defined in terms of it so the two cannot drift apart.
    #[must_use]
    pub fn neighbors(&self, qubit: PhysicalQubit) -> Vec<PhysicalQubit> {
        self.adjacent(qubit, CouplingMode::Directed)
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

    /// Whether an interaction between `a` and `b` is available under `mode`.
    ///
    /// Note the asymmetry: under [`CouplingMode::Directed`] the operand order
    /// is the question, and under the other two modes it is not.
    #[must_use]
    pub fn couples(&self, a: PhysicalQubit, b: PhysicalQubit, mode: CouplingMode) -> bool {
        let (forward, reverse) = (self.supports(a, b), self.supports(b, a));
        match mode {
            CouplingMode::Directed => forward,
            CouplingMode::Symmetric => forward && reverse,
            CouplingMode::Undirected => forward || reverse,
        }
    }

    /// Qubits adjacent to `qubit` under `mode`, ascending.
    ///
    /// Ascending order is not cosmetic: it is what makes
    /// [`Topology::shortest_path`] a pure function of the topology rather than
    /// of iteration order.
    #[must_use]
    pub fn adjacent(&self, qubit: PhysicalQubit, mode: CouplingMode) -> Vec<PhysicalQubit> {
        (0..self.qubit_count)
            .map(PhysicalQubit)
            .filter(|other| *other != qubit && self.couples(qubit, *other, mode))
            .collect()
    }

    /// A shortest path from `from` to `to` under `mode`, inclusive of both
    /// endpoints. `None` when no path exists.
    ///
    /// Deterministic by construction: breadth-first with a FIFO frontier,
    /// expanding [`Topology::adjacent`] in ascending order, so the path found
    /// is the lexicographically smallest among the shortest ones. Routing
    /// reports a final layout derived from this, so "some shortest path" would
    /// not be a strong enough contract — the *same* one must come back every
    /// time (Stage D §8).
    ///
    /// A path from a qubit to itself is `[q]`, of length one.
    #[must_use]
    pub fn shortest_path(
        &self,
        from: PhysicalQubit,
        to: PhysicalQubit,
        mode: CouplingMode,
    ) -> Option<Vec<PhysicalQubit>> {
        self.shortest_path_avoiding(from, to, mode, &BTreeSet::new())
    }

    /// A shortest path that does not pass through any qubit in `avoid`.
    ///
    /// The endpoints are exempt, since a path has to start and finish
    /// somewhere, but nothing in between may be an avoided qubit.
    ///
    /// Routing needs this, and needs it to be a *search* rather than a filter
    /// applied afterwards. Taking the single best path and rejecting it when
    /// it happens to touch a forbidden qubit refuses circuits that are
    /// perfectly routable: on a ring, two paths between the same pair can be
    /// equally short, and discarding the first would miss the second.
    #[must_use]
    pub fn shortest_path_avoiding(
        &self,
        from: PhysicalQubit,
        to: PhysicalQubit,
        mode: CouplingMode,
        avoid: &BTreeSet<PhysicalQubit>,
    ) -> Option<Vec<PhysicalQubit>> {
        if !self.contains(from) || !self.contains(to) {
            return None;
        }
        if from == to {
            return Some(vec![from]);
        }

        // `predecessor[i]` is the node `i` was first reached from.
        let mut predecessor: Vec<Option<PhysicalQubit>> = vec![None; self.qubit_count as usize];
        let mut seen = BTreeSet::new();
        let mut frontier = VecDeque::new();
        seen.insert(from);
        frontier.push_back(from);

        while let Some(current) = frontier.pop_front() {
            for next in self.adjacent(current, mode) {
                // The destination is always allowed; intermediate hops are
                // not, when they are being avoided.
                if next != to && avoid.contains(&next) {
                    continue;
                }
                if !seen.insert(next) {
                    continue;
                }
                predecessor[next.index() as usize] = Some(current);
                if next == to {
                    let mut path = vec![to];
                    let mut step = to;
                    while let Some(previous) = predecessor[step.index() as usize] {
                        path.push(previous);
                        step = previous;
                    }
                    path.reverse();
                    return Some(path);
                }
                frontier.push_back(next);
            }
        }
        None
    }

    /// Hop count between two qubits under `mode` — one less than the length
    /// of [`Topology::shortest_path`]. `None` when unreachable.
    #[must_use]
    pub fn distance(
        &self,
        from: PhysicalQubit,
        to: PhysicalQubit,
        mode: CouplingMode,
    ) -> Option<usize> {
        self.shortest_path(from, to, mode)
            .map(|path| path.len() - 1)
    }

    /// Whether every qubit can reach every other under `mode`.
    ///
    /// An empty topology is connected; a single qubit is connected. Routing
    /// checks this up front, because a circuit whose interacting qubits land
    /// in different components can never be repaired by any number of
    /// `SWAP`s, and discovering that per-gate would mean failing deep inside
    /// a search instead of at the entrance.
    #[must_use]
    pub fn is_connected(&self, mode: CouplingMode) -> bool {
        if self.qubit_count <= 1 {
            return true;
        }
        let root = PhysicalQubit(0);
        (1..self.qubit_count).all(|other| {
            self.shortest_path(root, PhysicalQubit(other), mode)
                .is_some()
        })
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
    fn coupling_modes_ask_three_different_questions() {
        let mut topology = Topology::disconnected(2);
        topology.add_directed(Q0, Q1);

        assert!(topology.couples(Q0, Q1, CouplingMode::Directed));
        assert!(!topology.couples(Q1, Q0, CouplingMode::Directed));
        // Declared one way only: not symmetric, but an interaction is
        // conditionally available.
        assert!(!topology.couples(Q0, Q1, CouplingMode::Symmetric));
        assert!(topology.couples(Q1, Q0, CouplingMode::Undirected));
    }

    #[test]
    fn a_directed_only_edge_is_unusable_for_an_unaided_swap() {
        // The distinction that matters to routing: `Undirected` finds a path
        // across a one-way edge, `Symmetric` does not.
        let mut topology = Topology::disconnected(3);
        topology.add_undirected(Q0, Q1);
        topology.add_directed(Q1, Q2);

        assert_eq!(topology.distance(Q0, Q2, CouplingMode::Undirected), Some(2));
        assert_eq!(topology.distance(Q0, Q2, CouplingMode::Symmetric), None);
    }

    #[test]
    fn shortest_path_includes_both_endpoints() {
        let topology = Topology::linear(4);
        assert_eq!(
            topology.shortest_path(Q0, PhysicalQubit(3), CouplingMode::Symmetric),
            Some(vec![Q0, Q1, Q2, PhysicalQubit(3)])
        );
        assert_eq!(
            topology.distance(Q0, PhysicalQubit(3), CouplingMode::Symmetric),
            Some(3)
        );
    }

    #[test]
    fn a_path_to_itself_is_a_single_hopless_step() {
        let topology = Topology::linear(3);
        assert_eq!(
            topology.shortest_path(Q1, Q1, CouplingMode::Symmetric),
            Some(vec![Q1])
        );
        assert_eq!(topology.distance(Q1, Q1, CouplingMode::Symmetric), Some(0));
    }

    #[test]
    fn shortest_path_picks_the_lexicographically_smallest_of_the_shortest() {
        // A diamond: 0 reaches 3 through either 1 or 2, both in two hops.
        // Routing derives a reported final layout from this choice, so "some
        // shortest path" is not a strong enough contract — it must be the
        // same one every time.
        let mut topology = Topology::disconnected(4);
        let q3 = PhysicalQubit(3);
        topology.add_undirected(Q0, Q2);
        topology.add_undirected(Q0, Q1);
        topology.add_undirected(Q2, q3);
        topology.add_undirected(Q1, q3);

        let path = topology.shortest_path(Q0, q3, CouplingMode::Symmetric);
        assert_eq!(path, Some(vec![Q0, Q1, q3]), "via the lower-indexed middle");

        // Insertion order must not change the answer.
        let mut reordered = Topology::disconnected(4);
        reordered.add_undirected(Q1, q3);
        reordered.add_undirected(Q0, Q1);
        reordered.add_undirected(Q2, q3);
        reordered.add_undirected(Q0, Q2);
        assert_eq!(
            reordered.shortest_path(Q0, q3, CouplingMode::Symmetric),
            path
        );
    }

    #[test]
    fn an_unreachable_qubit_has_no_path_and_no_distance() {
        let mut topology = Topology::disconnected(4);
        topology.add_undirected(Q0, Q1);
        topology.add_undirected(Q2, PhysicalQubit(3));

        assert_eq!(
            topology.shortest_path(Q0, Q2, CouplingMode::Undirected),
            None
        );
        assert_eq!(topology.distance(Q0, Q2, CouplingMode::Undirected), None);
        assert!(!topology.is_connected(CouplingMode::Undirected));
    }

    #[test]
    fn avoidance_routes_around_rather_than_refusing() {
        // A ring: 0-1-2-3-0. Both ways from 0 to 2 are two hops. Avoiding one
        // middle qubit must find the other route rather than give up, which
        // is what filtering a single best path after the fact would do.
        let q3 = PhysicalQubit(3);
        let mut ring = Topology::disconnected(4);
        ring.add_undirected(Q0, Q1);
        ring.add_undirected(Q1, Q2);
        ring.add_undirected(Q2, q3);
        ring.add_undirected(q3, Q0);

        assert_eq!(
            ring.shortest_path(Q0, Q2, CouplingMode::Symmetric),
            Some(vec![Q0, Q1, Q2])
        );
        assert_eq!(
            ring.shortest_path_avoiding(Q0, Q2, CouplingMode::Symmetric, &BTreeSet::from([Q1])),
            Some(vec![Q0, q3, Q2]),
            "the other way round the ring is just as short"
        );
    }

    #[test]
    fn avoidance_can_make_a_pair_genuinely_unreachable() {
        // On a line there is no detour, so avoiding the middle really does
        // separate the ends.
        let line = Topology::linear(3);
        assert!(
            line.shortest_path_avoiding(Q0, Q2, CouplingMode::Symmetric, &BTreeSet::from([Q1]))
                .is_none()
        );
    }

    #[test]
    fn avoiding_an_endpoint_does_not_prevent_reaching_it() {
        // A path has to finish somewhere. Only intermediate hops are barred.
        let line = Topology::linear(3);
        assert_eq!(
            line.shortest_path_avoiding(Q0, Q1, CouplingMode::Symmetric, &BTreeSet::from([Q1])),
            Some(vec![Q0, Q1])
        );
    }

    #[test]
    fn a_path_off_the_device_is_none_rather_than_a_panic() {
        let topology = Topology::linear(2);
        assert_eq!(
            topology.shortest_path(Q0, PhysicalQubit(9), CouplingMode::Symmetric),
            None
        );
    }

    #[test]
    fn connectivity_is_mode_sensitive() {
        let mut topology = Topology::disconnected(2);
        topology.add_directed(Q0, Q1);
        assert!(topology.is_connected(CouplingMode::Undirected));
        assert!(!topology.is_connected(CouplingMode::Symmetric));

        // Degenerate cases are connected, not a special-cased panic.
        assert!(Topology::disconnected(0).is_connected(CouplingMode::Symmetric));
        assert!(Topology::disconnected(1).is_connected(CouplingMode::Symmetric));
    }

    #[test]
    fn adjacent_and_neighbors_agree_on_the_directed_graph() {
        let mut topology = Topology::disconnected(3);
        topology.add_directed(Q0, Q1).add_directed(Q2, Q0);
        assert_eq!(
            topology.neighbors(Q0),
            topology.adjacent(Q0, CouplingMode::Directed)
        );
        assert_eq!(
            topology.adjacent(Q0, CouplingMode::Undirected),
            vec![Q1, Q2]
        );
        assert_eq!(topology.adjacent(Q0, CouplingMode::Symmetric), vec![]);
    }

    #[test]
    fn out_of_range_edges_are_detectable() {
        let mut topology = Topology::disconnected(2);
        topology.add_directed(Q0, PhysicalQubit(7));
        assert_eq!(topology.out_of_range_edges(), vec![(Q0, PhysicalQubit(7))]);
    }
}
