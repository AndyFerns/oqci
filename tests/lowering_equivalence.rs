//! The central correctness property for target lowering.
//!
//! For a generated circuit and a generated device, lowering must **either**
//! refuse with a typed error whose cause this test independently confirms,
//! **or** produce a circuit that is simultaneously
//!
//! 1. semantically equivalent to the original, modulo the final layout;
//! 2. reported fully legal by [`oqci::target::check`]; and
//! 3. byte-identical across repeated runs.
//!
//! # Why comparing full unitaries would be wrong
//!
//! The obvious check — build the whole unitary of the routed circuit and
//! compare it to the original's — **fails on correct routing**, so it is worth
//! writing down why. Take `C = [CX(q0,q1)]` on a three-qubit line with the
//! initial layout `{q0 -> p0, q1 -> p2}`. A correct router emits
//! `SWAP(p0,p1); CX(p1,p2)` and reports the final layout `{q0 -> p1,
//! q1 -> p2}`. The routed circuit and the expected `CX(p1,p2)` are genuinely
//! *different* three-qubit unitaries: they differ on inputs where `p1` is not
//! `|0>`. They agree exactly where it matters — on the subspace the program
//! actually uses, with ancillas in the ground state.
//!
//! # The isometry comparison
//!
//! So the claim is about a map from the logical input space, not about a full
//! operator. For each of the `2^n` logical basis states `j`:
//!
//! ```text
//! col_R[j] = simulate(prepare j through the INITIAL layout ++ routed)
//! col_E[j] = simulate(prepare j through the FINAL layout   ++ original relabelled by the FINAL layout)
//! assert |sum_j <col_R[j]|col_E[j]>| ~ 2^n
//! ```
//!
//! Three things fall out of that shape:
//!
//! - **Permutation** is handled by preparing the input through the *initial*
//!   layout and the expectation through the *final* one. This is exactly where
//!   a layout-inversion bug gets caught, so it is the line to read twice.
//! - **Padding** is free: both sides live on the routed circuit's register and
//!   start from `|0...0>`, so ancillas contribute no amplitude to either.
//! - **Global phase** is quotiented out by the modulus, as before.
//!
//! The overlaps are **summed before** the modulus is taken. Comparing each
//! column up to its own phase is a strictly weaker and false claim: it accepts
//! a rewrite that negates one basis state and leaves the rest, which changes
//! every superposition. See `tests/decomposition.rs` for that argument in
//! full, including the test that demonstrates it.
//!
//! # Why the generators are themselves tested
//!
//! A property test that never generates a hard case passes for the wrong
//! reason. On a four-qubit device a random pair of operands is usually
//! *already* adjacent, so a naive generator would exercise no routing at all.
//! [`generators_reach_the_hard_cases`] samples the strategy and asserts hard
//! floors on how often SWAPs, orientation repairs, non-involutive layouts and
//! wide gates actually occur.
//!
//! # Evidence that this suite can fail
//!
//! A harness that has never rejected anything is not evidence, so it has been
//! made to. Dropping one of the two `H` gates from the `Cx` orientation
//! repair in `src/lowering/routing.rs` — a plausible typo, and one that leaves
//! the circuit perfectly *legal*, so `check` reports nothing wrong — makes
//! [`lowering_is_equivalent_legal_and_deterministic`] fail with an overlap of
//! `1.3e-15` where `8` was required. Not a near miss: the circuits are
//! orthogonal. Restoring the gate returns the suite to green.
//!
//! That is the failure mode this file exists for. Legality and semantics are
//! independent properties, and only one of them has a checker inside the
//! compiler.

mod support;

use std::collections::BTreeSet;

use num_complex::Complex64;
use proptest::prelude::*;
use proptest::strategy::ValueTree;

use oqci::ir::{Circuit, CircuitBuilder, GateKind, Instruction, Param, QubitId};
use oqci::lowering::{
    Layout, LayoutChoice, Lowered, LoweringConfig, LoweringError, UnroutableReason, lower,
};
use oqci::target::{
    BasisProfile, BasisProfileBuilder, CouplingMode, PhysicalQubit, Topology, check,
};
use support::statevector::simulate_from_basis_state;

/// Hard cap for the state-vector oracle. Five qubits is 32 amplitudes per
/// column and 32 columns — cheap — while eight would already be uncomfortable
/// inside a proptest loop.
const MAX_QUBITS: u32 = 5;

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// A coupling between two physical qubits, and which orientations exist.
#[derive(Debug, Clone, Copy)]
enum Edge {
    Both(u32, u32),
    Forward(u32, u32),
    Reverse(u32, u32),
}

/// A connected topology: a random spanning tree, plus extra edges.
///
/// Built as a tree first so connectivity is guaranteed rather than hoped for,
/// and biased toward paths, which are what force long routes. Each edge
/// independently picks its orientations, with real weight on the one-way
/// cases — without those, orientation repair never fires and a whole phase of
/// the compiler goes untested.
fn topology_strategy() -> impl Strategy<Value = (u32, Topology)> {
    (3u32..=MAX_QUBITS).prop_flat_map(|n| {
        // `parent[i]` attaches qubit `i+1` to an earlier qubit. Choosing the
        // immediate predecessor often yields a path.
        let parents = prop::collection::vec(0usize..4, (n - 1) as usize);
        let orientations = prop::collection::vec(0usize..3, (n - 1) as usize);
        (Just(n), parents, orientations).prop_map(|(n, parents, orientations)| {
            let mut topology = Topology::disconnected(n);
            for (i, (back, orientation)) in parents.iter().zip(&orientations).enumerate() {
                let child = (i + 1) as u32;
                let parent = child.saturating_sub(1 + *back as u32);
                let edge = match orientation {
                    0 => Edge::Both(parent, child),
                    1 => Edge::Forward(parent, child),
                    _ => Edge::Reverse(parent, child),
                };
                match edge {
                    Edge::Both(a, b) => {
                        topology.add_undirected(PhysicalQubit(a), PhysicalQubit(b));
                    }
                    Edge::Forward(a, b) => {
                        topology.add_directed(PhysicalQubit(a), PhysicalQubit(b));
                    }
                    Edge::Reverse(a, b) => {
                        topology.add_directed(PhysicalQubit(b), PhysicalQubit(a));
                    }
                }
            }
            (n, topology)
        })
    })
}

/// A restricted-basis profile over a generated topology.
///
/// Deliberately `{rz, sx, x, cx}` rather than all-to-all with every gate: a
/// permissive target exercises neither decomposition nor orientation repair.
fn profile_for(topology: Topology) -> BasisProfile {
    BasisProfileBuilder::new("generated", "1", "test", topology)
        .operations(["rz", "sx", "x", "cx"])
        .decomposition_rules([
            "id-to-nothing",
            "y-to-rz-x",
            "z-to-rz",
            "h-to-rz-sx",
            "s-to-rz",
            "sdg-to-rz",
            "t-to-rz",
            "tdg-to-rz",
            "sxdg-to-sx",
            "rx-to-rz-sx",
            "ry-to-rz-sx",
            "p-to-rz",
            "u-to-rz-sx",
            "cy-to-cx",
            "cz-to-cx",
            "swap-to-cx",
            "ccx-to-cx",
        ])
        .cost_model("uniform")
        .build()
        .expect("generated profile is well-formed")
}

const ANGLES: &[f64] = &[
    0.0,
    0.61,
    -1.27,
    std::f64::consts::FRAC_PI_2,
    std::f64::consts::PI,
];

/// A unitary circuit over `n` qubits, biased toward the cases that hurt.
///
/// Two-qubit operands are drawn to be *distant* where possible, and a
/// superposition layer is prepended so no error can hide behind a product
/// state. Measurement, reset and symbolic parameters are excluded: the
/// state-vector oracle cannot model any of them, and they are covered by
/// their own tests.
fn circuit_strategy(n: u32, topology: Topology) -> impl Strategy<Value = Circuit> {
    let far_pairs = distant_pairs(&topology, n);
    prop::collection::vec(0usize..12, 1..7).prop_map(move |choices| {
        let mut b = CircuitBuilder::new("generated");
        let q = b.alloc_qubits(n);
        // Full support, so a permutation error cannot be invisible.
        for qubit in &q {
            b.h(*qubit);
        }
        for (step, choice) in choices.iter().enumerate() {
            let angle = Param::concrete(ANGLES[step % ANGLES.len()]);
            let one = q[step % q.len()];
            let (a, b_qubit) = far_pairs[step % far_pairs.len()];
            match choice {
                0 => b.gate(GateKind::H, [one]),
                1 => b.gate(GateKind::T, [one]),
                2 => b.gate(GateKind::Rx(angle), [one]),
                3 => b.gate(GateKind::Ry(angle), [one]),
                4 => b.gate(GateKind::P(angle), [one]),
                5 => b.gate(GateKind::SXdg, [one]),
                6 => b.gate(GateKind::Cx, [a, b_qubit]),
                7 => b.gate(GateKind::Cz, [a, b_qubit]),
                8 => b.gate(GateKind::Cy, [a, b_qubit]),
                9 => b.gate(GateKind::Swap, [a, b_qubit]),
                10 if n >= 3 => {
                    let c = q[(step + 2) % q.len()];
                    if a != c && b_qubit != c {
                        b.gate(GateKind::Ccx, [a, b_qubit, c])
                    } else {
                        b.gate(GateKind::Cx, [a, b_qubit])
                    }
                }
                _ => b.gate(GateKind::Y, [one]),
            };
        }
        b.build().expect("generated circuit is valid")
    })
}

/// A device and a circuit generated *for* it.
///
/// Composed with `prop_flat_map` rather than drawing the circuit from a fresh
/// runner inside the test body: proptest can only shrink what it generated,
/// and a circuit conjured mid-test would be re-conjured differently on every
/// shrink attempt, making a failure impossible to minimize.
fn case_strategy() -> impl Strategy<Value = (Circuit, BasisProfile)> {
    topology_strategy().prop_flat_map(|(n, topology)| {
        let profile = profile_for(topology.clone());
        circuit_strategy(n, topology).prop_map(move |circuit| (circuit, profile.clone()))
    })
}

/// Operand pairs at graph distance two or more, falling back to any pair.
///
/// This is the difference between a property test that routes and one that
/// silently never does.
fn distant_pairs(topology: &Topology, n: u32) -> Vec<(QubitId, QubitId)> {
    let mut far = Vec::new();
    let mut near = Vec::new();
    for a in 0..n {
        for b in 0..n {
            if a == b {
                continue;
            }
            let distance =
                topology.distance(PhysicalQubit(a), PhysicalQubit(b), CouplingMode::Undirected);
            match distance {
                Some(d) if d >= 2 => far.push((QubitId(a), QubitId(b))),
                _ => near.push((QubitId(a), QubitId(b))),
            }
        }
    }
    if far.is_empty() { near } else { far }
}

// ---------------------------------------------------------------------------
// The equivalence comparison
// ---------------------------------------------------------------------------

/// Whether a lowered circuit computes what the original did, on the logical
/// subspace, up to one global phase.
fn equivalent_modulo_layout(original: &Circuit, lowered: &Lowered) -> Result<(), String> {
    let n = original.num_qubits();
    let width = lowered.circuit.num_qubits();
    if width > MAX_QUBITS {
        return Err(format!(
            "lowered circuit is {width} qubits, over the oracle's limit"
        ));
    }

    // The expectation: the original circuit, relabelled onto the physical
    // wires its logical qubits *end* on.
    let expected = relabel(original, &lowered.final_layout, width)?;

    let mut trace = Complex64::new(0.0, 0.0);
    for basis in 0..1usize << n {
        let set: Vec<u32> = (0..n).filter(|q| basis & (1 << q) != 0).collect();

        // Input prepared where the logical qubits *start*...
        let start: Vec<u32> = set
            .iter()
            .map(|q| physical(&lowered.initial_layout, *q))
            .collect::<Result<_, _>>()?;
        let routed = simulate_from_basis_state(&lowered.circuit, &start, MAX_QUBITS);

        // ...and the expectation prepared where they *end*.
        let finish: Vec<u32> = set
            .iter()
            .map(|q| physical(&lowered.final_layout, *q))
            .collect::<Result<_, _>>()?;
        let reference = simulate_from_basis_state(&expected, &finish, MAX_QUBITS);

        for (a, b) in routed.iter().zip(&reference) {
            trace += a.conj() * b;
        }
    }

    let dimension = (1usize << n) as f64;
    let deviation = (trace.norm() - dimension).abs();
    if deviation > 1e-9 {
        return Err(format!(
            "|tr| is {} but should be {dimension}: off by {deviation:e}",
            trace.norm()
        ));
    }
    Ok(())
}

fn physical(layout: &Layout, logical: u32) -> Result<u32, String> {
    layout
        .physical(QubitId(logical))
        .map(|p| p.index())
        .ok_or_else(|| format!("logical qubit {logical} is not in the layout"))
}

/// The original circuit, rewritten onto physical wires via a layout.
fn relabel(circuit: &Circuit, layout: &Layout, width: u32) -> Result<Circuit, String> {
    let mut b = CircuitBuilder::new("expected");
    b.alloc_qubits(width);
    b.alloc_clbits(circuit.num_clbits());
    for instruction in circuit.instructions() {
        let Instruction::Gate { kind, qubits } = instruction else {
            return Err("the oracle only handles unitary circuits".into());
        };
        let mapped: Result<Vec<QubitId>, String> = qubits
            .iter()
            .map(|q| physical(layout, q.index()).map(QubitId))
            .collect();
        b.gate(kind.clone(), mapped?);
    }
    b.build().map_err(|e| e.to_string())
}

/// Recomputes the final layout independently, by replaying the `Swap`s the
/// lowered circuit actually contains.
///
/// Closes the circularity in the comparison above, which otherwise trusts
/// lowering's own report of where qubits ended up: a bug that corrupted the
/// circuit and the layout in the same direction would pass unnoticed. This is
/// what catches "updated the layout but emitted the swap on the wrong pair".
///
/// Two conditions on the caller, both real:
///
/// - Lower with `decompose: false`, or decomposition will have rewritten the
///   `Swap`s into CNOTs and there is nothing left to replay.
/// - Only use it on circuits that contain **no `Swap` of their own**. A
///   program-level swap exchanges the two qubits' *states*; a routing swap
///   exchanges the *mapping*. They are indistinguishable in the output, so a
///   replay over a circuit containing both would attribute the program's
///   swaps to routing and disagree for entirely correct reasons. Keeping the
///   distinction outside the compiler is what preserves this check's
///   independence — having routing tag its own insertions would make the test
///   trust the very thing it is auditing.
fn replay_final_layout(initial: &Layout, routed: &Circuit) -> Layout {
    let mut layout = initial.clone();
    for instruction in routed.instructions() {
        if let Instruction::Gate {
            kind: GateKind::Swap,
            qubits,
        } = instruction
        {
            layout.swap_physical(
                PhysicalQubit(qubits[0].index()),
                PhysicalQubit(qubits[1].index()),
            );
        }
    }
    layout
}

// ---------------------------------------------------------------------------
// Independently-confirmed refusals
// ---------------------------------------------------------------------------

/// Confirms a refusal was forced, using a witness computed here rather than
/// taken from the compiler.
///
/// Without this, an implementation that refused almost everything would sail
/// through the whole suite.
fn refusal_is_justified(
    error: &LoweringError,
    circuit: &Circuit,
    profile: &BasisProfile,
) -> Result<(), String> {
    match error {
        LoweringError::Unroutable {
            reason: UnroutableReason::NoPath,
            from,
            to,
            ..
        } => {
            // The test's own reachability check, not the compiler's.
            let symmetric = profile
                .topology()
                .distance(*from, *to, CouplingMode::Symmetric);
            let undirected = profile
                .topology()
                .distance(*from, *to, CouplingMode::Undirected);
            if symmetric.is_none() && undirected.is_none() {
                Ok(())
            } else {
                Err(format!(
                    "claimed {from} and {to} unreachable, but distances are \
                     symmetric={symmetric:?} undirected={undirected:?}"
                ))
            }
        }
        LoweringError::Unroutable {
            reason: UnroutableReason::MeasurementFrozenWire,
            ..
        } => {
            if circuit
                .instructions()
                .iter()
                .any(|i| matches!(i, Instruction::Measure { .. }))
            {
                Ok(())
            } else {
                Err("blamed a measured wire in a circuit with no measurement".into())
            }
        }
        LoweringError::UnrepairableOrientation { mnemonic, .. } => {
            // Only operations with no verified reversal may be refused this
            // way. `cx`, `cz` and `swap` all have one.
            if matches!(mnemonic.as_str(), "cx" | "cz" | "swap") {
                Err(format!(
                    "`{mnemonic}` has a known reversal and should not be refused"
                ))
            } else {
                Ok(())
            }
        }
        LoweringError::Layout { .. } => Ok(()),
        other => Err(format!("unexpected refusal: {other}")),
    }
}

// ---------------------------------------------------------------------------
// The properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 192, ..ProptestConfig::default() })]

    /// The headline property: lowering is correct, legal and deterministic,
    /// or it refuses for a reason this test can independently confirm.
    #[test]
    fn lowering_is_equivalent_legal_and_deterministic(
        (circuit, profile) in case_strategy(),
        choice in 0usize..2,
    ) {
        let config = LoweringConfig {
            layout: if choice == 0 { LayoutChoice::Trivial } else { LayoutChoice::Dense },
            ..LoweringConfig::default()
        };

        match lower(&circuit, &profile, &config) {
            Ok(lowered) => {
                // (b) legal, by the target model's own judgement.
                let report = check(&lowered.circuit, &profile);
                prop_assert!(
                    report.is_legal(),
                    "lowering returned Ok but check reports {:?}",
                    report.violations
                );
                // ...and the things `check` structurally cannot see.
                for instruction in lowered.circuit.instructions() {
                    prop_assert!(
                        instruction.qubits().len() <= 2,
                        "a wide gate survived lowering: {instruction:?}"
                    );
                }
                prop_assert!(lowered.initial_layout.is_injective());
                prop_assert!(lowered.final_layout.is_injective());

                // (a) equivalent, modulo the final layout.
                if let Err(why) = equivalent_modulo_layout(&circuit, &lowered) {
                    prop_assert!(false, "lowering changed what the circuit computes: {why}");
                }

                // (c) deterministic.
                let again = lower(&circuit, &profile, &config).expect("second run");
                prop_assert_eq!(&lowered.circuit, &again.circuit);
                prop_assert_eq!(&lowered.final_layout, &again.final_layout);
                prop_assert_eq!(lowered.swaps_inserted, again.swaps_inserted);
            }
            Err(error) => {
                if let Err(why) = refusal_is_justified(&error, &circuit, &profile) {
                    prop_assert!(false, "unjustified refusal: {why}");
                }
            }
        }
    }

    /// The reported final layout matches one recomputed from the emitted
    /// SWAPs. Breaks the circularity in the equivalence check.
    #[test]
    fn the_reported_final_layout_matches_the_emitted_swaps(
        (circuit, profile) in case_strategy(),
    ) {
        // See `replay_final_layout`: a program's own swaps are
        // indistinguishable from routing's in the output.
        prop_assume!(!contains_swap(&circuit));

        // Routing only: after decomposition the `Swap`s are gone.
        let config = LoweringConfig { decompose: false, ..LoweringConfig::default() };
        if let Ok(lowered) = lower(&circuit, &profile, &config) {
            let replayed = replay_final_layout(&lowered.initial_layout, &lowered.circuit);
            prop_assert_eq!(
                replayed.permutation(),
                lowered.final_layout.permutation(),
                "the reported layout disagrees with the swaps actually emitted"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The generators are themselves tested
// ---------------------------------------------------------------------------

/// Coverage floors. Without these, every property above can be green while
/// exercising nothing interesting — which is the failure mode of a
/// property-based suite, and the one that is invisible unless measured.
#[test]
fn generators_reach_the_hard_cases() {
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let (mut routed, mut repaired, mut non_involutive, mut wide, mut refused) = (0, 0, 0, 0, 0);
    let mut refusal_kinds: BTreeSet<&'static str> = BTreeSet::new();
    let samples = 200;

    for _ in 0..samples {
        let (circuit, profile) = case_strategy().new_tree(&mut runner).unwrap().current();

        if circuit.instructions().iter().any(|i| i.qubits().len() > 2) {
            wide += 1;
        }

        match lower(&circuit, &profile, &LoweringConfig::default()) {
            Ok(lowered) => {
                if lowered.swaps_inserted > 0 {
                    routed += 1;
                }
                if lowered.orientations_repaired > 0 {
                    repaired += 1;
                }
                // A permutation with a cycle of length three or more. Only
                // these distinguish a layout from its inverse: a single swap
                // is its own inverse, as is any product of disjoint
                // transpositions, and those dominate small linear cases.
                if has_long_cycle(&lowered.final_layout.permutation()) {
                    non_involutive += 1;
                }
            }
            Err(error) => {
                refused += 1;
                refusal_kinds.insert(match error {
                    LoweringError::Unroutable {
                        reason: UnroutableReason::NoPath,
                        ..
                    } => "unroutable-no-path",
                    LoweringError::Unroutable { .. } => "unroutable-frozen",
                    LoweringError::UnrepairableOrientation { .. } => "orientation",
                    LoweringError::Layout { .. } => "layout",
                    _ => "other",
                });
            }
        }
    }

    assert!(
        routed * 10 >= samples,
        "only {routed}/{samples} cases inserted a SWAP"
    );
    assert!(
        repaired * 20 >= samples,
        "only {repaired}/{samples} cases repaired an orientation"
    );
    assert!(
        non_involutive * 50 >= samples,
        "only {non_involutive}/{samples} cases produced a layout with a 3-cycle, \
         so a layout-inversion bug would be invisible"
    );
    assert!(
        wide * 20 >= samples,
        "only {wide}/{samples} cases contained a wide gate"
    );
    // Deliberately *not* asserting a refusal floor here. `topology_strategy`
    // builds from a spanning tree, so every device it produces is connected
    // and every circuit is lowerable — and that all 200 cases succeeded is
    // itself the result worth having. It is the completeness half of the
    // property: lowering does not refuse work it should accept.
    assert_eq!(
        refused, 0,
        "a connected device with a closed rule set should never force a refusal,          but {refused}/{samples} cases were refused: {refusal_kinds:?}"
    );
}

/// The refusal paths, on devices that genuinely cannot run the circuit.
///
/// Separate from the coverage test above because it needs a generator that
/// produces *disconnected* topologies — which the main strategy deliberately
/// does not, since its job is to exercise routing rather than to defeat it.
#[test]
fn refusals_happen_only_when_they_are_forced() {
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let mut refusals: BTreeSet<&'static str> = BTreeSet::new();
    let mut refused = 0;
    let samples = 120;

    for _ in 0..samples {
        // Two disjoint halves: nothing can bridge them.
        let (n, _) = topology_strategy().new_tree(&mut runner).unwrap().current();
        let mut split = Topology::disconnected(n);
        for a in 1..n {
            if a != n / 2 {
                split.add_undirected(PhysicalQubit(a - 1), PhysicalQubit(a));
            }
        }
        let profile = profile_for(split.clone());
        let circuit = circuit_strategy(n, split.clone())
            .new_tree(&mut runner)
            .unwrap()
            .current();

        match lower(&circuit, &profile, &LoweringConfig::default()) {
            Ok(lowered) => {
                // Succeeding on a disconnected device is fine — but only if
                // every two-qubit operation really did stay inside one
                // component. Verified against the test's own reachability.
                for instruction in lowered.circuit.instructions() {
                    let qubits = instruction.qubits();
                    if qubits.len() == 2 {
                        assert!(
                            split
                                .distance(
                                    PhysicalQubit(qubits[0].index()),
                                    PhysicalQubit(qubits[1].index()),
                                    CouplingMode::Undirected
                                )
                                .is_some(),
                            "lowering emitted an operation across disconnected components"
                        );
                    }
                }
            }
            Err(error) => {
                refused += 1;
                assert!(
                    refusal_is_justified(&error, &circuit, &profile).is_ok(),
                    "unjustified refusal: {error}"
                );
                refusals.insert(match error {
                    LoweringError::Unroutable {
                        reason: UnroutableReason::NoPath,
                        ..
                    } => "unroutable-no-path",
                    LoweringError::Unroutable { .. } => "unroutable-frozen",
                    LoweringError::UnrepairableOrientation { .. } => "orientation",
                    _ => "other",
                });
            }
        }
    }

    assert!(
        refused * 4 >= samples,
        "only {refused}/{samples} disconnected cases were refused,          so the refusal paths are barely exercised"
    );
}

/// Whether a circuit contains a `Swap` the programmer wrote.
fn contains_swap(circuit: &Circuit) -> bool {
    circuit.instructions().iter().any(|i| {
        matches!(
            i,
            Instruction::Gate {
                kind: GateKind::Swap,
                ..
            }
        )
    })
}

/// Whether a permutation has a cycle of length three or more.
fn has_long_cycle(permutation: &[u32]) -> bool {
    let mut seen = vec![false; permutation.len()];
    for start in 0..permutation.len() {
        if seen[start] {
            continue;
        }
        let mut length = 0;
        let mut current = start;
        while !seen[current] {
            seen[current] = true;
            let next = permutation[current] as usize;
            if next >= permutation.len() {
                break;
            }
            current = next;
            length += 1;
        }
        if length >= 3 {
            return true;
        }
    }
    false
}

/// The comparison must be able to fail. A harness that has never rejected
/// anything is not evidence.
#[test]
fn the_equivalence_check_rejects_a_wrong_layout() {
    let profile = oqci::target::builtin::linear_nisq(3);
    let mut b = CircuitBuilder::new("bell");
    let q = b.alloc_qubits(3);
    b.h(q[0]).cx(q[0], q[2]);
    let circuit = b.build().unwrap();

    let lowered = lower(&circuit, &profile, &LoweringConfig::default()).unwrap();
    assert!(lowered.swaps_inserted > 0, "this circuit must need routing");
    assert!(equivalent_modulo_layout(&circuit, &lowered).is_ok());

    // Now claim the qubits never moved. The routed circuit really does permute
    // them, so the comparison must notice.
    let mut lying = lowered.clone();
    lying.final_layout = lying.initial_layout.clone();
    assert!(
        equivalent_modulo_layout(&circuit, &lying).is_err(),
        "the harness accepted a final layout that contradicts the emitted swaps"
    );
}
