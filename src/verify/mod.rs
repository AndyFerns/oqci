//! Semantic verification during compilation.
//!
//! Stage G §3.1 classifies each transformation by the strongest checker that
//! covers it: none, structural (I1–I8 re-validation, target legality),
//! semantic in tests only, or **semantic in the compiler**. This module is the
//! last kind. When [`VerifyConfig::enabled`] is set, compilation checks that
//! parameter binding, every optimization pass that ran — extension passes
//! included — and target lowering each preserved what the circuit computes,
//! and refuses to produce an executable if one did not.
//!
//! # What a verdict means
//!
//! - **`Equivalent`** names the strength at which it was proved:
//!   - `identical`: the two circuits are the same instruction list;
//!   - `substitution`: binding replaced exactly the bound symbols, with
//!     exactly the bound values, and changed nothing else;
//!   - `operator`: the two unitaries agree on every input, up to one global
//!     phase (modulo layout, for lowering);
//!   - `initial_state`: they agree on `|0…0⟩`, the only state a program ever
//!     starts in;
//!   - `distribution`: the probability distributions over the measured
//!     classical bits agree.
//! - **`NotEquivalent`** means an observable difference: none of the above
//!   holds. With [`OnMismatch::Fail`] (the default) compilation stops with
//!   [`crate::compile::CompileError::SemanticMismatch`].
//! - **`Inconclusive`** means the checker could not decide, and says why: the
//!   circuit is wider than [`VerifyConfig::max_qubits`], a parameter is still
//!   symbolic, a gate is opaque, or a measurement or reset is followed by
//!   further work. An inconclusive verdict never fails compilation and is
//!   never counted as a pass (Stage G §3.1's inconclusive rate).
//!
//! Comparison is exhaustive over the computational basis — no random input
//! states, so no seed (§33.15). QIR emission is listed as not checked: there
//! is no interpreter for it here.
//!
//! # Why this is not the simulator the non-goals rule out
//!
//! §32 rules out "a custom quantum simulator replacing established simulator
//! frameworks". This checker never samples, takes no shot count, returns no
//! counts, has no noise model and no execution path — `Backend::execute` still
//! refuses in-process execution, and a test still asserts it. Its only output
//! is a verdict comparing two circuits OQCI itself produced, capped at a few
//! qubits. Execution remains the job of Aer, CUDA-Q and IBM hardware through
//! the Python adapters.
//!
//! # Independence
//!
//! The `statevector` submodule writes every gate's matrix from its definition and does not
//! read the decomposition rules or the pass tables it audits. It shares its
//! origin with the test-only simulator in `tests/support/`, so those two are
//! not independent of each other; Qiskit's `quantum_info`
//! (`python/tests/test_rules.py`) remains the external check on the rules.

mod statevector;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use num_complex::Complex64 as C;
use serde::Serialize;

use crate::ir::{Circuit, GateKind, Instruction, Param};
use crate::lowering::Lowered;
use statevector::{State, Unsimulable};

/// How (and whether) to verify a compilation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VerifyConfig {
    /// Whether to verify at all. Off by default: verification costs time
    /// exponential in the width it checks.
    pub enabled: bool,
    /// The widest circuit (in active qubits) the checker will simulate.
    pub max_qubits: u32,
    /// The numerical tolerance on every comparison.
    pub tolerance: f64,
    /// What a `NotEquivalent` verdict does.
    pub on_mismatch: OnMismatch,
}

impl Default for VerifyConfig {
    fn default() -> Self {
        VerifyConfig {
            enabled: false,
            max_qubits: 8,
            tolerance: 1e-9,
            on_mismatch: OnMismatch::Fail,
        }
    }
}

/// What to do when a transformation is found not to preserve semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnMismatch {
    /// Stop compilation; no executable is produced.
    #[default]
    Fail,
    /// Record the verdict and continue. For investigating a failure, never
    /// for producing results.
    Report,
}

/// How strongly an equivalence was established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    /// The same instruction list.
    Identical,
    /// An exact parameter substitution.
    Substitution,
    /// The same unitary, up to global phase.
    Operator,
    /// The same state from `|0…0⟩`, up to global phase.
    InitialState,
    /// The same distribution over the measured classical bits.
    Distribution,
}

/// Why the checker could not decide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InconclusiveReason {
    /// Wider than the configured cap.
    TooManyQubits {
        /// Active qubits the check would need.
        qubits: u32,
        /// The cap.
        cap: u32,
    },
    /// A parameter is still symbolic.
    SymbolicParameter {
        /// The symbol.
        symbol: String,
    },
    /// An opaque gate, whose semantics OQCI does not define.
    OpaqueGate {
        /// Its name.
        name: String,
    },
    /// A measurement is followed by further work on its qubit.
    NonTerminalMeasurement {
        /// Program index of the measurement.
        index: usize,
    },
    /// A reset of a qubit that had already been used.
    Reset {
        /// Program index of the reset.
        index: usize,
    },
}

/// The checker's conclusion about one transformation.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// Semantics preserved, at the given strength.
    Equivalent {
        /// How strongly.
        strength: Strength,
    },
    /// An observable difference.
    NotEquivalent {
        /// The weakest comparison attempted, which also failed.
        strength_tested: Strength,
        /// How far from equal that comparison was.
        residual: f64,
    },
    /// Not decided.
    Inconclusive {
        /// Why.
        reason: InconclusiveReason,
    },
}

/// One transformation and its verdict.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Check {
    /// `binding`, `pass:<id>#<position>`, or `lowering`.
    pub transformation: String,
    /// What the checker concluded.
    pub verdict: Verdict,
}

/// Every check a compilation made, with totals.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct VerificationReport {
    /// One entry per checked transformation, in pipeline order.
    pub checks: Vec<Check>,
    /// How many were proved equivalent.
    pub equivalent: usize,
    /// How many were found not equivalent.
    pub not_equivalent: usize,
    /// How many could not be decided.
    pub inconclusive: usize,
    /// Transformations the checker does not cover at all.
    pub not_checked: Vec<String>,
}

impl VerificationReport {
    /// An empty report. QIR emission is always listed as not checked.
    #[must_use]
    pub fn new() -> Self {
        VerificationReport {
            not_checked: vec!["qir-emission".to_string()],
            ..VerificationReport::default()
        }
    }

    /// Records a verdict.
    pub fn push(&mut self, transformation: impl Into<String>, verdict: Verdict) {
        match verdict {
            Verdict::Equivalent { .. } => self.equivalent += 1,
            Verdict::NotEquivalent { .. } => self.not_equivalent += 1,
            Verdict::Inconclusive { .. } => self.inconclusive += 1,
        }
        self.checks.push(Check {
            transformation: transformation.into(),
            verdict,
        });
    }

    /// The first transformation found not to preserve semantics.
    #[must_use]
    pub fn first_mismatch(&self) -> Option<&Check> {
        self.checks
            .iter()
            .find(|check| matches!(check.verdict, Verdict::NotEquivalent { .. }))
    }
}

// --- Binding -----------------------------------------------------------------

/// Checks parameter binding by exact substitution, independently of
/// [`crate::ir::bind_parameters`]: the bound circuit must be the parsed one,
/// instruction for instruction, with each bound symbol replaced by exactly its
/// value and every other parameter unchanged.
#[must_use]
pub fn check_binding(
    parsed: &Circuit,
    bound: &Circuit,
    bindings: &HashMap<String, f64>,
) -> Verdict {
    let mismatch = || Verdict::NotEquivalent {
        strength_tested: Strength::Substitution,
        residual: 1.0,
    };
    if parsed.num_qubits() != bound.num_qubits()
        || parsed.num_clbits() != bound.num_clbits()
        || parsed.len() != bound.len()
    {
        return mismatch();
    }
    for (before, after) in parsed.instructions().iter().zip(bound.instructions()) {
        let same = match (before, after) {
            (
                Instruction::Gate {
                    kind: kind_a,
                    qubits: qubits_a,
                },
                Instruction::Gate {
                    kind: kind_b,
                    qubits: qubits_b,
                },
            ) => {
                qubits_a == qubits_b
                    && kind_a.mnemonic() == kind_b.mnemonic()
                    && kind_a.params().len() == kind_b.params().len()
                    && kind_a
                        .params()
                        .iter()
                        .zip(kind_b.params())
                        .all(|(a, b)| substituted(a, &b, bindings))
            }
            (a, b) => a == b,
        };
        if !same {
            return mismatch();
        }
    }
    Verdict::Equivalent {
        strength: Strength::Substitution,
    }
}

fn substituted(before: &Param, after: &Param, bindings: &HashMap<String, f64>) -> bool {
    match (before, after.as_concrete()) {
        (Param::Symbol(name), Some(value)) => bindings
            .get(name)
            .is_some_and(|bound| bound.to_bits() == value.radians().to_bits()),
        _ => before == after,
    }
}

// --- Passes ------------------------------------------------------------------

/// Checks that an optimization pass preserved semantics.
#[must_use]
pub fn check_pass(before: &Circuit, after: &Circuit, config: &VerifyConfig) -> Verdict {
    if before == after {
        return Verdict::Equivalent {
            strength: Strength::Identical,
        };
    }
    let (a, b) = match (Program::read(before), Program::read(after)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(reason), _) | (_, Err(reason)) => return Verdict::Inconclusive { reason },
    };
    // Both circuits act on the same register: compare on the qubits either
    // touches, compacted.
    let active: BTreeSet<u32> = a.touched.union(&b.touched).copied().collect();
    let compact = Compact::new(&active);
    let width = compact.width();
    if width > config.max_qubits as usize {
        return too_wide(width, config);
    }
    let same_measurements = a.measures == b.measures;
    let embed = |logical: usize| logical; // identity layout
    compare(
        &a,
        &b,
        &compact,
        &compact,
        width,
        width,
        embed,
        same_measurements,
        config,
    )
}

// --- Lowering ----------------------------------------------------------------

/// Checks that target lowering preserved semantics, modulo layout: the
/// lowered circuit, run with logical qubit `i` placed on its initial physical
/// qubit, must produce what the original produces with qubit `i` read from its
/// final physical qubit, and leave every other physical qubit in `|0⟩`.
///
/// Trusting the reported final layout is the right specification here: an
/// executable's results are interpreted through that same layout, so a layout
/// that disagreed with the circuit would be wrong for every consumer.
#[must_use]
pub fn check_lowering(original: &Circuit, lowered: &Lowered, config: &VerifyConfig) -> Verdict {
    let (a, b) = match (Program::read(original), Program::read(&lowered.circuit)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(reason), _) | (_, Err(reason)) => return Verdict::Inconclusive { reason },
    };
    let n = original.num_qubits();
    let mut initial = Vec::new();
    let mut last = Vec::new();
    for logical in 0..n {
        let qubit = crate::ir::QubitId(logical);
        match (
            lowered.initial_layout.physical(qubit),
            lowered.final_layout.physical(qubit),
        ) {
            (Some(start), Some(end)) => {
                initial.push(start.index());
                last.push(end.index());
            }
            _ => {
                return Verdict::NotEquivalent {
                    strength_tested: Strength::Operator,
                    residual: 1.0,
                };
            }
        }
    }

    let logical: BTreeSet<u32> = (0..n).collect();
    let logical_compact = Compact::new(&logical);
    let physical: BTreeSet<u32> = b
        .touched
        .iter()
        .copied()
        .chain(initial.iter().copied())
        .chain(last.iter().copied())
        .collect();
    let physical_compact = Compact::new(&physical);
    let width = physical_compact.width();
    if width > config.max_qubits as usize || n as usize > config.max_qubits as usize {
        return too_wide(width.max(n as usize), config);
    }

    // Measurements must land on the same classical bits, read from where each
    // logical qubit finally sits.
    let relocated: BTreeMap<u32, u32> = a
        .measures
        .iter()
        .map(|(clbit, qubit)| (*clbit, last[*qubit as usize]))
        .collect();
    let same_measurements = relocated == b.measures;

    // The original's register, compacted, is the logical one. The lowered
    // register is the physical one, and the two are related by the layouts.
    let start = |q: usize| physical_compact.index(initial[q]);
    let end = |q: usize| physical_compact.index(last[q]);
    let lowered_input = |logical_bits: usize| place(logical_bits, n as usize, start);
    let embed = |logical_bits: usize| place(logical_bits, n as usize, end);

    compare_with_inputs(
        &a,
        &b,
        &logical_compact,
        &physical_compact,
        n as usize,
        width,
        lowered_input,
        embed,
        same_measurements,
        config,
    )
}

/// Maps a bit pattern over `n` logical qubits onto compact physical indices.
fn place(bits: usize, n: usize, to: impl Fn(usize) -> usize) -> usize {
    (0..n)
        .filter(|q| bits & (1 << q) != 0)
        .map(|q| 1 << to(q))
        .sum()
}

// --- Shared machinery --------------------------------------------------------

/// A circuit read for comparison: its gates, its terminal measurements, and
/// the qubits it touches.
struct Program {
    gates: Vec<(GateKind, Vec<u32>)>,
    /// clbit -> measured qubit (the last write wins, as at runtime).
    measures: BTreeMap<u32, u32>,
    touched: BTreeSet<u32>,
}

impl Program {
    fn read(circuit: &Circuit) -> Result<Program, InconclusiveReason> {
        let mut gates = Vec::new();
        let mut measures = BTreeMap::new();
        let mut touched = BTreeSet::new();
        let mut measured_at: HashMap<u32, usize> = HashMap::new();

        for (index, instruction) in circuit.instructions().iter().enumerate() {
            match instruction {
                Instruction::Reset { qubit } => {
                    // A reset of a fresh qubit acts on |0> and does nothing.
                    if touched.contains(&qubit.0) {
                        return Err(InconclusiveReason::Reset { index });
                    }
                }
                Instruction::Measure { qubit, target } => {
                    if let Some(earlier) = measured_at.get(&qubit.0) {
                        return Err(InconclusiveReason::NonTerminalMeasurement { index: *earlier });
                    }
                    measured_at.insert(qubit.0, index);
                    measures.insert(target.0, qubit.0);
                    touched.insert(qubit.0);
                }
                Instruction::Gate { kind, qubits } => {
                    for qubit in qubits {
                        if let Some(earlier) = measured_at.get(&qubit.0) {
                            return Err(InconclusiveReason::NonTerminalMeasurement {
                                index: *earlier,
                            });
                        }
                    }
                    if let GateKind::Opaque { name, .. } = kind {
                        return Err(InconclusiveReason::OpaqueGate { name: name.clone() });
                    }
                    if let Some(symbol) = kind.symbols().into_iter().next() {
                        return Err(InconclusiveReason::SymbolicParameter { symbol });
                    }
                    touched.extend(qubits.iter().map(|q| q.0));
                    gates.push((kind.clone(), qubits.iter().map(|q| q.0).collect()));
                }
            }
        }
        Ok(Program {
            gates,
            measures,
            touched,
        })
    }

    fn run(&self, compact: &Compact, width: usize, input: usize) -> Result<State, Unsimulable> {
        let mut state = State::basis(width, input);
        for (kind, qubits) in &self.gates {
            let mapped: Vec<usize> = qubits.iter().map(|q| compact.index(*q)).collect();
            state.gate(kind, &mapped)?;
        }
        Ok(state)
    }

    /// The distribution over measured classical bits, from `|0…0⟩`.
    fn distribution(
        &self,
        compact: &Compact,
        width: usize,
    ) -> Result<BTreeMap<u64, f64>, Unsimulable> {
        let state = self.run(compact, width, 0)?;
        let mut distribution = BTreeMap::new();
        for (index, amplitude) in state.amplitudes().iter().enumerate() {
            let probability = amplitude.norm_sqr();
            if probability == 0.0 {
                continue;
            }
            let mut outcome = 0u64;
            for (clbit, qubit) in &self.measures {
                if index & (1 << compact.index(*qubit)) != 0 {
                    outcome |= 1 << clbit;
                }
            }
            *distribution.entry(outcome).or_insert(0.0) += probability;
        }
        Ok(distribution)
    }
}

/// A dense renumbering of the qubits a comparison needs.
struct Compact {
    map: BTreeMap<u32, usize>,
}

impl Compact {
    fn new(active: &BTreeSet<u32>) -> Self {
        Compact {
            map: active.iter().enumerate().map(|(i, q)| (*q, i)).collect(),
        }
    }

    fn width(&self) -> usize {
        self.map.len()
    }

    fn index(&self, qubit: u32) -> usize {
        self.map[&qubit]
    }
}

fn too_wide(qubits: usize, config: &VerifyConfig) -> Verdict {
    Verdict::Inconclusive {
        reason: InconclusiveReason::TooManyQubits {
            qubits: u32::try_from(qubits).unwrap_or(u32::MAX),
            cap: config.max_qubits,
        },
    }
}

fn unsimulable(error: Unsimulable) -> Verdict {
    Verdict::Inconclusive {
        reason: match error {
            Unsimulable::Symbolic(symbol) => InconclusiveReason::SymbolicParameter { symbol },
            Unsimulable::Opaque(name) => InconclusiveReason::OpaqueGate { name },
        },
    }
}

/// A comparison where both circuits share one register (a pass).
#[allow(clippy::too_many_arguments)]
fn compare(
    a: &Program,
    b: &Program,
    a_compact: &Compact,
    b_compact: &Compact,
    a_width: usize,
    b_width: usize,
    embed: impl Fn(usize) -> usize,
    same_measurements: bool,
    config: &VerifyConfig,
) -> Verdict {
    compare_with_inputs(
        a,
        b,
        a_compact,
        b_compact,
        a_width,
        b_width,
        &embed,
        &embed,
        same_measurements,
        config,
    )
}

/// The strength ladder: operator, then initial state, then distribution.
///
/// `b_input(k)` is where the second circuit starts for the first circuit's
/// basis input `k`; `embed(j)` is where the first circuit's output amplitude
/// `j` is expected in the second circuit's register. The first circuit's
/// register has `a_width` qubits, all of them inputs.
#[allow(clippy::too_many_arguments)]
fn compare_with_inputs(
    a: &Program,
    b: &Program,
    a_compact: &Compact,
    b_compact: &Compact,
    a_width: usize,
    b_width: usize,
    b_input: impl Fn(usize) -> usize,
    embed: impl Fn(usize) -> usize,
    same_measurements: bool,
    config: &VerifyConfig,
) -> Verdict {
    let tolerance = config.tolerance;
    // Inner product of a's output (embedded) with b's output, for one input.
    let overlap = |input: usize| -> Result<C, Unsimulable> {
        let expected = a.run(a_compact, a_width, input)?;
        let actual = b.run(b_compact, b_width, b_input(input))?;
        Ok(expected
            .amplitudes()
            .iter()
            .enumerate()
            .map(|(j, e)| actual.amplitudes()[embed(j)].conj() * e)
            .sum())
    };

    let mut last_residual = 1.0;
    let mut last_strength = Strength::Distribution;
    if same_measurements {
        // Operator: the summed trace, modulus taken last, so a per-input
        // phase discrepancy (a wrong relative phase) cannot hide.
        let dimension = 1usize << a_width;
        let mut trace = C::new(0.0, 0.0);
        for input in 0..dimension {
            match overlap(input) {
                Ok(value) => trace += value,
                Err(error) => return unsimulable(error),
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let residual = (trace.norm() / dimension as f64 - 1.0).abs();
        if residual < tolerance {
            return Verdict::Equivalent {
                strength: Strength::Operator,
            };
        }

        // Initial state: programs only ever start in |0...0>.
        let residual = match overlap(0) {
            Ok(value) => (value.norm() - 1.0).abs(),
            Err(error) => return unsimulable(error),
        };
        if residual < tolerance {
            return Verdict::Equivalent {
                strength: Strength::InitialState,
            };
        }
        last_residual = residual;
        last_strength = Strength::InitialState;
    }

    // Distribution over the measured classical bits.
    if !a.measures.is_empty() || !b.measures.is_empty() {
        let (pa, pb) = match (
            a.distribution(a_compact, a_width),
            b.distribution(b_compact, b_width),
        ) {
            (Ok(pa), Ok(pb)) => (pa, pb),
            (Err(error), _) | (_, Err(error)) => return unsimulable(error),
        };
        let keys: BTreeSet<&u64> = pa.keys().chain(pb.keys()).collect();
        let residual = keys
            .into_iter()
            .map(|k| (pa.get(k).unwrap_or(&0.0) - pb.get(k).unwrap_or(&0.0)).abs())
            .fold(0.0, f64::max);
        if residual < tolerance {
            return Verdict::Equivalent {
                strength: Strength::Distribution,
            };
        }
        last_residual = residual;
        last_strength = Strength::Distribution;
    }

    Verdict::NotEquivalent {
        strength_tested: last_strength,
        residual: last_residual,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CircuitBuilder, ClbitId, QubitId};
    use crate::lowering::{LoweringConfig, lower};
    use crate::target::builtin;

    fn config() -> VerifyConfig {
        VerifyConfig {
            enabled: true,
            ..VerifyConfig::default()
        }
    }

    fn build(f: impl FnOnce(&mut CircuitBuilder)) -> Circuit {
        let mut b = CircuitBuilder::new("t");
        b.alloc_qubits(3);
        b.alloc_clbits(3);
        f(&mut b);
        b.build().unwrap()
    }

    #[test]
    fn an_identical_circuit_is_identical() {
        let c = build(|b| {
            b.h(QubitId(0));
        });
        assert_eq!(
            check_pass(&c, &c, &config()),
            Verdict::Equivalent {
                strength: Strength::Identical
            }
        );
    }

    #[test]
    fn a_correct_rewrite_is_operator_equivalent() {
        let before = build(|b| {
            b.h(QubitId(0)).h(QubitId(0)).x(QubitId(1));
        });
        let after = build(|b| {
            b.x(QubitId(1));
        });
        assert_eq!(
            check_pass(&before, &after, &config()),
            Verdict::Equivalent {
                strength: Strength::Operator
            }
        );
    }

    #[test]
    fn a_relative_phase_error_is_caught() {
        // S and Sdg agree on |0> and on every basis state's probability, but
        // not as operators — and not on a superposition.
        let before = build(|b| {
            b.h(QubitId(0))
                .gate(GateKind::S, [QubitId(0)])
                .h(QubitId(0));
        });
        let after = build(|b| {
            b.h(QubitId(0))
                .gate(GateKind::Sdg, [QubitId(0)])
                .h(QubitId(0));
        });
        assert!(matches!(
            check_pass(&before, &after, &config()),
            Verdict::NotEquivalent { .. }
        ));
    }

    #[test]
    fn removing_a_diagonal_gate_before_measurement_is_distribution_equivalent() {
        let before = build(|b| {
            b.h(QubitId(0))
                .rz(0.7, QubitId(0))
                .measure(QubitId(0), ClbitId(0));
        });
        let after = build(|b| {
            b.h(QubitId(0)).measure(QubitId(0), ClbitId(0));
        });
        assert_eq!(
            check_pass(&before, &after, &config()),
            Verdict::Equivalent {
                strength: Strength::Distribution
            }
        );
    }

    #[test]
    fn a_gate_on_the_initial_state_only_is_initial_state_equivalent() {
        // Rz on |0> is a phase; removing it at the start of a program is
        // correct for every program, though not as an operator.
        let before = build(|b| {
            b.rz(0.4, QubitId(0)).h(QubitId(0));
        });
        let after = build(|b| {
            b.h(QubitId(0));
        });
        assert_eq!(
            check_pass(&before, &after, &config()),
            Verdict::Equivalent {
                strength: Strength::InitialState
            }
        );
    }

    #[test]
    fn inconclusive_reasons_are_reported() {
        let symbolic = build(|b| {
            b.rz(Param::symbol("theta"), QubitId(0));
        });
        let other = build(|_| {});
        assert!(matches!(
            check_pass(&symbolic, &other, &config()),
            Verdict::Inconclusive {
                reason: InconclusiveReason::SymbolicParameter { .. }
            }
        ));

        let mid = build(|b| {
            b.measure(QubitId(0), ClbitId(0)).h(QubitId(0));
        });
        assert!(matches!(
            check_pass(&mid, &other, &config()),
            Verdict::Inconclusive {
                reason: InconclusiveReason::NonTerminalMeasurement { index: 0 }
            }
        ));

        let reset = build(|b| {
            b.h(QubitId(0)).reset(QubitId(0));
        });
        assert!(matches!(
            check_pass(&reset, &other, &config()),
            Verdict::Inconclusive {
                reason: InconclusiveReason::Reset { index: 1 }
            }
        ));

        let opaque = build(|b| {
            b.gate(
                GateKind::Opaque {
                    name: "mystery".into(),
                    params: vec![],
                },
                [QubitId(0)],
            );
        });
        assert!(matches!(
            check_pass(&opaque, &other, &config()),
            Verdict::Inconclusive {
                reason: InconclusiveReason::OpaqueGate { .. }
            }
        ));

        let narrow = VerifyConfig {
            max_qubits: 1,
            ..config()
        };
        let wide = build(|b| {
            b.cx(QubitId(0), QubitId(1));
        });
        assert!(matches!(
            check_pass(&wide, &other, &narrow),
            Verdict::Inconclusive {
                reason: InconclusiveReason::TooManyQubits { qubits: 2, cap: 1 }
            }
        ));
    }

    #[test]
    fn a_leading_reset_is_harmless() {
        let with_reset = build(|b| {
            b.reset(QubitId(0)).h(QubitId(0));
        });
        let without = build(|b| {
            b.h(QubitId(0));
        });
        assert!(matches!(
            check_pass(&with_reset, &without, &config()),
            Verdict::Equivalent { .. }
        ));
    }

    #[test]
    fn lowering_onto_a_line_is_equivalent_modulo_layout() {
        // A CX between the ends of a 3-qubit line forces routing.
        let circuit = build(|b| {
            b.h(QubitId(0))
                .cx(QubitId(0), QubitId(2))
                .measure(QubitId(0), ClbitId(0))
                .measure(QubitId(2), ClbitId(2));
        });
        let lowered = lower(
            &circuit,
            &builtin::linear_nisq(3),
            &LoweringConfig::default(),
        )
        .unwrap();
        assert!(lowered.swaps_inserted > 0, "the test must exercise routing");
        assert!(matches!(
            check_lowering(&circuit, &lowered, &config()),
            Verdict::Equivalent { .. }
        ));
    }

    #[test]
    fn a_forged_final_layout_is_caught() {
        let circuit = build(|b| {
            b.x(QubitId(0)).cx(QubitId(0), QubitId(2));
        });
        let mut lowered = lower(
            &circuit,
            &builtin::linear_nisq(3),
            &LoweringConfig::default(),
        )
        .unwrap();
        // Claim the qubits ended where they started.
        lowered.final_layout = lowered.initial_layout.clone();
        assert!(lowered.swaps_inserted > 0);
        assert!(matches!(
            check_lowering(&circuit, &lowered, &config()),
            Verdict::NotEquivalent { .. }
        ));
    }

    #[test]
    fn binding_is_checked_by_exact_substitution() {
        let parsed = build(|b| {
            b.rz(Param::symbol("theta"), QubitId(0)).rx(0.5, QubitId(1));
        });
        let bindings = HashMap::from([("theta".to_string(), 1.25)]);
        let bound = crate::ir::bind_parameters(&parsed, &bindings).unwrap();
        assert_eq!(
            check_binding(&parsed, &bound, &bindings),
            Verdict::Equivalent {
                strength: Strength::Substitution
            }
        );

        let wrong = build(|b| {
            b.rz(1.0, QubitId(0)).rx(0.5, QubitId(1));
        });
        assert!(matches!(
            check_binding(&parsed, &wrong, &bindings),
            Verdict::NotEquivalent { .. }
        ));
    }
}
