//! The CUDA-Q frontend never panics on any input.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oqci_fuzz::cudaq(data));
