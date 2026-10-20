//! The OpenQASM 3 frontend never panics on any input: it returns a circuit
//! (which, by construction, has passed I1-I8) or a typed error.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oqci_fuzz::openqasm(data));
