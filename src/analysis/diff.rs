//! Structural diffing of two circuits.
//!
//! Answers "what did that pass actually do?" in the same shape a reader
//! already understands from `git diff`: a sequence of kept, removed and added
//! instructions. It is computed **independently** of any pass's own reporting
//! — a pass claiming it cancelled two gates is a claim; this module checks the
//! circuits.
//!
//! The algorithm is a textbook longest-common-subsequence alignment over the
//! two instruction lists. That is `O(n·m)` in time and memory, which is fine
//! at the circuit sizes this project targets (and the sizes a human inspects
//! interactively); a very large circuit would want a Myers-style diff instead.

use crate::ir::{Circuit, Instruction};

/// One aligned position in a circuit diff.
#[derive(Debug, Clone, PartialEq)]
pub enum DiffEntry {
    /// Present in both circuits.
    Unchanged(Instruction),
    /// Present only in the "before" circuit.
    Removed(Instruction),
    /// Present only in the "after" circuit.
    Added(Instruction),
}

impl DiffEntry {
    /// The instruction this entry refers to, whatever its disposition.
    #[must_use]
    pub fn instruction(&self) -> &Instruction {
        match self {
            DiffEntry::Unchanged(i) | DiffEntry::Removed(i) | DiffEntry::Added(i) => i,
        }
    }

    /// The `git`-style marker for this entry: `' '`, `'-'` or `'+'`.
    #[must_use]
    pub const fn marker(&self) -> char {
        match self {
            DiffEntry::Unchanged(_) => ' ',
            DiffEntry::Removed(_) => '-',
            DiffEntry::Added(_) => '+',
        }
    }
}

/// The result of aligning two circuits.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CircuitDiff {
    /// Aligned entries, in order.
    pub entries: Vec<DiffEntry>,
}

impl CircuitDiff {
    /// `true` if the two circuits have identical instruction sequences.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries
            .iter()
            .all(|e| matches!(e, DiffEntry::Unchanged(_)))
    }

    /// Number of instructions present only in the "before" circuit.
    #[must_use]
    pub fn removed_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e, DiffEntry::Removed(_)))
            .count()
    }

    /// Number of instructions present only in the "after" circuit.
    #[must_use]
    pub fn added_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e, DiffEntry::Added(_)))
            .count()
    }
}

/// Aligns two circuits' instruction sequences.
///
/// ```
/// use oqci::analysis::diff_circuits;
/// use oqci::ir::CircuitBuilder;
///
/// let mut before = CircuitBuilder::new("c");
/// let q0 = before.alloc_qubit();
/// before.h(q0).x(q0).h(q0);
///
/// let mut after = CircuitBuilder::new("c");
/// let q0 = after.alloc_qubit();
/// after.h(q0).h(q0);
///
/// let diff = diff_circuits(&before.build().unwrap(), &after.build().unwrap());
/// assert_eq!(diff.removed_count(), 1);
/// assert_eq!(diff.added_count(), 0);
/// ```
#[must_use]
pub fn diff_circuits(before: &Circuit, after: &Circuit) -> CircuitDiff {
    CircuitDiff {
        entries: align(before.instructions(), after.instructions()),
    }
}

/// Longest-common-subsequence alignment of two instruction slices.
fn align(before: &[Instruction], after: &[Instruction]) -> Vec<DiffEntry> {
    let (n, m) = (before.len(), after.len());

    // lcs[i][j] = length of the LCS of before[i..] and after[j..].
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if before[i] == after[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    // Walk the table forwards, preferring a match, then whichever side keeps
    // the longer common subsequence available.
    let mut entries = Vec::with_capacity(n.max(m));
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if before[i] == after[j] {
            entries.push(DiffEntry::Unchanged(before[i].clone()));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            entries.push(DiffEntry::Removed(before[i].clone()));
            i += 1;
        } else {
            entries.push(DiffEntry::Added(after[j].clone()));
            j += 1;
        }
    }
    entries.extend(before[i..].iter().cloned().map(DiffEntry::Removed));
    entries.extend(after[j..].iter().cloned().map(DiffEntry::Added));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CircuitBuilder, GateKind, QubitId};

    fn circuit(build: impl FnOnce(&mut CircuitBuilder)) -> Circuit {
        let mut b = CircuitBuilder::new("c");
        b.alloc_qubits(3);
        build(&mut b);
        b.build().unwrap()
    }

    #[test]
    fn identical_circuits_produce_no_changes() {
        let a = circuit(|b| {
            b.h(QubitId(0)).cx(QubitId(0), QubitId(1));
        });
        let diff = diff_circuits(&a, &a);
        assert!(diff.is_empty());
        assert_eq!(diff.entries.len(), 2);
        assert!(
            diff.entries
                .iter()
                .all(|e| matches!(e, DiffEntry::Unchanged(_)))
        );
    }

    #[test]
    fn a_removed_gate_is_reported_once() {
        let before = circuit(|b| {
            b.h(QubitId(0)).x(QubitId(0)).h(QubitId(0));
        });
        let after = circuit(|b| {
            b.h(QubitId(0)).h(QubitId(0));
        });

        let diff = diff_circuits(&before, &after);
        assert_eq!(diff.removed_count(), 1);
        assert_eq!(diff.added_count(), 0);
        assert_eq!(
            diff.entries
                .iter()
                .find(|e| matches!(e, DiffEntry::Removed(_)))
                .map(DiffEntry::instruction),
            Some(&Instruction::Gate {
                kind: GateKind::X,
                qubits: vec![QubitId(0)]
            })
        );
    }

    #[test]
    fn an_added_gate_is_reported_once() {
        let before = circuit(|b| {
            b.h(QubitId(0));
        });
        let after = circuit(|b| {
            b.h(QubitId(0)).x(QubitId(1));
        });

        let diff = diff_circuits(&before, &after);
        assert_eq!(diff.added_count(), 1);
        assert_eq!(diff.removed_count(), 0);
    }

    #[test]
    fn a_replacement_shows_as_a_removal_plus_an_addition() {
        // What rotation merging looks like: two gates become one.
        let before = circuit(|b| {
            b.rz(0.25, QubitId(0)).rz(0.5, QubitId(0));
        });
        let after = circuit(|b| {
            b.rz(0.75, QubitId(0));
        });

        let diff = diff_circuits(&before, &after);
        assert_eq!(diff.removed_count(), 2);
        assert_eq!(diff.added_count(), 1);
        assert!(!diff.is_empty());
    }

    #[test]
    fn everything_removed_when_the_after_circuit_is_empty() {
        let before = circuit(|b| {
            b.h(QubitId(0)).x(QubitId(1));
        });
        let after = circuit(|_| {});

        let diff = diff_circuits(&before, &after);
        assert_eq!(diff.removed_count(), 2);
        assert_eq!(diff.entries.len(), 2);
    }

    #[test]
    fn common_prefix_and_suffix_are_preserved_around_a_change() {
        let before = circuit(|b| {
            b.h(QubitId(0)).x(QubitId(1)).h(QubitId(2));
        });
        let after = circuit(|b| {
            b.h(QubitId(0)).h(QubitId(2));
        });

        let diff = diff_circuits(&before, &after);
        let markers: String = diff.entries.iter().map(DiffEntry::marker).collect();
        assert_eq!(markers, " - ");
    }

    #[test]
    fn markers_render_git_style() {
        assert_eq!(
            DiffEntry::Unchanged(Instruction::Reset { qubit: QubitId(0) }).marker(),
            ' '
        );
        assert_eq!(
            DiffEntry::Removed(Instruction::Reset { qubit: QubitId(0) }).marker(),
            '-'
        );
        assert_eq!(
            DiffEntry::Added(Instruction::Reset { qubit: QubitId(0) }).marker(),
            '+'
        );
    }
}
