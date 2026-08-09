//! QIR emitter: extended-intrinsic declarations for non-standard gates,
//! hexadecimal double literals for angles, and Base-Profile module flags.

use std::f64::consts::PI;

use oqci::ir::{Angle, CircuitBuilder, GateKind, emit_qir, qc_to_qco};

/// Build a circuit, run the full pipeline, and return the emitted QIR text.
fn qir_of(build: impl FnOnce(&mut CircuitBuilder)) -> String {
    let mut b = CircuitBuilder::new("qir");
    build(&mut b);
    let circuit = b.build().expect("valid circuit");
    let dag = qc_to_qco(&circuit).expect("acyclic conversion");
    emit_qir(&dag).expect("lowering succeeds")
}

/// Every non-standard gate (id, p, u, cy, swap, ccx, and an Opaque) is emitted
/// as a declared extended `__quantum__qis__*__body` intrinsic.
#[test]
fn extended_intrinsics_declared_for_nonstandard_gates() {
    let qir = qir_of(|b| {
        let q0 = b.alloc_qubit();
        let q1 = b.alloc_qubit();
        let q2 = b.alloc_qubit();
        b.gate(GateKind::I, [q0]);
        b.gate(GateKind::P(Angle::new(0.5)), [q0]);
        b.gate(
            GateKind::U {
                theta: Angle::new(0.1),
                phi: Angle::new(0.2),
                lambda: Angle::new(0.3),
            },
            [q0],
        );
        b.gate(GateKind::Cy, [q0, q1]);
        b.swap(q0, q1);
        b.ccx(q0, q1, q2);
        b.gate(
            GateKind::Opaque {
                name: "iswap".into(),
                params: vec![],
            },
            [q0, q1],
        );
    });

    for body in [
        "declare void @__quantum__qis__id__body",
        "declare void @__quantum__qis__p__body",
        "declare void @__quantum__qis__u__body",
        "declare void @__quantum__qis__cy__body",
        "declare void @__quantum__qis__swap__body",
        "declare void @__quantum__qis__ccx__body",
        "declare void @__quantum__qis__iswap__body",
    ] {
        assert!(qir.contains(body), "missing declaration: {body}");
    }
}

/// Angles are emitted as exact hexadecimal `double` literals, never decimals.
#[test]
fn angles_emitted_as_hex_double_not_decimal() {
    let qir = qir_of(|b| {
        let q0 = b.alloc_qubit();
        b.rz(Angle::new(PI), q0);
        b.gate(GateKind::P(Angle::new(0.5)), [q0]);
    });

    assert!(qir.contains(&format!("double 0x{:016X}", PI.to_bits())));
    assert!(qir.contains(&format!("double 0x{:016X}", 0.5f64.to_bits())));
    // No decimal double literal should appear anywhere.
    assert!(!qir.contains("double 3."));
    assert!(!qir.contains("double 0.5"));
}

/// The Base-Profile module flags are all present on the entry attribute group.
#[test]
fn base_profile_module_flags_present() {
    let qir = qir_of(|b| {
        let q0 = b.alloc_qubit();
        b.h(q0);
    });

    for flag in [
        "\"entry_point\"",
        "\"qir_profiles\"=\"base_profile\"",
        "\"required_num_qubits\"",
        "\"required_num_results\"",
    ] {
        assert!(qir.contains(flag), "missing module flag: {flag}");
    }
}

/// required_num_qubits / required_num_results reflect the declared registers.
#[test]
fn required_counts_reflect_registers() {
    let qir = qir_of(|b| {
        let qs = b.alloc_qubits(3);
        let cs = b.alloc_clbits(2);
        b.h(qs[0]).measure(qs[0], cs[0]).measure(qs[1], cs[1]);
    });
    assert!(qir.contains("\"required_num_qubits\"=\"3\""));
    assert!(qir.contains("\"required_num_results\"=\"2\""));
}
