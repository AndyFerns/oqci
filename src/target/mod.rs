//! Target models: describing what a backend accepts, and what it finds
//! expensive.
//!
//! This module is the compiler's answer to "compile *for what?*". Everything
//! before it — the IR, the frontends, the optimization passes — is deliberately
//! target-independent. Everything after it (mapping, routing, decomposition,
//! execution) needs to know which device is being compiled for, and this is
//! where that knowledge lives.
//!
//! It exists as its own layer because of three locked decisions:
//!
//! - **Stage D** — an abstract gate vocabulary must not be rewritten every time
//!   a backend changes. Instead a backend supplies a [`BasisProfile`]
//!   describing its operations, connectivity and constraints, and lowering
//!   consults that.
//! - **Stage C** — backend-specific behaviour belongs behind a target
//!   interface, never as `if IBM … else if Cirq …` branching scattered through
//!   optimization code.
//! - **Stage E** — the *target* defines what is expensive, via a
//!   [`CostModel`]; the optimizer only chooses among transformations.
//!
//! # What this layer does
//!
//! - [`BasisProfile`] — the formal target description (`final-deliverables-spec.md` §11).
//! - [`Topology`] — physical qubits and **directed** couplings
//!   ([`Topology::add_undirected`] declares both directions explicitly).
//! - [`check`] — validates a circuit against a profile, reporting every
//!   [`Violation`] rather than the first.
//! - [`CostModel`] / [`Cost`] — structured, component-preserving cost.
//! - [`builtin`] — two synthetic profiles, neither describing real hardware.
//!
//! # What this layer deliberately does *not* do
//!
//! It **describes** targets; it does not **apply** them. Qubit mapping,
//! routing/SWAP insertion (§8.6–8.7), basis decomposition (§8.8) and execution
//! (§9–10) are all absent. Stage D §4 is explicit that target description,
//! target lowering and routing "must not be conflated", and that separation is
//! why [`check`] reports problems instead of quietly repairing them.
//!
//! One consequence is visible in [`check`]'s behaviour: with no layout step
//! yet, a circuit's logical qubit `n` is checked against physical qubit `n`.
//! A connectivity violation therefore means "this will not run *as written*",
//! not "this can never run here" — routing is what closes that gap.
//!
//! Decomposition rules are likewise recorded by identifier only. The data
//! model Stage D §5 describes (source operation, target sequence, parameter
//! transformation, operand mapping, exactness) lands with the pass that
//! executes it, so it can be designed against a real consumer rather than
//! guessed at now.

pub mod builtin;
pub mod cost;
pub mod legality;
pub mod profile;
pub mod topology;

pub use cost::{Cost, CostModel, WeightedCostModel};
pub use legality::{LegalityReport, Violation, check};
pub use profile::{
    BasisProfile, BasisProfileBuilder, MeasurementSupport, ParameterConstraint, TargetError,
};
pub use topology::{PhysicalQubit, Topology};
