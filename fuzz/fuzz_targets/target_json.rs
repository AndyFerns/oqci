//! A target description from outside the process (§29: safe parsing of
//! untrusted input) is either loaded — and then compiles a small circuit
//! without panicking — or refused with a typed error.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oqci_fuzz::target_json(data));
