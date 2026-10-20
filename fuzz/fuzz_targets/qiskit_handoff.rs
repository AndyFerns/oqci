//! The Qiskit translation core accepts any handoff struct the PyO3 layer
//! could build — arbitrary names, operands and parameters — without panicking.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oqci_fuzz::qiskit_handoff(data));
