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
//! oqci passes                  # list the available passes
//! ```
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
mod snapshot;
mod watch;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::frontend::FrontendError;
use crate::ir::IrError;
use crate::pass::{PassError, PassManager, PassSelection};
use crate::target::{BasisProfile, builtin};
use pipeline::Stage;

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
    /// List the registered optimization passes.
    Passes,
    /// List the built-in target profiles.
    Targets,
}

/// Options shared by every command that reads a program.
#[derive(Debug, clap::Args)]
struct InputArgs {
    /// OpenQASM 3 source file.
    input: PathBuf,
    /// Bind a symbolic parameter, e.g. `--bind theta=1.57`. Repeatable.
    #[arg(long = "bind", value_name = "NAME=VALUE")]
    bindings: Vec<String>,
    /// Emit machine-readable JSON instead of text.
    #[arg(long)]
    json: bool,
    /// Check the circuit against a target profile (see `oqci targets`).
    #[arg(long, value_name = "ID")]
    target: Option<String>,
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
    dispatch(Cli::parse())
}

fn dispatch(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Compile(args) => {
            let input = prepare(&args.input, &args.emit)?;
            let report = pipeline::run_compile(
                &args.input.input,
                &input.source,
                &input.bindings,
                &input.emit,
                input.target.as_ref(),
            )?;
            emit_report(&report, args.input.json, false)
        }
        Command::Optimize(args) => {
            let input = prepare(&args.input, &args.emit)?;
            let selection = selection_from(&args.passes, &args.disable)?;
            let report = pipeline::run_optimize(
                &args.input.input,
                &input.source,
                &input.bindings,
                &selection,
                &input.emit,
                args.diff,
                input.target.as_ref(),
            )?;
            emit_report(&report, args.input.json, false)
        }
        Command::Analyze(args) => {
            let input = prepare(&args.input, &[])?;
            let report = if args.optimized {
                pipeline::run_optimize(
                    &args.input.input,
                    &input.source,
                    &input.bindings,
                    &PassSelection::All,
                    &[Stage::QcIr],
                    false,
                    input.target.as_ref(),
                )?
            } else {
                pipeline::run_compile(
                    &args.input.input,
                    &input.source,
                    &input.bindings,
                    &[Stage::QcIr],
                    input.target.as_ref(),
                )?
            };
            emit_report(&report, args.input.json, true)
        }
        Command::Watch(args) => run_watch(&args),
        Command::Passes => {
            list_passes();
            Ok(())
        }
        Command::Targets => {
            list_targets();
            Ok(())
        }
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
fn selection_from(only: &[String], except: &[String]) -> Result<PassSelection, CliError> {
    match (only.is_empty(), except.is_empty()) {
        (true, true) => Ok(PassSelection::All),
        (false, true) => {
            validate_ids(only)?;
            Ok(PassSelection::only(only.iter().cloned()))
        }
        (true, false) => {
            validate_ids(except)?;
            Ok(PassSelection::all_except(except.iter().cloned()))
        }
        (false, false) => Err(CliError::Usage(
            "--passes and --disable cannot be combined; use one or the other".into(),
        )),
    }
}

/// Rejects unknown pass ids rather than silently running a different pipeline
/// than the user asked for.
fn validate_ids(ids: &[String]) -> Result<(), CliError> {
    let known = PassManager::default_pipeline().pass_ids();
    for id in ids {
        if !known.contains(&id.as_str()) {
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

fn run_watch(args: &WatchArgs) -> Result<(), CliError> {
    let path = args.input.input.clone();
    let bindings = parse_bindings(&args.input.bindings)?;
    let selection = selection_from(&args.passes, &args.disable)?;
    let emit = if args.emit.is_empty() {
        Stage::all()
    } else {
        args.emit.clone()
    };
    let (mode, diff, as_json) = (args.mode, args.diff, args.input.json);
    let target = resolve_target(args.input.target.as_deref())?;

    watch::watch(&path.clone(), move || {
        // Re-read every time: the file is the input, and it just changed.
        let source = read(&path)?;
        let report = match mode {
            Mode::Compile => {
                pipeline::run_compile(&path, &source, &bindings, &emit, target.as_ref())?
            }
            Mode::Optimize => pipeline::run_optimize(
                &path,
                &source,
                &bindings,
                &selection,
                &emit,
                diff,
                target.as_ref(),
            )?,
        };
        if as_json {
            render::json(&report)
        } else {
            Ok(render::human(&report))
        }
    })
}

fn list_passes() {
    let manager = PassManager::default_pipeline();
    println!("default pipeline, in order:\n");
    for (position, id) in manager.pass_ids().iter().enumerate() {
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
         against one. Both profiles are synthetic: neither describes real hardware."
    );
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
            selection_from(&[], &[]).unwrap(),
            PassSelection::All
        ));
    }

    #[test]
    fn passes_and_disable_cannot_be_combined() {
        let error = selection_from(&["schedule".into()], &["canonicalize".into()]).unwrap_err();
        assert!(matches!(error, CliError::Usage(msg) if msg.contains("cannot be combined")));
    }

    #[test]
    fn unknown_pass_ids_are_rejected_with_the_available_list() {
        let error = selection_from(&["no-such-pass".into()], &[]).unwrap_err();
        assert!(
            matches!(error, CliError::Usage(msg) if msg.contains("unknown pass")
                && msg.contains("gate-cancellation"))
        );
    }

    #[test]
    fn known_pass_ids_are_accepted() {
        assert!(selection_from(&["gate-cancellation".into()], &[]).is_ok());
        assert!(selection_from(&[], &["rotation-merge".into()]).is_ok());
    }

    #[test]
    fn the_argument_parser_is_well_formed() {
        // clap's own consistency check: catches conflicting flags, duplicate
        // short options and the like at test time rather than at runtime.
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
