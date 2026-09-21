//! End-to-end tests for the `oqci` binary.
//!
//! These run the actual compiled executable against the checked-in
//! `examples/*.qasm` files, so they cover the parts unit tests cannot: that
//! argument parsing, file reading, exit codes and the rendered output all work
//! together. `watch` is excluded — it is a blocking loop by design, and its
//! event-filtering logic is unit-tested in `src/cli/watch.rs`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Path to the binary under test, as cargo built it for this run.
fn binary() -> PathBuf {
    // target/debug/deps/cli-<hash> -> target/debug/oqci
    let mut path = std::env::current_exe().expect("test binary path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join(format!("oqci{}", std::env::consts::EXE_SUFFIX))
}

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(name)
}

fn run(args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .output()
        .expect("the oqci binary should be runnable")
}

fn stdout_of(args: &[&str]) -> String {
    let output = run(args);
    assert!(
        output.status.success(),
        "`oqci {}` failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("output is UTF-8")
}

fn json_of(args: &[&str]) -> serde_json::Value {
    serde_json::from_str(&stdout_of(args)).expect("--json output should parse")
}

// --- compile ----------------------------------------------------------------

#[test]
fn compile_shows_every_stage() {
    let text = stdout_of(&["compile", example("bell.qasm").to_str().unwrap()]);
    assert!(text.contains("-- qc-ir --"));
    assert!(text.contains("-- qco-ir --"));
    assert!(text.contains("-- qir --"));
    assert!(text.contains("h %q0"));
    assert!(text.contains("@__quantum__qis__cnot__body"));
}

#[test]
fn compile_honours_the_emit_selection() {
    let text = stdout_of(&[
        "compile",
        example("bell.qasm").to_str().unwrap(),
        "--emit",
        "qir",
    ]);
    assert!(text.contains("-- qir --"));
    assert!(!text.contains("-- qc-ir --"));
}

#[test]
fn compile_emits_valid_json() {
    let value = json_of(&["compile", example("bell.qasm").to_str().unwrap(), "--json"]);

    assert_eq!(value["frontend"], "openqasm3");
    assert_eq!(value["circuit_name"], "bell");
    assert_eq!(value["stages"][0]["stage"], "qc-ir");
    assert_eq!(value["stages"][0]["instructions"][0]["gate"], "h");
    assert_eq!(value["stages"][0]["metrics"]["num_qubits"], 2);
    assert_eq!(value["stages"][1]["graph"]["depth"], 3);
    assert!(
        value["stages"][2]["qir"]
            .as_str()
            .unwrap()
            .contains("define void")
    );
}

// --- optimize ---------------------------------------------------------------

#[test]
fn optimize_reports_each_pass_and_the_diff() {
    let text = stdout_of(&[
        "optimize",
        example("ghz3.qasm").to_str().unwrap(),
        "--diff",
        "--emit",
        "qc-ir",
    ]);

    assert!(text.contains("-- passes --"));
    assert!(text.contains("gate-cancellation"));
    assert!(text.contains("cancelled 1 inverse pair(s)"));
    assert!(text.contains("-- diff --"));
    assert!(
        text.contains("- x %q2"),
        "the cancelled pair shows as removed"
    );
    assert!(text.contains("-- optimized --"));
}

#[test]
fn optimize_can_ablate_a_pass() {
    let path = example("ghz3.qasm");
    let optimized = stdout_of(&["optimize", path.to_str().unwrap(), "--emit", "qc-ir"]);
    let ablated = stdout_of(&[
        "optimize",
        path.to_str().unwrap(),
        "--disable",
        "gate-cancellation",
        "--emit",
        "qc-ir",
    ]);

    assert!(optimized.contains("-- optimized -- 3 qubit(s), 3 clbit(s), 7 op(s)"));
    assert!(
        ablated.contains("-- optimized -- 3 qubit(s), 3 clbit(s), 9 op(s)"),
        "with cancellation disabled the pair survives"
    );
    assert!(ablated.contains("[skip] gate-cancellation"));
}

#[test]
fn optimize_json_carries_pass_records() {
    let value = json_of(&[
        "optimize",
        example("ghz3.qasm").to_str().unwrap(),
        "--json",
        "--diff",
        "--emit",
        "qc-ir",
    ]);

    let passes = value["passes"].as_array().expect("pass records");
    assert_eq!(passes.len(), 5);
    let cancellation = passes
        .iter()
        .find(|p| p["id"] == "gate-cancellation")
        .expect("the cancellation record");
    assert_eq!(cancellation["changed"], true);
    assert_eq!(cancellation["before"]["op_count"], 9);
    assert_eq!(cancellation["after"]["op_count"], 7);
    assert!(
        value["diff"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["marker"] == "-")
    );
}

// --- analyze ----------------------------------------------------------------

#[test]
fn analyze_reports_metrics() {
    let text = stdout_of(&["analyze", example("bell.qasm").to_str().unwrap()]);
    assert!(text.contains("qubits              2"));
    assert!(text.contains("operations          4"));
    assert!(text.contains("depth               3"));
}

#[test]
fn analyze_optimized_measures_the_optimized_circuit() {
    let path = example("ghz3.qasm");
    let parsed = stdout_of(&["analyze", path.to_str().unwrap()]);
    let optimized = stdout_of(&["analyze", path.to_str().unwrap(), "--optimized"]);

    assert!(parsed.contains("operations          9"));
    assert!(optimized.contains("operations          7"));
}

// --- parameters -------------------------------------------------------------

#[test]
fn an_unbound_circuit_reports_rather_than_failing() {
    let output = run(&["compile", example("parameterized.qasm").to_str().unwrap()]);
    assert!(
        output.status.success(),
        "unbound parameters are not an error"
    );

    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("unbound parameter(s): phi, theta"));
    assert!(text.contains("unavailable:"));
}

#[test]
fn binding_parameters_enables_qir_emission() {
    let text = stdout_of(&[
        "compile",
        example("parameterized.qasm").to_str().unwrap(),
        "--bind",
        "theta=1.5708",
        "--bind",
        "phi=0.7854",
        "--emit",
        "qir",
    ]);
    assert!(text.contains("@__quantum__qis__ry__body"));
    assert!(!text.contains("unavailable"));
}

// --- passes -----------------------------------------------------------------

#[test]
fn passes_lists_the_default_pipeline_in_order() {
    let text = stdout_of(&["passes"]);
    assert!(text.contains("1. canonicalize"));
    assert!(text.contains("2. gate-cancellation"));
    assert!(text.contains("3. rotation-merge"));
    assert!(text.contains("5. schedule"));
}

// --- targets ----------------------------------------------------------------

#[test]
fn targets_lists_the_builtin_profiles() {
    let text = stdout_of(&["targets"]);
    assert!(text.contains("ideal-simulator@1"));
    assert!(text.contains("linear-nisq@1"));
    assert!(
        text.contains("synthetic"),
        "the listing must not imply these are real devices"
    );
}

#[test]
fn a_target_reports_legality_and_cost() {
    let text = stdout_of(&[
        "compile",
        example("bell.qasm").to_str().unwrap(),
        "--target",
        "ideal-simulator",
        "--emit",
        "qc-ir",
    ]);
    assert!(text.contains("-- target -- ideal-simulator@1"));
    assert!(text.contains("legality: OK"));
    assert!(text.contains("scalar score"));
    assert!(
        text.contains("from: depth_weight"),
        "a scalar must arrive with the weights behind it"
    );
}

#[test]
fn a_restricted_target_names_the_offending_gate() {
    let text = stdout_of(&[
        "compile",
        example("bell.qasm").to_str().unwrap(),
        "--target",
        "linear-nisq",
        "--emit",
        "qc-ir",
    ]);
    assert!(text.contains("violation(s)"));
    assert!(text.contains("`h` is not in the target's basis set"));
}

#[test]
fn target_json_carries_the_full_report() {
    let value = json_of(&[
        "compile",
        example("bell.qasm").to_str().unwrap(),
        "--target",
        "linear-nisq",
        "--emit",
        "qc-ir",
        "--json",
    ]);

    let target = &value["target"];
    assert_eq!(target["profile"], "linear-nisq@1");
    assert_eq!(target["backend_id"], "generic-nisq");
    assert_eq!(target["legal"], false);
    assert_eq!(target["violations"][0]["kind"], "unsupported_operation");
    assert_eq!(target["violations"][0]["mnemonic"], "h");
    assert_eq!(target["cost"]["non_native_gate_count"], 1);
    assert!(
        target["cost_model_configuration"]["two_qubit_weight"].is_string(),
        "the weights behind the scalar must be machine-readable too"
    );
}

#[test]
fn analyze_accepts_a_target_too() {
    let text = stdout_of(&[
        "analyze",
        example("bell.qasm").to_str().unwrap(),
        "--target",
        "ideal-simulator",
    ]);
    assert!(text.contains("-- target --"));
}

#[test]
fn optimize_checks_the_optimized_circuit_against_the_target() {
    // The optimized circuit is what would actually be submitted.
    let value = json_of(&[
        "optimize",
        example("ghz3.qasm").to_str().unwrap(),
        "--target",
        "ideal-simulator",
        "--emit",
        "qc-ir",
        "--json",
    ]);
    assert_eq!(value["target"]["cost"]["total_gate_count"], 7);
}

#[test]
fn no_target_flag_means_no_target_section() {
    let value = json_of(&[
        "compile",
        example("bell.qasm").to_str().unwrap(),
        "--emit",
        "qc-ir",
        "--json",
    ]);
    assert!(value.get("target").is_none());
}

// --- failure paths ----------------------------------------------------------

#[test]
fn an_unknown_target_is_rejected_and_lists_the_valid_ones() {
    let output = run(&[
        "compile",
        example("bell.qasm").to_str().unwrap(),
        "--target",
        "no-such-device",
    ]);
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown target"));
    assert!(stderr.contains("ideal-simulator"));
}

#[test]
fn a_missing_file_fails_with_a_clear_message() {
    let output = run(&["compile", "definitely/not/here.qasm"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot read"));
}

#[test]
fn an_unknown_pass_is_rejected_and_lists_the_valid_ones() {
    let output = run(&[
        "optimize",
        example("bell.qasm").to_str().unwrap(),
        "--passes",
        "no-such-pass",
    ]);
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown pass"));
    assert!(stderr.contains("gate-cancellation"));
}

#[test]
fn a_malformed_binding_is_rejected() {
    let output = run(&[
        "compile",
        example("parameterized.qasm").to_str().unwrap(),
        "--bind",
        "theta",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("NAME=VALUE"));
}

#[test]
fn a_syntax_error_reports_its_position() {
    let path = std::env::temp_dir().join("oqci_cli_syntax_error.qasm");
    std::fs::write(&path, "qubit[2] q;\nh q[0]\n").expect("write the temp file");

    let output = run(&["compile", path.to_str().unwrap()]);
    let _ = std::fs::remove_file(&path);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("line"), "got: {stderr}");
}

#[test]
fn an_unsupported_construct_names_itself() {
    let path = std::env::temp_dir().join("oqci_cli_unsupported.qasm");
    std::fs::write(&path, "qubit[1] q;\nbit[1] c;\nif (c == 1) { x q[0]; }\n")
        .expect("write the temp file");

    let output = run(&["compile", path.to_str().unwrap()]);
    let _ = std::fs::remove_file(&path);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("control flow"));
}

// --- stability --------------------------------------------------------------

#[test]
fn compile_output_is_stable_across_runs() {
    // Determinism is a project-wide requirement; if rendering or traversal
    // ever became order-dependent, this is where it would show.
    let path = example("ghz3.qasm");
    let args = ["compile", path.to_str().unwrap()];
    assert_eq!(stdout_of(&args), stdout_of(&args));
}

#[test]
fn every_example_compiles() {
    for name in ["bell.qasm", "ghz3.qasm", "parameterized.qasm"] {
        let output = run(&["compile", example(name).to_str().unwrap()]);
        assert!(
            output.status.success(),
            "{name} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

// --- lowering and backends --------------------------------------------------

#[test]
fn backends_lists_what_can_be_compiled_for() {
    let out = stdout_of(&["backends"]);
    assert!(out.contains("simulator"));
    assert!(out.contains("ibm-illustrative"));
    // The listing must not leave anyone thinking a circuit will run here.
    assert!(out.contains("No backend executes in this process"));
}

#[test]
fn lower_reports_the_whole_schedule() {
    let out = stdout_of(&[
        "lower",
        example("ghz3.qasm").to_str().unwrap(),
        "--backend",
        "simulator-nisq",
    ]);
    for step in [
        "arity-reduction",
        "layout",
        "routing",
        "basis-decomposition",
        "orientation-repair",
        "single-qubit-cleanup",
        "verify",
    ] {
        assert!(out.contains(step), "`{step}` missing from:\n{out}");
    }
    assert!(out.contains("legal for this target"));
}

#[test]
fn lower_emits_json_matching_the_schema() {
    let out = stdout_of(&[
        "lower",
        example("bell.qasm").to_str().unwrap(),
        "--backend",
        "simulator-nisq",
        "--json",
    ]);
    let json: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    let lowering = &json["lowering"];
    assert_eq!(lowering["backend"], "simulator-nisq");
    assert_eq!(lowering["profile"], "linear-nisq@1");
    assert_eq!(lowering["legal"], true);
    assert!(lowering["initial_layout"].is_array());
    assert!(lowering["steps"].as_array().unwrap().len() >= 7);
}

#[test]
fn lower_can_skip_routing_and_says_the_result_is_illegal() {
    // `--no-route` is an inspection aid, not a compilation mode. The report
    // must not present an unroutable circuit as ready to run.
    let far = temp_qasm("far.qasm", "qubit[3] q; h q[0]; cx q[0], q[2];");
    let out = stdout_of(&[
        "lower",
        far.to_str().unwrap(),
        "--backend",
        "simulator-nisq",
        "--no-route",
    ]);
    assert!(out.contains("0 swap(s) inserted"));
    assert!(out.contains("NOT legal"), "got:\n{out}");
}

#[test]
fn a_dense_layout_can_be_requested() {
    let far = temp_qasm("dense.qasm", "qubit[4] q; cx q[0], q[3]; cx q[0], q[3];");
    let out = stdout_of(&[
        "lower",
        far.to_str().unwrap(),
        "--backend",
        "simulator-nisq",
        "--layout",
        "dense",
    ]);
    assert!(out.contains("dense layout"), "got:\n{out}");
}

#[test]
fn an_unknown_backend_is_rejected_with_the_alternatives() {
    let output = run(&[
        "lower",
        example("bell.qasm").to_str().unwrap(),
        "--backend",
        "no-such-machine",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("simulator"), "got: {stderr}");
}

#[test]
fn a_circuit_the_device_cannot_run_fails_with_a_reason() {
    // Wider than the device. The exit code matters as much as the message:
    // a script must be able to tell that nothing usable was produced.
    let wide = temp_qasm("wide.qasm", "qubit[9] q; h q[0]; cx q[0], q[8];");
    let output = run(&[
        "lower",
        wide.to_str().unwrap(),
        "--backend",
        "simulator-nisq",
    ]);
    assert!(!output.status.success());
}

#[test]
fn prepare_writes_a_replayable_executable() {
    let out = stdout_of(&[
        "prepare",
        example("bell.qasm").to_str().unwrap(),
        "--backend",
        "simulator-nisq",
        "--shots",
        "512",
        "--seed",
        "7",
    ]);
    let executable: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");

    assert_eq!(executable["backend_id"], "simulator-nisq");
    assert_eq!(executable["settings"]["shots"], 512);
    assert_eq!(executable["settings"]["seed"], 7);

    // Every operation must be one the target actually supports.
    for op in executable["ops"].as_array().unwrap() {
        let name = op["op"].as_str().unwrap();
        assert!(
            ["rz", "sx", "x", "cx", "measure"].contains(&name),
            "`{name}` is outside the linear-nisq basis"
        );
    }
}

#[test]
fn prepare_records_the_provenance_of_what_it_built() {
    let out = stdout_of(&[
        "prepare",
        example("bell.qasm").to_str().unwrap(),
        "--backend",
        "simulator-nisq",
    ]);
    let executable: serde_json::Value = serde_json::from_str(&out).unwrap();
    let provenance = &executable["provenance"];

    assert_eq!(provenance["backend_id"], "simulator-nisq");
    assert_eq!(provenance["profile_id"], "linear-nisq@1");
    assert_eq!(provenance["compiler_version"], env!("CARGO_PKG_VERSION"));
    assert!(
        provenance["git_commit"]
            .as_str()
            .is_some_and(|c| !c.is_empty())
    );
    assert!(
        !provenance["cost_model_configuration"]
            .as_object()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn prepare_can_write_to_a_file() {
    let out_path = std::env::temp_dir().join("oqci-prepared.json");
    let _ = std::fs::remove_file(&out_path);

    let output = run(&[
        "prepare",
        example("bell.qasm").to_str().unwrap(),
        "--backend",
        "simulator",
        "-o",
        out_path.to_str().unwrap(),
    ]);
    assert!(output.status.success());

    let body = std::fs::read_to_string(&out_path).expect("the file was written");
    let executable: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(executable["backend_id"], "simulator");
    let _ = std::fs::remove_file(&out_path);
}

#[test]
fn preparing_an_unbound_circuit_refuses_rather_than_guessing_a_value() {
    // `parameterized.qasm` has a free parameter. No execution API accepts a
    // symbol, and substituting one here would run a different circuit from
    // the one that was written.
    let output = run(&[
        "prepare",
        example("parameterized.qasm").to_str().unwrap(),
        "--backend",
        "simulator",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("bind"),
        "the error should say what to do: {stderr}"
    );
}

#[test]
fn binding_the_parameter_makes_the_same_circuit_preparable() {
    let output = run(&[
        "prepare",
        example("parameterized.qasm").to_str().unwrap(),
        "--backend",
        "simulator",
        "--bind",
        "theta=0.5",
        "--bind",
        "phi=1.25",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn lower_output_is_stable_across_runs() {
    let args = [
        "lower",
        "examples/ghz3.qasm",
        "--backend",
        "ibm-illustrative",
        "--layout",
        "dense",
    ];
    assert_eq!(stdout_of(&args), stdout_of(&args));
}

/// Writes a scratch program and returns its path.
fn temp_qasm(name: &str, body: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("oqci-cli-{name}"));
    std::fs::write(&path, body).expect("scratch file");
    path
}
