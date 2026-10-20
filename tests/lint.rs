//! The lint boundary (§13.5): one firing and one non-firing case per rule,
//! deterministic output, and an extension rule registered from outside.

use std::sync::Arc;

use oqci::analysis::lint::{Diagnostic, LintContext, LintRule, Linter, Severity};
use oqci::compile::{CompilerConfig, compile};
use oqci::extension::{ExtensionError, ExtensionOrigin, Registry};
use oqci::ir::{Circuit, CircuitBuilder, ClbitId, GateKind, Param, QubitId};
use oqci::target::builtin;

fn circuit(qubits: u32, clbits: u32, f: impl FnOnce(&mut CircuitBuilder)) -> Circuit {
    let mut b = CircuitBuilder::new("t");
    b.alloc_qubits(qubits);
    b.alloc_clbits(clbits);
    f(&mut b);
    b.build().unwrap()
}

fn findings(circuit: &Circuit, rule: &str) -> Vec<Diagnostic> {
    Linter::default_rules()
        .lint(circuit, None)
        .diagnostics
        .into_iter()
        .filter(|d| d.rule == rule)
        .collect()
}

fn q(i: u32) -> QubitId {
    QubitId(i)
}

#[test]
fn cancellable_pair_fires_on_adjacent_inverses_only() {
    let fires = circuit(1, 0, |b| {
        b.h(q(0)).h(q(0));
    });
    let found = findings(&fires, "cancellable-pair");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].instructions, vec![0, 1]);
    assert_eq!(found[0].severity, Severity::Warning);

    // Something on the same wire in between blocks it, exactly as it blocks
    // the gate-cancellation pass.
    let blocked = circuit(1, 0, |b| {
        b.h(q(0)).x(q(0)).h(q(0));
    });
    assert!(findings(&blocked, "cancellable-pair").is_empty());
}

#[test]
fn identity_operation_distinguishes_exact_from_global_phase() {
    use std::f64::consts::TAU;
    let exact = circuit(1, 0, |b| {
        b.gate(GateKind::I, [q(0)])
            .rz(0.0, q(0))
            .rx(2.0 * TAU, q(0))
            .gate(GateKind::P(Param::concrete(TAU)), [q(0)]);
    });
    let found = findings(&exact, "identity-operation");
    assert_eq!(found.len(), 4, "{found:?}");
    assert!(found.iter().all(|d| d.severity == Severity::Warning));

    // Rz(2π) = −I: the identity only up to global phase, reported as info.
    let phase = circuit(1, 0, |b| {
        b.rz(TAU, q(0));
    });
    let found = findings(&phase, "identity-operation");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Info);

    let real = circuit(1, 0, |b| {
        b.rz(0.3, q(0)).rx(TAU + 1e-9, q(0));
    });
    assert!(findings(&real, "identity-operation").is_empty());
}

#[test]
fn unobservable_gate_uses_liveness_not_position() {
    // The X on q1 feeds the measured q0 through the CX: live. The H on q2
    // touches nothing anyone measures: dead.
    let c = circuit(3, 1, |b| {
        b.x(q(1)).h(q(2)).cx(q(1), q(0)).measure(q(0), ClbitId(0));
    });
    let found = findings(&c, "unobservable-gate");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].instructions, vec![1]);

    // A gate after the last measurement on its qubit is dead too.
    let after = circuit(1, 1, |b| {
        b.h(q(0)).measure(q(0), ClbitId(0)).x(q(0));
    });
    assert_eq!(
        findings(&after, "unobservable-gate")[0].instructions,
        vec![2]
    );

    // A circuit that never measures may be a subroutine: nothing is reported.
    let silent = circuit(2, 0, |b| {
        b.h(q(0)).x(q(1));
    });
    assert!(findings(&silent, "unobservable-gate").is_empty());
}

#[test]
fn unused_resource_reports_idle_qubits_and_overwritten_clbits() {
    let c = circuit(3, 2, |b| {
        b.h(q(0))
            .measure(q(0), ClbitId(0))
            .measure(q(1), ClbitId(0));
    });
    let found = findings(&c, "unused-resource");
    let messages: Vec<&str> = found.iter().map(|d| d.message.as_str()).collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("qubit 2 is declared but never used"))
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("classical bit 1 is declared but never written"))
    );
    let overwrite = found
        .iter()
        .find(|d| d.message.contains("written 2 times"))
        .unwrap();
    assert_eq!(overwrite.severity, Severity::Warning);

    let tidy = circuit(1, 1, |b| {
        b.h(q(0)).measure(q(0), ClbitId(0));
    });
    assert!(findings(&tidy, "unused-resource").is_empty());
}

#[test]
fn target_unsupported_needs_a_target_and_separates_decomposable_from_impossible() {
    let c = circuit(2, 2, |b| {
        b.h(q(0)).measure(q(0), ClbitId(0)).x(q(0)).reset(q(1));
    });
    // Silent without a target.
    assert!(findings(&c, "target-unsupported").is_empty());

    let report = Linter::default_rules().lint(&c, Some(&builtin::linear_nisq(2)));
    let found: Vec<&Diagnostic> = report
        .diagnostics
        .iter()
        .filter(|d| d.rule == "target-unsupported")
        .collect();
    // `h` decomposes (info); mid-circuit measurement and reset cannot run
    // on linear-nisq at all (error).
    assert!(
        found
            .iter()
            .any(|d| d.severity == Severity::Info && d.message.contains("h-to-rz-sx"))
    );
    assert!(
        found.iter().any(
            |d| d.severity == Severity::Error && d.message.contains("measures only at the end")
        )
    );
    assert!(
        found
            .iter()
            .any(|d| d.severity == Severity::Error && d.message.contains("cannot reset"))
    );
    assert_eq!(report.count_at_least(Severity::Error), 2);
}

#[test]
fn lint_output_is_ordered_and_repeatable() {
    let c = circuit(3, 2, |b| {
        b.h(q(0)).h(q(0)).rz(0.0, q(1)).measure(q(1), ClbitId(0));
    });
    let first = Linter::default_rules().lint(&c, Some(&builtin::linear_nisq(3)));
    let second = Linter::default_rules().lint(&c, Some(&builtin::linear_nisq(3)));
    assert_eq!(first, second);
    let firsts: Vec<Option<usize>> = first
        .diagnostics
        .iter()
        .map(|d| d.instructions.first().copied())
        .collect();
    let mut sorted = firsts.clone();
    sorted.sort();
    assert_eq!(firsts, sorted);
}

/// An extension rule: flags any `ccx`, as a team that avoids Toffolis might.
struct NoToffoli;

impl LintRule for NoToffoli {
    fn id(&self) -> &str {
        "no-toffoli"
    }

    fn description(&self) -> &str {
        "flags every ccx"
    }

    fn check(&self, context: &LintContext<'_>) -> Vec<Diagnostic> {
        context
            .circuit
            .instructions()
            .iter()
            .enumerate()
            .filter(|(_, inst)| {
                matches!(
                    inst,
                    oqci::ir::Instruction::Gate {
                        kind: GateKind::Ccx,
                        ..
                    }
                )
            })
            .map(|(index, _)| Diagnostic {
                rule: self.id().to_string(),
                severity: Severity::Warning,
                instructions: vec![index],
                qubits: vec![],
                message: "ccx".to_string(),
            })
            .collect()
    }
}

#[test]
fn an_extension_lint_rule_runs_through_compilation() {
    let mut registry = Registry::default();
    registry
        .register_lint_rule(ExtensionOrigin::new("test", "0"), Arc::new(NoToffoli))
        .unwrap();
    let config = CompilerConfig {
        lint: true,
        extensions: registry.clone(),
        ..CompilerConfig::default()
    };
    let artifacts = compile("qubit[3] q; ccx q[0], q[1], q[2];", &config).unwrap();
    let lint = artifacts.lint.unwrap();
    assert!(lint.rules.contains(&"no-toffoli".to_string()));
    assert!(lint.diagnostics.iter().any(|d| d.rule == "no-toffoli"));

    // A rule may not take a built-in's id.
    struct Impostor;
    impl LintRule for Impostor {
        fn id(&self) -> &str {
            "cancellable-pair"
        }
        fn description(&self) -> &str {
            ""
        }
        fn check(&self, _: &LintContext<'_>) -> Vec<Diagnostic> {
            Vec::new()
        }
    }
    let err = registry
        .register_lint_rule(ExtensionOrigin::new("test", "0"), Arc::new(Impostor))
        .unwrap_err();
    assert!(matches!(
        err,
        ExtensionError::DuplicateId { kind: "lint", .. }
    ));
}

#[test]
fn lint_is_off_unless_asked_for() {
    let artifacts = compile("qubit[1] q; h q[0]; h q[0];", &CompilerConfig::default()).unwrap();
    assert!(artifacts.lint.is_none());
}
