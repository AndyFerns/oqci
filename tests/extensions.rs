//! The extension registry (§18): what it accepts, what it refuses, and that
//! an empty one is exactly the built-in compiler.

use std::sync::Arc;

use oqci::compile::{CompileError, CompilerConfig, compile, compile_with_frontend};
use oqci::extension::{ExtensionError, ExtensionOrigin, FrontendPlugin, PassPlacement, Registry};
use oqci::frontend::FrontendError;
use oqci::ir::{Circuit, CircuitBuilder};
use oqci::pass::{Pass, PassContext, PassError, PassManager, PassOutput};

fn origin() -> ExtensionOrigin {
    ExtensionOrigin::new("ext-test", "1.2.3")
}

/// A pass that does nothing, under a configurable id.
struct Noop(&'static str);

impl Pass for Noop {
    fn id(&self) -> &'static str {
        self.0
    }
    fn description(&self) -> &'static str {
        "does nothing"
    }
    fn run(&self, circuit: &Circuit, _: &PassContext<'_>) -> Result<PassOutput, PassError> {
        Ok(PassOutput::unchanged(circuit.clone()))
    }
}

/// A frontend reading `n` as "n qubits, an H on each".
struct Counting(&'static str);

impl FrontendPlugin for Counting {
    fn id(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "n qubits, an H on each"
    }
    fn file_extensions(&self) -> &[&str] {
        &["count"]
    }
    fn parse(&self, source: &str, name: &str) -> Result<Circuit, FrontendError> {
        let n: u32 = source
            .trim()
            .parse()
            .map_err(|_| FrontendError::syntax("expected a qubit count", 1, 1))?;
        let mut b = CircuitBuilder::new(name);
        for q in b.alloc_qubits(n) {
            b.h(q);
        }
        Ok(b.build()?)
    }
}

#[test]
fn an_empty_registry_is_the_built_in_compiler() {
    let registry = Registry::default();
    assert!(registry.is_empty());
    assert_eq!(
        registry.pass_ids(),
        PassManager::default_pipeline()
            .pass_ids()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        registry.backend_ids(),
        oqci::backend::all()
            .iter()
            .map(|b| b.id().to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(registry.frontend_ids(), vec!["openqasm3", "cudaq"]);

    let artifacts = compile(
        "qubit[2] q; h q[0]; cx q[0], q[1];",
        &CompilerConfig {
            backend: Some("simulator-nisq".into()),
            ..CompilerConfig::default()
        },
    )
    .unwrap();
    assert!(artifacts.extensions.is_empty());
    assert!(artifacts.provenance.unwrap().extensions.is_empty());
}

#[test]
fn pass_placement_follows_the_anchor() {
    let mut registry = Registry::default();
    registry
        .register_pass(origin(), PassPlacement::default(), Arc::new(Noop("a")))
        .unwrap()
        .register_pass(
            origin(),
            PassPlacement::AfterLast("canonicalize".into()),
            Arc::new(Noop("b")),
        )
        .unwrap()
        .register_pass(
            origin(),
            PassPlacement::BeforeFirst("canonicalize".into()),
            Arc::new(Noop("c")),
        )
        .unwrap()
        .register_pass(origin(), PassPlacement::End, Arc::new(Noop("d")))
        .unwrap();
    let ids = registry.pass_ids();
    let position = |id: &str| ids.iter().position(|x| x == id).unwrap();
    assert_eq!(ids.first().map(String::as_str), Some("c"));
    assert_eq!(ids.last().map(String::as_str), Some("d"));
    assert_eq!(position("a") + 1, position("schedule"));
    assert_eq!(
        position("b"),
        ids.iter().rposition(|x| x == "canonicalize").unwrap() + 1
    );
}

#[test]
fn registrations_that_would_be_ambiguous_are_refused() {
    let mut registry = Registry::default();
    // A pass id colliding with a built-in would make --disable hit both.
    let err = registry
        .register_pass(origin(), PassPlacement::End, Arc::new(Noop("canonicalize")))
        .unwrap_err();
    assert!(matches!(
        err,
        ExtensionError::DuplicateId { kind: "pass", .. }
    ));

    let err = registry
        .register_pass(
            origin(),
            PassPlacement::BeforeFirst("no-such-pass".into()),
            Arc::new(Noop("x")),
        )
        .unwrap_err();
    assert!(matches!(err, ExtensionError::UnknownAnchor { .. }));

    let err = registry
        .register_pass(origin(), PassPlacement::End, Arc::new(Noop("")))
        .unwrap_err();
    assert!(matches!(err, ExtensionError::EmptyId { kind: "pass" }));

    let err = registry
        .register_frontend(origin(), Arc::new(Counting("openqasm3")))
        .unwrap_err();
    assert!(matches!(
        err,
        ExtensionError::DuplicateId {
            kind: "frontend",
            ..
        }
    ));
}

#[test]
fn an_extension_frontend_is_selected_by_id_or_by_file_extension() {
    let mut registry = Registry::default();
    registry
        .register_frontend(origin(), Arc::new(Counting("count")))
        .unwrap()
        .register_pass(origin(), PassPlacement::default(), Arc::new(Noop("noop")))
        .unwrap();
    assert_eq!(
        registry
            .frontend_for_path(std::path::Path::new("x.COUNT"))
            .id(),
        "count"
    );
    assert_eq!(
        registry
            .frontend_for_path(std::path::Path::new("x.qasm"))
            .id(),
        "openqasm3"
    );

    let config = CompilerConfig {
        extensions: registry,
        ..CompilerConfig::default()
    };
    let artifacts = compile_with_frontend("3", "three", "count", &config).unwrap();
    assert_eq!(artifacts.source_metrics.num_qubits, 3);
    assert_eq!(artifacts.frontend_id.as_deref(), Some("count"));
    assert_eq!(
        artifacts.extensions,
        vec![
            "ext-test@1.2.3 frontend:count".to_string(),
            "ext-test@1.2.3 pass:noop".to_string()
        ]
    );
    // Its timing is reported as parsing only; no split it did not measure.
    assert!(artifacts.timings.frontend.is_some());
    assert!(artifacts.timings.ir_construction.is_none());
}

#[test]
fn a_disabled_extension_pass_is_not_listed_as_taking_part() {
    let mut registry = Registry::default();
    registry
        .register_pass(origin(), PassPlacement::default(), Arc::new(Noop("noop")))
        .unwrap();
    let config = CompilerConfig {
        extensions: registry,
        passes: oqci::pass::PassSelection::all_except(["noop"]),
        ..CompilerConfig::default()
    };
    let artifacts = compile("qubit[1] q; h q[0];", &config).unwrap();
    assert!(artifacts.extensions.is_empty());
}

#[test]
fn an_unknown_frontend_lists_the_alternatives() {
    let err = compile_with_frontend("", "x", "brainfuck", &CompilerConfig::default()).unwrap_err();
    match err {
        CompileError::UnknownFrontend { available, .. } => {
            assert_eq!(available, "openqasm3, cudaq");
        }
        other => panic!("got {other}"),
    }
}

#[test]
fn timings_split_parsing_from_ir_construction_for_built_in_frontends() {
    let artifacts = compile(
        "qubit[2] q; h q[0]; cx q[0], q[1];",
        &CompilerConfig {
            backend: Some("simulator".into()),
            ..CompilerConfig::default()
        },
    )
    .unwrap();
    let t = artifacts.timings;
    assert!(t.frontend.is_some() && t.ir_construction.is_some());
    assert!(t.lowering.is_some() && t.cost.is_some() && t.preparation.is_some());
    assert!(t.lint.is_none() && t.verification.is_none());
    assert!(t.total >= t.optimization + t.lowering.unwrap());

    // Entering with a built circuit has no frontend time to report.
    let circuit = oqci::frontend::parse_openqasm3("qubit[1] q; h q[0];").unwrap();
    let artifacts = oqci::compile::compile_circuit(circuit, &CompilerConfig::default()).unwrap();
    assert!(artifacts.timings.frontend.is_none());
    assert!(artifacts.timings.ir_construction.is_none());
}
