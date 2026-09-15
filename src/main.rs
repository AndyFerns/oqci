//! The `oqci` binary — a thin entry point around [`oqci::cli`].
//!
//! All behaviour lives in the library so that the CLI and any other consumer
//! share one implementation; see `docs/cli.md`.

use std::process::ExitCode;

fn main() -> ExitCode {
    match oqci::cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
