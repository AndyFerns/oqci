//! QIR 1.0 conformance of every module OQCI emits.
//!
//! Every example program is compiled for every way QIR can come out of the
//! compiler — target-independent, and for each built-in backend — in both
//! pointer styles, and the canonical QIR artifact is checked against the
//! structural rules of the QIR 1.0 Base Profile:
//! `qir-alliance/qir-spec`, tag `1.0`, commit
//! `e872897d107516d889718b5b6f5fee909dd64238`,
//! `specification/profiles/Base_Profile.md`.
//!
//! **What this is and is not.** These checks are written by OQCI against its
//! own reading of the specification, so they are *not independent evidence*
//! of conformance. The independent checks are external: when
//! `OQCI_QIR_CORPUS_DIR` is set, this test also writes every module it
//! checked to that directory, and CI feeds the directory to PyQIR 0.12.5 and
//! to `llvm-as` (`scripts/validate_qir.py`, `.github/workflows/ci.yml`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use oqci::compile::{
    CompilerConfig, Frontend, QirArtifact, QirRequest, Stop, available_backends, compile_named,
};
use oqci::ir::{PointerStyle, QirProfile};

fn examples() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> =
        std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("examples"))
            .expect("examples directory")
            .map(|entry| entry.expect("directory entry").path())
            .filter(|path| {
                matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("qasm" | "py")
                )
            })
            .collect();
    paths.sort();
    paths
}

/// Every (example, backend, pointer style), with its canonical QIR artifact.
///
/// QIR needs concrete angles, so each free parameter of a parameterized
/// example is bound to a fixed test value. The value is a fixture for this
/// structural test only — it is not a benchmark parameter (Stage G).
fn corpus() -> Vec<(String, QirArtifact)> {
    const TEST_ANGLE: f64 = 0.5;

    let mut targets: Vec<Option<String>> = vec![None];
    targets.extend(available_backends().into_iter().map(|(id, _)| Some(id)));

    let mut modules = Vec::new();
    for path in examples() {
        let source = std::fs::read_to_string(&path).expect("example source");
        let stem = path.file_stem().unwrap().to_str().unwrap().to_string();
        let frontend = Frontend::for_path(&path);
        let bindings: HashMap<String, f64> = frontend
            .parse(&source, &stem)
            .unwrap_or_else(|e| panic!("{stem}: {e}"))
            .parameters()
            .into_iter()
            .map(|name| (name, TEST_ANGLE))
            .collect();
        for backend in &targets {
            for pointers in [PointerStyle::Opaque, PointerStyle::Typed] {
                let config = CompilerConfig {
                    frontend,
                    bindings: bindings.clone(),
                    backend: backend.clone(),
                    stop: Stop::Lowered,
                    qir: QirRequest {
                        pointers,
                        ..QirRequest::default()
                    },
                    ..CompilerConfig::default()
                };
                let artifacts = compile_named(&source, &stem, &config)
                    .unwrap_or_else(|e| panic!("{stem} for {backend:?}: {e}"));
                let qir = artifacts
                    .qir
                    .unwrap_or_else(|| panic!("{stem} for {backend:?}: no QIR"));
                let id = format!(
                    "{stem}__{}__{}",
                    backend.as_deref().unwrap_or("none"),
                    pointers.id()
                );
                modules.push((id, qir));
            }
        }
    }
    modules
}

/// The text of one basic block, from its label to the next blank line.
fn block<'a>(text: &'a str, label: &str) -> Option<&'a str> {
    let start = text.find(&format!("\n{label}:\n"))? + label.len() + 3;
    let end = text[start..].find("\n\n").map_or(text.len(), |n| start + n);
    Some(&text[start..end])
}

fn calls(block: &str) -> Vec<&str> {
    block
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("call "))
        .collect()
}

/// Rules every module follows, whatever its label.
fn check_skeleton(id: &str, qir: &QirArtifact) {
    let text = &qir.text;
    let has = |needle: &str| assert!(text.contains(needle), "{id}: missing `{needle}`");

    // Entry point: no parameters, returns an i64 exit code, 0 on success.
    assert!(
        text.lines()
            .any(|l| l.starts_with("define i64 @") && l.ends_with("() #0 {")),
        "{id}: entry point must take no parameters and return i64"
    );
    has("  ret i64 0\n");

    // The entry block initializes the execution environment.
    let entry = block(text, "entry").unwrap_or_else(|| panic!("{id}: no entry block"));
    assert!(
        calls(entry)[0].starts_with("call void @__quantum__rt__initialize("),
        "{id}: entry block must initialize"
    );

    // Entry-point attributes, with a value on output_labeling_schema.
    for attribute in [
        "\"entry_point\"",
        "\"output_labeling_schema\"=\"oqci_clbit\"",
        "\"required_num_qubits\"=\"",
        "\"required_num_results\"=\"",
    ] {
        has(attribute);
    }
    has(&format!(
        "\"qir_profiles\"=\"{}\"",
        qir.profile.attribute_value()
    ));

    // The four required module flags.
    has("!llvm.module.flags = !{!0, !1, !2, !3}");
    has("!0 = !{i32 1, !\"qir_major_version\", i32 1}");
    has("!1 = !{i32 7, !\"qir_minor_version\", i32 0}");
    has("!2 = !{i32 1, !\"dynamic_qubit_management\", i1 false}");
    has("!3 = !{i32 1, !\"dynamic_result_management\", i1 false}");

    // Measurement is irreversible and writes a writeonly result.
    if text.contains("@__quantum__qis__mz__body(") {
        assert!(
            text.lines()
                .any(|l| l.starts_with("declare void @__quantum__qis__mz__body(")
                    && l.contains(" writeonly)")
                    && l.ends_with(" #1")),
            "{id}: mz must be declared irreversible with a writeonly result"
        );
        has("attributes #1 = { \"irreversible\" }");
    }

    // Every recorded result has its own non-null label.
    let output = block(text, "output").unwrap_or_else(|| panic!("{id}: no output block"));
    let records: Vec<&str> = calls(output);
    let mut labels: Vec<String> = records
        .iter()
        .map(|call| {
            // The label is the one global the call's arguments reference:
            // `ptr @0` (opaque) or `… [3 x i8]* @0, i32 0, i32 0` (typed).
            let args = &call[call.find('(').expect("call has arguments")..];
            assert!(
                !args.contains("null"),
                "{id}: output label must be non-null: {call}"
            );
            let at = args
                .find('@')
                .unwrap_or_else(|| panic!("{id}: unlabeled output: {call}"));
            args[at..]
                .chars()
                .take_while(|c| *c == '@' || c.is_ascii_digit())
                .collect()
        })
        .collect();
    let count = labels.len();
    labels.sort_unstable();
    labels.dedup();
    assert_eq!(labels.len(), count, "{id}: output labels must be unique");

    // Pointer style is uniform.
    match qir.pointers {
        PointerStyle::Opaque => assert!(
            !text.contains("%Qubit*"),
            "{id}: typed pointer in opaque module"
        ),
        PointerStyle::Typed => assert!(
            !text.contains("(ptr"),
            "{id}: opaque pointer in typed module"
        ),
    }
}

/// The Base Profile's additional structural rules.
fn check_base(id: &str, qir: &QirArtifact) {
    let text = &qir.text;
    let body = block(text, "body").unwrap_or_else(|| panic!("{id}: no body block"));
    let measurements =
        block(text, "measurements").unwrap_or_else(|| panic!("{id}: no measurements block"));

    // The blocks come in the order the Base Profile prescribes.
    let order: Vec<usize> = ["\nentry:", "\nbody:", "\nmeasurements:", "\noutput:"]
        .iter()
        .map(|label| text.find(label).unwrap())
        .collect();
    assert!(
        order.windows(2).all(|w| w[0] < w[1]),
        "{id}: blocks out of order"
    );

    // Body: only non-irreversible QIS calls. Measurements: only mz.
    for call in calls(body) {
        assert!(
            call.contains("@__quantum__qis__"),
            "{id}: non-QIS call in body: {call}"
        );
        assert!(
            !call.contains("__mz__"),
            "{id}: measurement in body: {call}"
        );
    }
    for call in calls(measurements) {
        assert!(
            call.starts_with("call void @__quantum__qis__mz__body("),
            "{id}: non-measurement in measurements block: {call}"
        );
    }
    assert!(
        !text.contains("__quantum__qis__reset__body"),
        "{id}: reset in a Base module"
    );
    assert!(
        qir.base_violations.is_empty(),
        "{id}: Base module with violations"
    );
}

#[test]
fn every_emitted_module_follows_the_qir_1_0_structure() {
    let corpus = corpus();
    assert!(!corpus.is_empty());

    let mut base = 0;
    for (id, qir) in &corpus {
        check_skeleton(id, qir);
        if qir.profile == QirProfile::Base {
            check_base(id, qir);
            base += 1;
        } else {
            assert!(
                !qir.base_violations.is_empty(),
                "{id}: custom without a reason"
            );
        }
    }
    // Lowering onto qir-base must make every concrete example Base.
    for (id, qir) in &corpus {
        if id.contains("__qir-base__") {
            assert_eq!(
                qir.profile,
                QirProfile::Base,
                "{id}: qir-base output is not Base"
            );
        }
    }
    assert!(base > 0);

    if let Some(dir) = std::env::var_os("OQCI_QIR_CORPUS_DIR") {
        let dir = PathBuf::from(dir);
        for style in ["opaque", "typed"] {
            std::fs::create_dir_all(dir.join(style)).expect("corpus directory");
        }
        for (id, qir) in &corpus {
            let path = dir.join(qir.pointers.id()).join(format!("{id}.ll"));
            std::fs::write(&path, &qir.text).expect("write corpus module");
        }
        // Opaque gates whose names need quoting are not in any example, but
        // they are exactly what an external parser should see once.
        for pointers in [PointerStyle::Opaque, PointerStyle::Typed] {
            let path = dir
                .join(pointers.id())
                .join(format!("opaque_names__none__{}.ll", pointers.id()));
            std::fs::write(&path, opaque_names_module(pointers)).expect("write corpus module");
        }
        eprintln!(
            "wrote {} QIR module(s) to {}",
            corpus.len() + 2,
            dir.display()
        );
    }
}

/// A custom module whose `Opaque` gate names LLVM cannot spell unquoted.
fn opaque_names_module(pointers: PointerStyle) -> String {
    use oqci::ir::{CircuitBuilder, GateKind, QirOptions, emit_qir_with, qc_to_qco};

    let mut b = CircuitBuilder::new("opaque_names");
    let q = b.alloc_qubits(2);
    let c = b.alloc_clbits(1);
    for name in ["my gate", "say\"hi\\", "kebab-name.v2"] {
        b.gate(
            GateKind::Opaque {
                name: name.into(),
                params: vec![],
            },
            [q[0], q[1]],
        );
    }
    b.measure(q[0], c[0]);
    let dag = qc_to_qco(&b.build().unwrap()).unwrap();
    emit_qir_with(&dag, &QirOptions { pointers }).unwrap()
}
