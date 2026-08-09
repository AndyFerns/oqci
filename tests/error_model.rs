//! Error-model coverage: every `IrError` variant is reached, and a match over
//! all known variants guards against silent gaps (subject to the external-crate
//! `#[non_exhaustive]` wildcard requirement).

use oqci::ir::{Angle, CircuitBuilder, ClbitId, GateKind, IrError, QubitId};

/// Collect one value of each of the eight `IrError` variants: seven via
/// `build()` and `CyclicGraph` constructed directly (enum-level
/// `#[non_exhaustive]` permits variant construction, only exhaustive matching
/// is blocked).
fn one_of_each() -> Vec<IrError> {
    let qubit_range = {
        let mut b = CircuitBuilder::new("e");
        let _q = b.alloc_qubit();
        b.gate(GateKind::X, [QubitId(9)]);
        b.build().expect_err("qubit range")
    };
    let clbit_range = {
        let mut b = CircuitBuilder::new("e");
        let q0 = b.alloc_qubit();
        b.measure(q0, ClbitId(9));
        b.build().expect_err("clbit range")
    };
    let arity = {
        let mut b = CircuitBuilder::new("e");
        let q0 = b.alloc_qubit();
        b.gate(GateKind::Cx, [q0]);
        b.build().expect_err("arity")
    };
    let duplicate = {
        let mut b = CircuitBuilder::new("e");
        let q0 = b.alloc_qubit();
        let _q1 = b.alloc_qubit();
        b.gate(GateKind::Cx, [q0, q0]);
        b.build().expect_err("duplicate")
    };
    let empty_name = {
        let mut b = CircuitBuilder::new("e");
        let q0 = b.alloc_qubit();
        b.gate(
            GateKind::Opaque {
                name: String::new(),
                params: vec![],
            },
            [q0],
        );
        b.build().expect_err("empty name")
    };
    let empty_operands = {
        let mut b = CircuitBuilder::new("e");
        b.gate(
            GateKind::Opaque {
                name: "custom".into(),
                params: vec![],
            },
            [],
        );
        b.build().expect_err("empty operands")
    };
    let non_finite = {
        let mut b = CircuitBuilder::new("e");
        let q0 = b.alloc_qubit();
        b.rx(Angle::new(f64::NAN), q0);
        b.build().expect_err("non finite")
    };

    vec![
        qubit_range,
        clbit_range,
        arity,
        duplicate,
        empty_name,
        empty_operands,
        non_finite,
        IrError::CyclicGraph,
    ]
}

/// A stable discriminant per known variant; the `_` arm is mandated by
/// `#[non_exhaustive]` in an external crate and must never be hit here.
fn discriminant(e: &IrError) -> &'static str {
    match e {
        IrError::QubitOutOfRange { .. } => "QubitOutOfRange",
        IrError::ClbitOutOfRange { .. } => "ClbitOutOfRange",
        IrError::GateArityMismatch { .. } => "GateArityMismatch",
        IrError::DuplicateQubit { .. } => "DuplicateQubit",
        IrError::EmptyOpaqueName => "EmptyOpaqueName",
        IrError::EmptyOpaqueOperands { .. } => "EmptyOpaqueOperands",
        IrError::NonFiniteAngle { .. } => "NonFiniteAngle",
        IrError::CyclicGraph => "CyclicGraph",
        _ => "unknown",
    }
}

/// Every variant is constructible/reachable and has a non-empty Display string.
#[test]
fn every_error_variant_is_constructible_and_displays() {
    let errors = one_of_each();
    assert_eq!(errors.len(), 8);
    for e in &errors {
        assert!(!format!("{e}").is_empty(), "empty Display for {e:?}");
    }
}

/// The match names every known variant: no reached variant falls to `_`.
#[test]
fn error_variant_match_is_total_over_known_variants() {
    let mut seen = std::collections::HashSet::new();
    for e in one_of_each() {
        let name = discriminant(&e);
        assert_ne!(name, "unknown", "unmatched variant: {e:?}");
        seen.insert(name);
    }
    assert_eq!(seen.len(), 8, "expected all 8 distinct variants");
}
