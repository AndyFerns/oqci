//! `oqci-demo`: the `oqci` CLI with the demonstration extensions registered.
//!
//! ```text
//! cargo run -p oqci-plugin-demo -- lower circuit.qtoy --backend demo-ring --verify
//! cargo run -p oqci-plugin-demo -- passes      # lists hzh-to-x
//! cargo run -p oqci-plugin-demo -- backends    # lists demo-ring
//! ```

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut registry = oqci::extension::Registry::default();
    if let Err(error) = oqci_plugin_demo::register(&mut registry) {
        eprintln!("error: {error}");
        return ExitCode::FAILURE;
    }
    match oqci::cli::run_with(registry) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
