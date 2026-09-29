//! QIR emitter: extended-intrinsic declarations for non-standard gates,
//! hexadecimal double literals for angles, and the QIR 1.0 entry-point
//! attributes and module flags.

use std::f64::consts::PI;

use oqci::ir::{CircuitBuilder, GateKind, Param, emit_qir, qc_to_qco};

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
        b.gate(GateKind::P(Param::concrete(0.5)), [q0]);
        b.gate(
            GateKind::U {
                theta: Param::concrete(0.1),
                phi: Param::concrete(0.2),
                lambda: Param::concrete(0.3),
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
        b.rz(Param::concrete(PI), q0);
        b.gate(GateKind::P(Param::concrete(0.5)), [q0]);
    });

    assert!(qir.contains(&format!("double 0x{:016X}", PI.to_bits())));
    assert!(qir.contains(&format!("double 0x{:016X}", 0.5f64.to_bits())));
    // No decimal double literal should appear anywhere.
    assert!(!qir.contains("double 3."));
    assert!(!qir.contains("double 0.5"));
}

/// The QIR 1.0 entry-point attributes and module flags are present, and the
/// public emitter labels the module `custom`: only the orchestrator, after
/// checking the `qir-base` profile, may claim `base_profile`.
#[test]
fn qir_1_0_attributes_and_module_flags_present() {
    let qir = qir_of(|b| {
        let q0 = b.alloc_qubit();
        b.h(q0);
    });

    for attribute in [
        "\"entry_point\"",
        "\"output_labeling_schema\"=\"oqci_clbit\"",
        "\"qir_profiles\"=\"custom\"",
        "\"required_num_qubits\"=\"1\"",
        "\"required_num_results\"=\"0\"",
        "!llvm.module.flags = !{!0, !1, !2, !3}",
        "!\"qir_major_version\", i32 1}",
        "!\"qir_minor_version\", i32 0}",
        "!\"dynamic_qubit_management\", i1 false}",
        "!\"dynamic_result_management\", i1 false}",
    ] {
        assert!(qir.contains(attribute), "missing: {attribute}");
    }
    assert!(!qir.contains("base_profile"));
}

/// required_num_qubits / required_num_results count what the entry point
/// uses (highest index + 1), not the declared register sizes: a lowered
/// circuit's register is as wide as its device, and the Base Profile asks for
/// "the number of qubits used by the entry point".
#[test]
fn required_counts_reflect_what_the_entry_point_uses() {
    let qir = qir_of(|b| {
        let qs = b.alloc_qubits(3);
        let cs = b.alloc_clbits(2);
        b.h(qs[0]).measure(qs[0], cs[0]).measure(qs[1], cs[1]);
    });
    assert!(qir.contains("\"required_num_qubits\"=\"2\""));
    assert!(qir.contains("\"required_num_results\"=\"2\""));
}

/// An `Opaque` name LLVM cannot spell unquoted is emitted as a quoted global,
/// consistently in the declaration and the call, with `"` and `\` escaped.
#[test]
fn opaque_names_outside_llvm_identifier_syntax_are_quoted() {
    let opaque = |name: &str| GateKind::Opaque {
        name: name.into(),
        params: vec![],
    };
    let qir = qir_of(|b| {
        let q0 = b.alloc_qubit();
        b.gate(opaque("my gate"), [q0]);
        b.gate(opaque("say\"hi\\"), [q0]);
        b.gate(opaque("kebab-name.v2"), [q0]);
    });

    assert!(qir.contains("declare void @\"__quantum__qis__my gate__body\"(ptr)"));
    assert!(qir.contains("call void @\"__quantum__qis__my gate__body\"("));
    assert!(qir.contains(r#"@"__quantum__qis__say\22hi\5C__body""#));
    // Hyphens and dots are legal unquoted, so this one needs no quoting.
    assert!(qir.contains("declare void @__quantum__qis__kebab-name.v2__body(ptr)"));
    assert!(!qir.contains("@__quantum__qis__my gate"));
}
