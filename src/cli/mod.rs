//! The `oqci` command-line inspector.
//!
//! A way to see what the compiler actually produced at every stage, for a
//! real program, without writing a test first — and, in `watch` mode, to keep
//! seeing it as you edit.
//!
//! ```text
//! oqci compile  <input.qasm>   # frontend -> QC-IR -> QCO-IR -> QIR
//! oqci optimize <input.qasm>   # the same, with the pass pipeline and a diff
//! oqci analyze  <input.qasm>   # resource metrics only
//! oqci watch    <input.qasm>   # re-run on every save
//! oqci lint     <input.qasm>   # static checks, optionally against a target
//! oqci lower    <input.qasm> --backend ID | --target-file FILE
//! oqci passes                  # list the available passes
//! ```
//!
//! Running an executable is not a command of this binary: execution happens
//! outside the Rust crate, in an execution adapter. `python -m oqci run`
//! compiles through the same library and runs the result on Aer, CUDA-Q or
//! IBM hardware (`docs/cli.md`).
//!
//! # This module contains no compiler logic
//!
//! `final-deliverables-spec.md` §19 requires that the CLI "must not duplicate
//! compiler logic that belongs in the Rust library — it should invoke the
//! same public compiler APIs", and that requirement is structural here rather
//! than aspirational: the `pipeline` module is the only one that calls the
//! compiler, and it does nothing but delegate to
//! [`crate::frontend`], [`crate::ir`], [`crate::pass`] and
//! [`crate::analysis`]. Every metric printed was computed by the same
//! function the pass manager uses for its own bookkeeping, so the tool's view
//! of a circuit and the compiler's cannot drift apart.

mod pipeline;
mod render;
/// Stage-snapshot view types (`PipelineReport` and friends).
///
/// Public so an external crate — e.g. a visualization server — can read the
/// same JSON contract this CLI's `--json` flag prints, through the same
/// builder functions, without duplicating any of it. See
/// `docs/visualization.md`.
pub mod snapshot;
mod watch;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::extension::{ExtensionOrigin, Registry};
use crate::frontend::FrontendError;
use crate::ir::IrError;
use crate::pass::{PassError, PassSelection};
use crate::target::{BasisProfile, builtin};
use pipeline::{Session, Stage};

/// Anything that can stop a CLI command.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CliError {
    /// The input file could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// The path that failed.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The source program was rejected by the frontend.
    #[error(transparent)]
    Frontend(#[from] FrontendError),
    /// An IR operation failed.
    #[error(transparent)]
    Ir(#[from] IrError),
    /// A pass failed.
    #[error(transparent)]
    Pass(#[from] PassError),
    /// JSON serialization failed.
    #[error("cannot serialize report: {0}")]
    Json(#[from] serde_json::Error),
    /// Compilation failed — frontend, passes, lowering or preparation.
    ///
    /// Carried transparently because the orchestrator's errors are already
    /// written for a reader: re-wrapping them would only put a layer of
    /// "compilation failed:" in front of a message that already says what
    /// went wrong and what to do about it.
    #[error(transparent)]
    Compile(#[from] crate::compile::CompileError),
    /// A command-line argument was not usable.
    #[error("{0}")]
    Usage(String),
    /// The filesystem watch could not be established.
    #[error("watch failed: {0}")]
    Watch(String),
}

/// Inspect and optimize quantum circuits with OQCI.
#[derive(Debug, Parser)]
#[command(name = "oqci", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compile a program and show each pipeline stage.
    Compile(CompileArgs),
    /// Optimize a program, showing what each pass did.
    Optimize(OptimizeArgs),
    /// Report a program's resource metrics.
    Analyze(AnalyzeArgs),
    /// Re-run compile or optimize whenever the file changes.
    Watch(WatchArgs),
    /// Lower a program onto a backend, showing layout, routing and decomposition.
    Lower(LowerArgs),
    /// Lower a program and write the executable a backend would run.
    Prepare(PrepareArgs),
    /// Run the lint rules over a program, optionally against a target.
    Lint(LintArgs),
    /// List the backends this build can compile for.
    Backends,
    /// List the registered optimization passes.
    Passes,
    /// List the built-in target profiles.
    Targets,
    /// List the lint rules.
    LintRules,
}

/// Options shared by every command that reads a program.
#[derive(Debug, clap::Args)]
struct InputArgs {
    /// Source file: OpenQASM 3, or a CUDA-Q kernel if it ends in `.py`.
    input: PathBuf,
    /// Read the source with this frontend instead of the one the file
    /// extension implies.
    #[arg(long, value_name = "ID")]
    frontend: Option<String>,
    /// Include lint findings in the report.
    #[arg(long)]
    lint: bool,
    /// Include per-stage compile times. Measurements: never part of a
    /// deterministic comparison.
    #[arg(long)]
    timings: bool,
    /// Check that binding, every pass and lowering preserved the circuit's
    /// semantics, and refuse to finish if one did not.
    #[arg(long)]
    verify: bool,
    /// The widest circuit, in active qubits, that `--verify` will simulate;
    /// wider ones are reported inconclusive.
    #[arg(long, value_name = "N", default_value_t = 8, requires = "verify")]
    verify_max_qubits: u32,
    /// Report a failed verification instead of stopping. For investigating a
    /// failure, never for producing results.
    #[arg(long, requires = "verify")]
    verify_report_only: bool,
    /// Bind a symbolic parameter, e.g. `--bind theta=1.57`. Repeatable.
    #[arg(long = "bind", value_name = "NAME=VALUE")]
    bindings: Vec<String>,
    /// Emit machine-readable JSON instead of text.
    #[arg(long)]
    json: bool,
    /// Check the circuit against a target profile (see `oqci targets`).
    #[arg(long, value_name = "ID")]
    target: Option<String>,
    /// Which QIR profile is acceptable: `any` labels a module `base_profile`
    /// when the circuit qualifies and `custom` otherwise; `base` refuses
    /// instead of emitting `custom`.
    #[arg(long, value_enum, default_value_t = QirProfileArg::Any)]
    qir_profile: QirProfileArg,
    /// How QIR spells pointers: `opaque` (`ptr`, LLVM 17+) or `typed`
    /// (`%Qubit*`, as in the QIR 1.0 specification's examples).
    #[arg(long, value_enum, default_value_t = QirPointersArg::Opaque)]
    qir_pointers: QirPointersArg,
}

/// Which QIR profile a compilation may produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum QirProfileArg {
    /// `base_profile` when the circuit qualifies, `custom` otherwise.
    Any,
    /// `base_profile`, or an error naming why the circuit does not qualify.
    Base,
}

/// How QIR spells pointers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum QirPointersArg {
    /// Opaque pointers, `ptr`.
    Opaque,
    /// Typed pointers, `%Qubit*` / `%Result*`.
    Typed,
}

impl InputArgs {
    fn qir_request(&self) -> crate::compile::QirRequest {
        crate::compile::QirRequest {
            pointers: match self.qir_pointers {
                QirPointersArg::Opaque => crate::ir::PointerStyle::Opaque,
                QirPointersArg::Typed => crate::ir::PointerStyle::Typed,
            },
            profile: match self.qir_profile {
                QirProfileArg::Any => crate::compile::QirProfileRequirement::Any,
                QirProfileArg::Base => crate::compile::QirProfileRequirement::Base,
            },
        }
    }
}

#[derive(Debug, clap::Args)]
struct CompileArgs {
    #[command(flatten)]
    input: InputArgs,
    /// Stages to show. Defaults to all of them.
    #[arg(long, value_delimiter = ',', value_name = "STAGE")]
    emit: Vec<Stage>,
}

#[derive(Debug, clap::Args)]
struct OptimizeArgs {
    #[command(flatten)]
    input: InputArgs,
    /// Stages to show. Defaults to all of them.
    #[arg(long, value_delimiter = ',', value_name = "STAGE")]
    emit: Vec<Stage>,
    /// Run only these passes (see `oqci passes`).
    #[arg(long, value_delimiter = ',', value_name = "ID")]
    passes: Vec<String>,
    /// Run every pass except these — the shape of an ablation study.
    #[arg(long, value_delimiter = ',', value_name = "ID")]
    disable: Vec<String>,
    /// Show a before/after instruction diff.
    #[arg(long)]
    diff: bool,
}

/// Which initial layout to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum LayoutArg {
    /// Logical `n` on physical `n`.
    Trivial,
    /// Seat interacting qubits near each other.
    Dense,
}

impl LayoutArg {
    fn choice(self) -> crate::lowering::LayoutChoice {
        match self {
            LayoutArg::Trivial => crate::lowering::LayoutChoice::Trivial,
            LayoutArg::Dense => crate::lowering::LayoutChoice::Dense,
        }
    }
}

/// How a command names the device it compiles for: a registered backend, or
/// a target description loaded from a file.
#[derive(Debug, clap::Args)]
struct DeviceArgs {
    /// Backend to compile for. See `oqci backends`.
    #[arg(long, value_name = "ID", conflicts_with = "target_file")]
    backend: Option<String>,

    /// Compile for the target described in this JSON file (a serialized
    /// basis profile, e.g. one retrieved from a real device).
    #[arg(long, value_name = "FILE")]
    target_file: Option<PathBuf>,

    /// Which backend kind to load the target file as (`ibm`, `simulator`).
    /// Defaults to the profile's own `backend_id`.
    #[arg(long, value_name = "KIND", requires = "target_file")]
    target_kind: Option<String>,

    /// The id to register the loaded target under. Defaults to the profile's.
    #[arg(long, value_name = "ID", requires = "target_file")]
    target_id: Option<String>,
}

impl DeviceArgs {
    /// Resolves the device to a backend id, loading a target file into the
    /// session's registry when one was given. `None` when neither was given.
    fn resolve(&self, session: &mut Session) -> Result<Option<String>, CliError> {
        if let Some(id) = &self.backend {
            return Ok(Some(id.clone()));
        }
        let Some(path) = &self.target_file else {
            return Ok(None);
        };
        let json = read(path)?;
        let id = session
            .registry
            .register_target_json(
                ExtensionOrigin::target_file(path.display().to_string()),
                &json,
                self.target_kind.as_deref(),
                self.target_id.as_deref(),
            )
            .map_err(|error| CliError::Usage(format!("{}: {error}", path.display())))?;
        Ok(Some(id))
    }
}

/// Options shared by `lower` and `prepare`.
#[derive(Debug, clap::Args)]
struct LoweringArgs {
    #[command(flatten)]
    device: DeviceArgs,

    /// Initial layout strategy.
    #[arg(long, value_enum, default_value_t = LayoutArg::Trivial)]
    layout: LayoutArg,

    /// Skip SWAP insertion. An inspection aid: the result is usually illegal,
    /// and the report says so.
    #[arg(long)]
    no_route: bool,

    /// Skip basis decomposition, leaving abstract gates in place.
    #[arg(long)]
    no_decompose: bool,
}

impl LoweringArgs {
    fn config(&self) -> crate::lowering::LoweringConfig {
        crate::lowering::LoweringConfig {
            layout: self.layout.choice(),
            route: !self.no_route,
            decompose: !self.no_decompose,
        }
    }
}

#[derive(Debug, clap::Args)]
struct LowerArgs {
    #[command(flatten)]
    input: InputArgs,

    #[command(flatten)]
    lowering: LoweringArgs,

    /// Which stages to print.
    #[arg(long, value_enum, value_delimiter = ',', default_values_t = [Stage::QcIr])]
    emit: Vec<Stage>,

    /// Show what lowering changed, as a diff.
    #[arg(long)]
    diff: bool,
}

#[derive(Debug, clap::Args)]
struct PrepareArgs {
    #[command(flatten)]
    input: InputArgs,

    #[command(flatten)]
    lowering: LoweringArgs,

    /// How many times the executable should be run.
    #[arg(long, default_value_t = 1024)]
    shots: u32,

    /// Simulator seed, for a reproducible run.
    ///
    /// Not defaulted: choosing one would be picking an experimental parameter
    /// that the benchmarking protocol owns.
    #[arg(long)]
    seed: Option<u64>,

    /// Write the executable JSON here instead of to stdout.
    #[arg(short = 'o', long, value_name = "FILE")]
    output: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
struct LintArgs {
    #[command(flatten)]
    input: InputArgs,
    #[command(flatten)]
    device: DeviceArgs,
    /// Lint the optimized circuit instead of the parsed one.
    #[arg(long)]
    optimized: bool,
    /// Exit with an error when any finding has at least this severity.
    #[arg(long, value_enum, value_name = "SEVERITY")]
    deny: Option<DenyArg>,
}

/// The severity at which `oqci lint --deny` fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum DenyArg {
    /// Fail on any warning or error.
    Warnings,
    /// Fail only on errors.
    Errors,
}

#[derive(Debug, clap::Args)]
struct AnalyzeArgs {
    #[command(flatten)]
    input: InputArgs,
    /// Analyze the optimized circuit instead of the parsed one.
    #[arg(long)]
    optimized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Mode {
    /// Show each pipeline stage.
    Compile,
    /// Run the pass pipeline too.
    Optimize,
}

#[derive(Debug, clap::Args)]
struct WatchArgs {
    #[command(flatten)]
    input: InputArgs,
    /// What to re-run on each change.
    #[arg(long, value_enum, default_value_t = Mode::Optimize)]
    mode: Mode,
    /// Stages to show. Defaults to all of them.
    #[arg(long, value_delimiter = ',', value_name = "STAGE")]
    emit: Vec<Stage>,
    /// Run only these passes.
    #[arg(long, value_delimiter = ',', value_name = "ID")]
    passes: Vec<String>,
    /// Run every pass except these.
    #[arg(long, value_delimiter = ',', value_name = "ID")]
    disable: Vec<String>,
    /// Show a before/after instruction diff.
    #[arg(long)]
    diff: bool,
}

/// Parses arguments and runs the requested command.
///
/// # Errors
///
/// Returns the [`CliError`] that stopped the command.
pub fn run() -> Result<(), CliError> {
    run_with(Registry::default())
}

/// As [`run`], with extensions available to every command.
///
/// This is how an extension crate ships a CLI that knows its frontends,
/// passes and backends (§18): a `main` that builds a [`Registry`] and calls
/// this. The `oqci` binary itself calls it with no extensions.
///
/// # Errors
///
/// Returns the [`CliError`] that stopped the command.
pub fn run_with(registry: Registry) -> Result<(), CliError> {
    dispatch(Cli::parse(), registry)
}

/// A session for one command: the registry, plus the shared report options.
fn session_for(registry: &Registry, input: &InputArgs) -> Result<Session, CliError> {
    if let Some(id) = &input.frontend
        && registry.frontend(id).is_none()
    {
        return Err(CliError::Usage(format!(
            "unknown frontend `{id}`; available: {}",
            registry.frontend_ids().join(", ")
        )));
    }
    Ok(Session {
        registry: registry.clone(),
        frontend: input.frontend.clone(),
        lint: input.lint,
        timings: input.timings,
        verify: crate::verify::VerifyConfig {
            enabled: input.verify,
            max_qubits: input.verify_max_qubits,
            on_mismatch: if input.verify_report_only {
                crate::verify::OnMismatch::Report
            } else {
                crate::verify::OnMismatch::Fail
            },
            ..crate::verify::VerifyConfig::default()
        },
    })
}

fn dispatch(cli: Cli, registry: Registry) -> Result<(), CliError> {
    match cli.command {
        Command::Compile(args) => {
            let session = session_for(&registry, &args.input)?;
            let input = prepare(&args.input, &args.emit)?;
            let report = pipeline::run_compile_with(
                &args.input.input,
                &input.source,
                &input.bindings,
                &input.emit,
                input.target.as_ref(),
                &args.input.qir_request(),
                &session,
            )?;
            emit_report(&report, args.input.json, false)
        }
        Command::Optimize(args) => {
            let session = session_for(&registry, &args.input)?;
            let input = prepare(&args.input, &args.emit)?;
            let selection = selection_from(&registry, &args.passes, &args.disable)?;
            let report = pipeline::run_optimize_with(
                &args.input.input,
                &input.source,
                &input.bindings,
                &selection,
                &input.emit,
                args.diff,
                input.target.as_ref(),
                &args.input.qir_request(),
                &session,
            )?;
            emit_report(&report, args.input.json, false)
        }
        Command::Analyze(args) => {
            let session = session_for(&registry, &args.input)?;
            let input = prepare(&args.input, &[])?;
            let report = if args.optimized {
                pipeline::run_optimize_with(
                    &args.input.input,
                    &input.source,
                    &input.bindings,
                    &PassSelection::All,
                    &[Stage::QcIr],
                    false,
                    input.target.as_ref(),
                    &args.input.qir_request(),
                    &session,
                )?
            } else {
                pipeline::run_compile_with(
                    &args.input.input,
                    &input.source,
                    &input.bindings,
                    &[Stage::QcIr],
                    input.target.as_ref(),
                    &args.input.qir_request(),
                    &session,
                )?
            };
            emit_report(&report, args.input.json, true)
        }
        Command::Lower(args) => {
            let mut session = session_for(&registry, &args.input)?;
            let backend = require_device(&args.lowering.device, &mut session)?;
            let input = prepare(&args.input, &args.emit)?;
            let report = pipeline::run_lower(
                &args.input.input,
                &input.source,
                &pipeline::LowerRequest {
                    bindings: &input.bindings,
                    backend: &backend,
                    lowering: args.lowering.config(),
                    settings: crate::backend::ExecutionSettings::default(),
                    emit: &input.emit,
                    want_diff: args.diff,
                    prepare: false,
                    qir: args.input.qir_request(),
                },
                &session,
            )?;
            emit_report(&report, args.input.json, false)
        }
        Command::Prepare(args) => {
            let mut session = session_for(&registry, &args.input)?;
            let backend = require_device(&args.lowering.device, &mut session)?;
            let input = prepare(&args.input, &[])?;
            let settings = crate::backend::ExecutionSettings {
                shots: args.shots,
                seed: args.seed,
                memory: false,
            };
            let report = pipeline::run_lower(
                &args.input.input,
                &input.source,
                &pipeline::LowerRequest {
                    bindings: &input.bindings,
                    backend: &backend,
                    lowering: args.lowering.config(),
                    settings,
                    emit: &[],
                    want_diff: false,
                    prepare: true,
                    qir: args.input.qir_request(),
                },
                &session,
            )?;
            write_executable(&report, args.output.as_deref(), args.input.json)
        }
        Command::Lint(args) => run_lint(&registry, &args),
        Command::Backends => {
            list_backends(&registry);
            Ok(())
        }
        Command::Watch(args) => run_watch(&registry, &args),
        Command::Passes => {
            list_passes(&registry);
            Ok(())
        }
        Command::Targets => {
            list_targets();
            Ok(())
        }
        Command::LintRules => {
            list_lint_rules(&registry);
            Ok(())
        }
    }
}

/// `lower` and `prepare` need a device; say so if neither flag named one.
fn require_device(device: &DeviceArgs, session: &mut Session) -> Result<String, CliError> {
    device.resolve(session)?.ok_or_else(|| {
        CliError::Usage("name a device with --backend ID or --target-file FILE".into())
    })
}

fn run_lint(registry: &Registry, args: &LintArgs) -> Result<(), CliError> {
    let mut session = session_for(registry, &args.input)?;
    let input = prepare(&args.input, &[])?;
    // A device's profile when one was named, otherwise `--target`.
    let profile = match args.device.resolve(&mut session)? {
        Some(id) => Some(
            session
                .registry
                .backend(&id)
                .ok_or_else(|| {
                    CliError::Usage(format!(
                        "unknown backend `{id}`; available: {}",
                        session.registry.backend_ids().join(", ")
                    ))
                })?
                .profile()
                .clone(),
        ),
        None => input.target.clone(),
    };
    let report = pipeline::run_lint(
        &args.input.input,
        &input.source,
        &input.bindings,
        args.optimized,
        profile.as_ref(),
        &session,
    )?;
    let text = if args.input.json {
        render::json(&report)?
    } else {
        render::lint(&report)
    };
    println!("{text}");

    let lint = report.lint.as_ref().expect("lint always reports");
    let threshold = match args.deny {
        None => return Ok(()),
        Some(DenyArg::Warnings) => crate::analysis::Severity::Warning,
        Some(DenyArg::Errors) => crate::analysis::Severity::Error,
    };
    match lint.count_at_least(threshold) {
        0 => Ok(()),
        n => Err(CliError::Usage(format!(
            "{n} lint finding(s) at or above `{}`",
            threshold.id()
        ))),
    }
}

/// A command's inputs, read and normalized.
struct Prepared {
    source: String,
    bindings: HashMap<String, f64>,
    emit: Vec<Stage>,
    target: Option<BasisProfile>,
}

/// Reads the source and normalizes the shared options.
fn prepare(input: &InputArgs, emit: &[Stage]) -> Result<Prepared, CliError> {
    Ok(Prepared {
        source: read(&input.input)?,
        bindings: parse_bindings(&input.bindings)?,
        emit: if emit.is_empty() {
            Stage::all()
        } else {
            emit.to_vec()
        },
        target: resolve_target(input.target.as_deref())?,
    })
}

/// Looks up a built-in profile, rejecting an unknown id rather than silently
/// checking against nothing.
fn resolve_target(id: Option<&str>) -> Result<Option<BasisProfile>, CliError> {
    let Some(id) = id else {
        return Ok(None);
    };
    builtin::by_id(id).map(Some).ok_or_else(|| {
        let available: Vec<String> = builtin::all()
            .iter()
            .map(|profile| profile.id().to_string())
            .collect();
        CliError::Usage(format!(
            "unknown target `{id}`; available: {}",
            available.join(", ")
        ))
    })
}

fn read(path: &Path) -> Result<String, CliError> {
    std::fs::read_to_string(path).map_err(|source| CliError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Parses repeated `--bind NAME=VALUE` arguments.
fn parse_bindings(raw: &[String]) -> Result<HashMap<String, f64>, CliError> {
    raw.iter()
        .map(|entry| {
            let (name, value) = entry.split_once('=').ok_or_else(|| {
                CliError::Usage(format!("--bind expects NAME=VALUE, got `{entry}`"))
            })?;
            let value: f64 = value.trim().parse().map_err(|_| {
                CliError::Usage(format!("`{value}` is not a number in `--bind {entry}`"))
            })?;
            Ok((name.trim().to_string(), value))
        })
        .collect()
}

/// Builds a [`PassSelection`] from the `--passes` / `--disable` flags.
fn selection_from(
    registry: &Registry,
    only: &[String],
    except: &[String],
) -> Result<PassSelection, CliError> {
    match (only.is_empty(), except.is_empty()) {
        (true, true) => Ok(PassSelection::All),
        (false, true) => {
            validate_ids(registry, only)?;
            Ok(PassSelection::only(only.iter().cloned()))
        }
        (true, false) => {
            validate_ids(registry, except)?;
            Ok(PassSelection::all_except(except.iter().cloned()))
        }
        (false, false) => Err(CliError::Usage(
            "--passes and --disable cannot be combined; use one or the other".into(),
        )),
    }
}

/// Rejects unknown pass ids rather than silently running a different pipeline
/// than the user asked for.
fn validate_ids(registry: &Registry, ids: &[String]) -> Result<(), CliError> {
    let known = registry.pass_ids();
    for id in ids {
        if !known.contains(id) {
            return Err(CliError::Usage(format!(
                "unknown pass `{id}`; available: {}",
                known.join(", ")
            )));
        }
    }
    Ok(())
}

fn emit_report(
    report: &snapshot::PipelineReport,
    as_json: bool,
    metrics_only: bool,
) -> Result<(), CliError> {
    let text = match (as_json, metrics_only) {
        (true, _) => render::json(report)?,
        (false, true) => render::resource_report(report),
        (false, false) => render::human(report),
    };
    println!("{text}");
    Ok(())
}

fn run_watch(registry: &Registry, args: &WatchArgs) -> Result<(), CliError> {
    let session = session_for(registry, &args.input)?;
    let path = args.input.input.clone();
    let bindings = parse_bindings(&args.input.bindings)?;
    let selection = selection_from(registry, &args.passes, &args.disable)?;
    let emit = if args.emit.is_empty() {
        Stage::all()
    } else {
        args.emit.clone()
    };
    let (mode, diff, as_json) = (args.mode, args.diff, args.input.json);
    let target = resolve_target(args.input.target.as_deref())?;
    let qir = args.input.qir_request();

    watch::watch(&path.clone(), move || {
        // Re-read every time: the file is the input, and it just changed.
        let source = read(&path)?;
        let report = match mode {
            Mode::Compile => pipeline::run_compile_with(
                &path,
                &source,
                &bindings,
                &emit,
                target.as_ref(),
                &qir,
                &session,
            )?,
            Mode::Optimize => pipeline::run_optimize_with(
                &path,
                &source,
                &bindings,
                &selection,
                &emit,
                diff,
                target.as_ref(),
                &qir,
                &session,
            )?,
        };
        if as_json {
            render::json(&report)
        } else {
            Ok(render::human(&report))
        }
    })
}

/// Writes the prepared executable, to a file or to stdout.
fn write_executable(
    report: &snapshot::PipelineReport,
    output: Option<&std::path::Path>,
    json: bool,
) -> Result<(), CliError> {
    let view = report
        .executable
        .as_ref()
        .expect("prepare always produces an executable");
    let body = serde_json::to_string_pretty(&view.executable)?;

    match output {
        Some(path) => {
            std::fs::write(path, &body).map_err(|source| CliError::Io {
                path: path.display().to_string(),
                source,
            })?;
            if !json {
                println!(
                    "wrote {} operation(s) for `{}` to {}",
                    view.executable.ops.len(),
                    view.backend_id,
                    path.display()
                );
            }
            Ok(())
        }
        None => {
            println!("{body}");
            Ok(())
        }
    }
}

/// Lists the backends this build can compile for.
fn list_backends(registry: &Registry) {
    println!("backends:");
    for (id, description) in registry.available_backends() {
        println!("  {id:<20} {description}");
    }
    println!();
    println!("No backend executes in this process.");
    println!("`oqci prepare` writes the artifact; an execution adapter runs it.");
    println!("`--target-file FILE` compiles for a target described in a JSON file.");
}

fn list_lint_rules(registry: &Registry) {
    println!("lint rules:\n");
    for id in registry.linter().rule_ids() {
        println!("  {id}");
    }
    println!("\nrun them with `oqci lint FILE`, optionally against --backend or --target-file.");
}

fn list_passes(registry: &Registry) {
    println!("default pipeline, in order:\n");
    for (position, id) in registry.pass_ids().iter().enumerate() {
        println!("  {}. {id}", position + 1);
    }
    println!(
        "\nuse --passes ID,... to run only some, or --disable ID,... to ablate one.\n\
         `canonicalize` appears twice: it also cleans up after rotation merging."
    );
}

fn list_targets() {
    println!("built-in target profiles:\n");
    for profile in builtin::all() {
        println!("  {}", profile.qualified_id());
        println!("    backend       {}", profile.backend_id());
        println!(
            "    qubits        {} ({} directed coupling(s))",
            profile.qubit_count(),
            profile.topology().edge_count()
        );
        println!(
            "    basis         {}",
            profile.supported_operations().join(", ")
        );
        println!("    cost model    {}", profile.cost_model_id());
        println!();
    }
    println!(
        "use --target ID on `compile`, `optimize` or `analyze` to check a circuit\n\
         against one. All are synthetic: none describes real hardware, and
         `qir-base` describes an output format (the QIR Base Profile)."
    );
    println!();
    println!("`--target ID` checks a circuit against one of these and reports.");
    println!("To compile *for* a device, use `--backend` — see `oqci backends`,");
    println!("which lists one more profile that a backend carries rather than");
    println!("this registry.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_parse_name_value_pairs() {
        let parsed =
            parse_bindings(&["theta=1.5".to_string(), " phi = -0.25 ".to_string()]).unwrap();
        assert_eq!(parsed["theta"], 1.5);
        assert_eq!(parsed["phi"], -0.25);
    }

    #[test]
    fn a_binding_without_an_equals_sign_is_rejected() {
        let error = parse_bindings(&["theta".to_string()]).unwrap_err();
        assert!(matches!(error, CliError::Usage(msg) if msg.contains("NAME=VALUE")));
    }

    #[test]
    fn a_non_numeric_binding_is_rejected() {
        let error = parse_bindings(&["theta=abc".to_string()]).unwrap_err();
        assert!(matches!(error, CliError::Usage(msg) if msg.contains("not a number")));
    }

    #[test]
    fn no_flags_means_every_pass() {
        assert!(matches!(
            selection_from(&Registry::default(), &[], &[]).unwrap(),
            PassSelection::All
        ));
    }

    #[test]
    fn passes_and_disable_cannot_be_combined() {
        let error = selection_from(
            &Registry::default(),
            &["schedule".into()],
            &["canonicalize".into()],
        )
        .unwrap_err();
        assert!(matches!(error, CliError::Usage(msg) if msg.contains("cannot be combined")));
    }

    #[test]
    fn unknown_pass_ids_are_rejected_with_the_available_list() {
        let error =
            selection_from(&Registry::default(), &["no-such-pass".into()], &[]).unwrap_err();
        assert!(
            matches!(error, CliError::Usage(msg) if msg.contains("unknown pass")
                && msg.contains("gate-cancellation"))
        );
    }

    #[test]
    fn known_pass_ids_are_accepted() {
        let registry = Registry::default();
        assert!(selection_from(&registry, &["gate-cancellation".into()], &[]).is_ok());
        assert!(selection_from(&registry, &[], &["rotation-merge".into()]).is_ok());
    }

    #[test]
    fn the_argument_parser_is_well_formed() {
        // clap's own consistency check: catches conflicting flags, duplicate
        // short options and the like at test time rather than at runtime.
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
