//! Two negative tests per `CircuitBuilder::build()` validation invariant
//! (a boundary case and a clearly-invalid case), plus positive boundary
//! controls. Complements the single-negative unit tests in `src/ir/qc.rs`.

use oqci::ir::{Angle, CircuitBuilder, ClbitId, GateKind, IrError, QubitId};

/// I1 boundary: a qubit index exactly equal to the register size is rejected.
#[test]
fn qubit_range_boundary_index_equals_count() {
    let mut b = CircuitBuilder::new("i1-boundary");
    let _q = b.alloc_qubits(2);
    b.gate(GateKind::X, [QubitId(2)]); // valid ids are 0,1
    let err = b.build().expect_err("index == count must be out of range");
    assert!(matches!(err, IrError::QubitOutOfRange { .. }));
}

/// I1 clearly invalid: a wildly out-of-bounds qubit index is rejected.
#[test]
fn qubit_range_grossly_out_of_bounds() {
    let mut b = CircuitBuilder::new("i1-gross");
    let _q = b.alloc_qubit();
    b.gate(GateKind::X, [QubitId(9999)]);
    let err = b.build().expect_err("gross index must be out of range");
    assert!(matches!(err, IrError::QubitOutOfRange { .. }));
}

/// I2 boundary: a clbit index exactly equal to the register size is rejected.
#[test]
fn clbit_range_boundary_index_equals_count() {
    let mut b = CircuitBuilder::new("i2-boundary");
    let q0 = b.alloc_qubit();
    let _c = b.alloc_clbits(1);
    b.measure(q0, ClbitId(1)); // valid ids are 0
    let err = b.build().expect_err("index == count must be out of range");
    assert!(matches!(err, IrError::ClbitOutOfRange { .. }));
}

/// I2 clearly invalid: a wildly out-of-bounds clbit index is rejected.
#[test]
fn clbit_range_grossly_out_of_bounds() {
    let mut b = CircuitBuilder::new("i2-gross");
    let q0 = b.alloc_qubit();
    b.measure(q0, ClbitId(9999));
    let err = b.build().expect_err("gross index must be out of range");
    assert!(matches!(err, IrError::ClbitOutOfRange { .. }));
}

/// I3 boundary: an arity off-by-one (CX given one operand) is rejected.
#[test]
fn arity_off_by_one_cx_with_one_operand() {
    let mut b = CircuitBuilder::new("i3-boundary");
    let q0 = b.alloc_qubit();
    b.gate(GateKind::Cx, [q0]); // CX needs 2
    let err = b.build().expect_err("off-by-one arity must be rejected");
    assert!(matches!(
        err,
        IrError::GateArityMismatch {
            expected: 2,
            found: 1,
            ..
        }
    ));
}

/// I3 clearly invalid: CCX given zero operands is rejected on arity.
#[test]
fn arity_grossly_wrong_ccx_with_zero_operands() {
    let mut b = CircuitBuilder::new("i3-gross");
    let _q = b.alloc_qubits(3);
    b.gate(GateKind::Ccx, []); // CCX needs 3
    let err = b.build().expect_err("zero operands must be rejected");
    assert!(matches!(
        err,
        IrError::GateArityMismatch {
            expected: 3,
            found: 0,
            ..
        }
    ));
}

/// I4 boundary: the minimal duplicate (CX on the same qubit twice) is rejected.
#[test]
fn duplicate_minimal_cx_same_qubit() {
    let mut b = CircuitBuilder::new("i4-boundary");
    let q0 = b.alloc_qubit();
    let _q1 = b.alloc_qubit();
    b.gate(GateKind::Cx, [q0, q0]);
    let err = b.build().expect_err("duplicate operand must be rejected");
    assert!(matches!(err, IrError::DuplicateQubit { .. }));
}

/// I4 clearly invalid: CCX with all three operands identical is rejected.
#[test]
fn duplicate_all_three_ccx_same_qubit() {
    let mut b = CircuitBuilder::new("i4-gross");
    let q0 = b.alloc_qubit();
    let _rest = b.alloc_qubits(2);
    b.gate(GateKind::Ccx, [q0, q0, q0]);
    let err = b
        .build()
        .expect_err("all-identical operands must be rejected");
    assert!(matches!(err, IrError::DuplicateQubit { .. }));
}

/// I5 variant a: an opaque gate with an empty name (single qubit) is rejected.
#[test]
fn opaque_empty_name_single_qubit() {
    let mut b = CircuitBuilder::new("i5-a");
    let q0 = b.alloc_qubit();
    b.gate(
        GateKind::Opaque {
            name: String::new(),
            params: vec![],
        },
        [q0],
    );
    let err = b.build().expect_err("empty opaque name must be rejected");
    assert_eq!(err, IrError::EmptyOpaqueName);
}

/// I5 variant b: an empty opaque name is rejected even with params and 2 qubits.
#[test]
fn opaque_empty_name_multi_qubit_with_params() {
    let mut b = CircuitBuilder::new("i5-b");
    let q0 = b.alloc_qubit();
    let q1 = b.alloc_qubit();
    b.gate(
        GateKind::Opaque {
            name: String::new(),
            params: vec![Angle::new(0.5)],
        },
        [q0, q1],
    );
    let err = b.build().expect_err("empty opaque name must be rejected");
    assert_eq!(err, IrError::EmptyOpaqueName);
}

/// I6 variant a: an opaque gate with no operands and no params is rejected.
#[test]
fn opaque_no_operands_no_params() {
    let mut b = CircuitBuilder::new("i6-a");
    b.gate(
        GateKind::Opaque {
            name: "custom".into(),
            params: vec![],
        },
        [],
    );
    let err = b.build().expect_err("no operands must be rejected");
    assert!(matches!(err, IrError::EmptyOpaqueOperands { .. }));
}

/// I6 variant b: an opaque gate with params but no operands is rejected.
#[test]
fn opaque_no_operands_with_params() {
    let mut b = CircuitBuilder::new("i6-b");
    b.gate(
        GateKind::Opaque {
            name: "iswap".into(),
            params: vec![Angle::new(0.1)],
        },
        [],
    );
    let err = b.build().expect_err("no operands must be rejected");
    assert!(matches!(err, IrError::EmptyOpaqueOperands { .. }));
}

/// I7 variant a: a NaN rotation angle is rejected.
#[test]
fn angle_nan_rejected() {
    let mut b = CircuitBuilder::new("i7-a");
    let q0 = b.alloc_qubit();
    b.rx(Angle::new(f64::NAN), q0);
    let err = b.build().expect_err("NaN angle must be rejected");
    assert!(matches!(err, IrError::NonFiniteAngle { .. }));
}

/// I7 variant b: a positive-infinity rotation angle is rejected.
#[test]
fn angle_positive_infinity_rejected() {
    let mut b = CircuitBuilder::new("i7-b");
    let q0 = b.alloc_qubit();
    b.rz(Angle::new(f64::INFINITY), q0);
    let err = b.build().expect_err("inf angle must be rejected");
    assert!(matches!(err, IrError::NonFiniteAngle { .. }));
}

/// I7 variant c: a non-finite component inside a U gate is rejected.
#[test]
fn angle_u_gate_negative_infinity_rejected() {
    let mut b = CircuitBuilder::new("i7-c");
    let q0 = b.alloc_qubit();
    b.gate(
        GateKind::U {
            theta: Angle::new(f64::NEG_INFINITY),
            phi: Angle::new(0.0),
            lambda: Angle::new(0.0),
        },
        [q0],
    );
    let err = b
        .build()
        .expect_err("non-finite U component must be rejected");
    assert!(matches!(err, IrError::NonFiniteAngle { .. }));
}

/// I1 positive control: the maximum in-range qubit index is accepted.
#[test]
fn valid_boundary_max_qubit_index_accepted() {
    let mut b = CircuitBuilder::new("i1-ok");
    let qs = b.alloc_qubits(2);
    b.x(qs[1]); // highest valid index
    let circuit = b.build().expect("max valid qubit index must build");
    assert_eq!(circuit.num_qubits(), 2);
}

/// I2 positive control: the maximum in-range clbit index is accepted.
#[test]
fn valid_boundary_max_clbit_index_accepted() {
    let mut b = CircuitBuilder::new("i2-ok");
    let q0 = b.alloc_qubit();
    let cs = b.alloc_clbits(2);
    b.measure(q0, cs[1]); // highest valid index
    let circuit = b.build().expect("max valid clbit index must build");
    assert_eq!(circuit.num_clbits(), 2);
}
