//! QIR emission preserves what the circuit computes.
//!
//! The structural QIR tests (`qir_conformance.rs`, `qir_emit.rs`) check a
//! module's skeleton: declarations, metadata, counts, the measurement block.
//! None of them reads a gate call's operands back, so an emitter that wrote a
//! CX as `cnot(target, control)` passed all of them (the Stage G mutant
//! `qir-cx-operands-swapped` survived every checker). This file decodes the
//! emitted entry block back into a circuit — by parsing the text only, sharing
//! no code with the emitter — and requires it to have the same operator as the
//! source, column by column over every basis state, and the same
//! qubit-to-result measurement map.

mod support;

use std::collections::HashMap;
use std::path::Path;

use oqci::compile::Frontend;
use oqci::ir::{
    Circuit, CircuitBuilder, ClbitId, GateKind, Instruction, Param, PointerStyle, QirOptions,
    QubitId, bind_parameters, emit_qir_with, qc_to_qco,
};
use proptest::prelude::*;
use support::statevector::{overlap_trace, simulate_from_basis_state};

const MAX_QUBITS: u32 = 8;

/// What the entry block of a QIR module does, decoded from its text.
struct Decoded {
    gates: Vec<(GateKind, Vec<QubitId>)>,
    measures: Vec<(u32, u32)>,
}

/// `<ty> inttoptr (i64 N to <ty>)` (optionally `writeonly`) -> N.
fn pointer_index(arg: &str) -> u32 {
    let start = arg
        .find("(i64 ")
        .unwrap_or_else(|| panic!("not an inttoptr operand: {arg}"))
        + 5;
    let end = start + arg[start..].find(' ').expect("`to` after the index");
    arg[start..end].parse().expect("integer index")
}

/// `double 0x<16 hex digits>` -> the f64 with exactly those bits.
fn hex_double(arg: &str) -> f64 {
    let hex = arg
        .strip_prefix("double 0x")
        .unwrap_or_else(|| panic!("not a hex double: {arg}"));
    f64::from_bits(u64::from_str_radix(hex, 16).expect("16 hex digits"))
}

fn gate_from(name: &str, params: &[f64]) -> GateKind {
    let p = |i: usize| Param::concrete(params[i]);
    match name {
        "__quantum__qis__cnot__body" => GateKind::Cx,
        "__quantum__qis__s__adj" => GateKind::Sdg,
        "__quantum__qis__t__adj" => GateKind::Tdg,
        other => {
            let mnemonic = other
                .strip_prefix("__quantum__qis__")
                .and_then(|m| m.strip_suffix("__body"))
                .unwrap_or_else(|| panic!("unexpected intrinsic {other}"));
            match mnemonic {
                "id" => GateKind::I,
                "x" => GateKind::X,
                "y" => GateKind::Y,
                "z" => GateKind::Z,
                "h" => GateKind::H,
                "s" => GateKind::S,
                "t" => GateKind::T,
                "sx" => GateKind::SX,
                "sxdg" => GateKind::SXdg,
                "rx" => GateKind::Rx(p(0)),
                "ry" => GateKind::Ry(p(0)),
                "rz" => GateKind::Rz(p(0)),
                "p" => GateKind::P(p(0)),
                "u" => GateKind::U {
                    theta: p(0),
                    phi: p(1),
                    lambda: p(2),
                },
                "cy" => GateKind::Cy,
                "cz" => GateKind::Cz,
                "swap" => GateKind::Swap,
                "ccx" => GateKind::Ccx,
                unknown => panic!("no decoding for intrinsic `{unknown}`"),
            }
        }
    }
}

fn decode(qir: &str) -> Decoded {
    let mut decoded = Decoded {
        gates: Vec::new(),
        measures: Vec::new(),
    };
    for line in qir.lines() {
        let Some(call) = line.trim().strip_prefix("call void @__quantum__qis__") else {
            continue;
        };
        let open = call.find('(').expect("argument list");
        let name = format!("__quantum__qis__{}", &call[..open]);
        let args: Vec<&str> = call[open + 1..call.len() - 1].split(", ").collect();
        match name.as_str() {
            "__quantum__qis__mz__body" => {
                decoded
                    .measures
                    .push((pointer_index(args[0]), pointer_index(args[1])));
            }
            "__quantum__qis__reset__body" => panic!("reset is outside this comparison"),
            _ => {
                let params: Vec<f64> = args
                    .iter()
                    .filter(|a| a.starts_with("double "))
                    .map(|a| hex_double(a))
                    .collect();
                let qubits: Vec<QubitId> = args
                    .iter()
                    .filter(|a| !a.starts_with("double "))
                    .map(|a| QubitId(pointer_index(a)))
                    .collect();
                decoded.gates.push((gate_from(&name, &params), qubits));
            }
        }
    }
    decoded
}

/// The source's gates as a unitary circuit, and its measurement map.
fn split(circuit: &Circuit) -> (Circuit, Vec<(u32, u32)>) {
    let mut b = CircuitBuilder::new(circuit.name());
    b.alloc_qubits(circuit.num_qubits());
    let mut measures = Vec::new();
    for instruction in circuit.instructions() {
        match instruction {
            Instruction::Gate { kind, qubits } => {
                b.gate(kind.clone(), qubits.clone());
            }
            Instruction::Measure { qubit, target } => measures.push((qubit.0, target.0)),
            other => panic!("unexpected instruction in a QIR test circuit: {other:?}"),
        }
    }
    (b.build().expect("valid unitary part"), measures)
}

fn operator(circuit: &Circuit) -> Vec<Vec<num_complex::Complex64>> {
    let n = circuit.num_qubits();
    (0..1u32 << n)
        .map(|column| {
            let set: Vec<u32> = (0..n).filter(|q| column >> q & 1 == 1).collect();
            simulate_from_basis_state(circuit, &set, MAX_QUBITS)
        })
        .collect()
}

/// Emits `circuit` in both pointer styles and checks each module against it.
fn assert_qir_preserves(label: &str, circuit: &Circuit) {
    let (unitary, measures) = split(circuit);
    let expected = operator(&unitary);
    let dim = expected.len() as f64;
    let dag = qc_to_qco(circuit).expect("acyclic");
    for pointers in [PointerStyle::Opaque, PointerStyle::Typed] {
        let qir = emit_qir_with(&dag, &QirOptions { pointers }).expect("emits");
        let decoded = decode(&qir);
        let mut b = CircuitBuilder::new("decoded");
        b.alloc_qubits(circuit.num_qubits());
        for (kind, qubits) in decoded.gates {
            b.gate(kind, qubits);
        }
        let actual = operator(&b.build().expect("decoded circuit is valid"));
        // Equal up to one global phase iff |tr(A†B)| = 2^n.
        let overlap = overlap_trace(&expected, &actual).norm();
        assert!(
            (overlap - dim).abs() < 1e-9 * dim,
            "{label} ({pointers:?}): the QIR computes a different operator \
             (|tr(A†B)| = {overlap}, expected {dim})\n{qir}"
        );
        assert_eq!(
            decoded.measures, measures,
            "{label} ({pointers:?}): measurements write different results"
        );
    }
}

#[test]
fn every_gate_kind_survives_emission_with_its_operands_in_order() {
    // Every multi-qubit gate is applied with its operands in descending order
    // as well as ascending, on a non-trivial state, so that a reversal of any
    // operand list changes the operator.
    let mut b = CircuitBuilder::new("every_gate");
    b.alloc_qubits(3);
    b.alloc_clbits(3);
    let (q0, q1, q2) = (QubitId(0), QubitId(1), QubitId(2));
    b.h(q0).ry(0.3, q1).rx(1.1, q2);
    for kind in [
        GateKind::I,
        GateKind::X,
        GateKind::Y,
        GateKind::Z,
        GateKind::H,
        GateKind::S,
        GateKind::Sdg,
        GateKind::T,
        GateKind::Tdg,
        GateKind::SX,
        GateKind::SXdg,
        GateKind::Rx(Param::concrete(0.7)),
        GateKind::Ry(Param::concrete(-0.4)),
        GateKind::Rz(Param::concrete(2.2)),
        GateKind::P(Param::concrete(0.9)),
        GateKind::U {
            theta: Param::concrete(0.5),
            phi: Param::concrete(1.3),
            lambda: Param::concrete(-0.8),
        },
    ] {
        b.gate(kind, [q1]);
    }
    for kind in [GateKind::Cx, GateKind::Cy, GateKind::Cz, GateKind::Swap] {
        b.gate(kind.clone(), [q0, q1]);
        b.gate(kind.clone(), [q2, q0]);
        b.ry(0.6, q0);
    }
    b.gate(GateKind::Ccx, [q2, q0, q1]);
    b.gate(GateKind::Ccx, [q0, q1, q2]);
    b.measure(q0, ClbitId(2))
        .measure(q1, ClbitId(0))
        .measure(q2, ClbitId(1));
    assert_qir_preserves("every_gate", &b.build().unwrap());
}

#[test]
fn every_example_survives_emission() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut checked = 0;
    for entry in std::fs::read_dir(dir).expect("examples") {
        let path = entry.expect("entry").path();
        if !matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("qasm" | "py")
        ) {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let source = std::fs::read_to_string(&path).expect("source");
        let circuit = Frontend::for_path(&path)
            .parse(&source, &stem)
            .unwrap_or_else(|e| panic!("{stem}: {e}"));
        // QIR needs concrete angles: bind each free parameter to a fixture
        // value (not a benchmark parameter).
        let bindings: HashMap<String, f64> = circuit
            .parameters()
            .into_iter()
            .map(|name| (name, 0.5))
            .collect();
        let circuit = bind_parameters(&circuit, &bindings).expect("binds");
        if circuit.num_qubits() > MAX_QUBITS {
            continue;
        }
        assert_qir_preserves(&stem, &circuit);
        checked += 1;
    }
    assert!(checked >= 3, "only {checked} examples were checked");
}

const ANGLES: &[f64] = &[0.0, 0.25, -1.3, std::f64::consts::FRAC_PI_2, 2.9];

fn random_gate() -> impl Strategy<Value = (GateKind, usize)> {
    let angle = prop::sample::select(ANGLES).prop_map(Param::concrete);
    prop_oneof![
        prop::sample::select(vec![
            GateKind::I,
            GateKind::X,
            GateKind::Y,
            GateKind::Z,
            GateKind::H,
            GateKind::S,
            GateKind::Sdg,
            GateKind::T,
            GateKind::Tdg,
            GateKind::SX,
            GateKind::SXdg,
        ])
        .prop_map(|k| (k, 1)),
        angle.clone().prop_map(|a| (GateKind::Rx(a), 1)),
        angle.clone().prop_map(|a| (GateKind::Ry(a), 1)),
        angle.clone().prop_map(|a| (GateKind::Rz(a), 1)),
        angle.clone().prop_map(|a| (GateKind::P(a), 1)),
        (angle.clone(), angle.clone(), angle)
            .prop_map(|(theta, phi, lambda)| (GateKind::U { theta, phi, lambda }, 1)),
        prop::sample::select(vec![
            GateKind::Cx,
            GateKind::Cy,
            GateKind::Cz,
            GateKind::Swap
        ])
        .prop_map(|k| (k, 2)),
        Just((GateKind::Ccx, 3)),
    ]
}

fn random_circuit() -> impl Strategy<Value = Circuit> {
    (
        1u32..=3,
        prop::collection::vec((random_gate(), any::<prop::sample::Index>()), 0..16),
        any::<bool>(),
    )
        .prop_map(|(n, gates, measured)| {
            let mut b = CircuitBuilder::new("random");
            b.alloc_qubits(n);
            b.alloc_clbits(n);
            for ((kind, arity), pick) in gates {
                if arity > n as usize {
                    continue;
                }
                // A random arrangement of `arity` distinct qubits, in any order.
                let mut qubits: Vec<u32> = (0..n).collect();
                let start = pick.index(qubits.len());
                qubits.rotate_left(start);
                if pick.index(2) == 1 {
                    qubits.reverse();
                }
                let operands: Vec<QubitId> = qubits[..arity].iter().map(|&q| QubitId(q)).collect();
                b.gate(kind, operands);
            }
            if measured {
                for q in 0..n {
                    b.measure(QubitId(q), ClbitId(n - 1 - q));
                }
            }
            b.build().expect("valid by construction")
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn random_circuits_survive_emission(circuit in random_circuit()) {
        assert_qir_preserves("random", &circuit);
    }
}
