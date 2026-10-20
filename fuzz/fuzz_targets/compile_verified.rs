//! A semantic fuzz target: any program the OpenQASM frontend accepts, compiled
//! for a constrained target with in-compiler verification on, must never be
//! found NotEquivalent. A `SemanticMismatch` here is a miscompilation, and it
//! panics so libFuzzer records the input.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oqci_fuzz::compile_verified(data));
