//! Shared test-only helpers.
//!
//! Each integration-test binary compiles this module separately, so a helper
//! one test file needs is dead code to another. That is expected, not a
//! defect: the alternative is splitting the harness per consumer and letting
//! the copies drift.
#![allow(dead_code)]

pub mod statevector;
