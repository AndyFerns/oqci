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
//! routing and basis decomposition (§8.6–8.8) live in [`crate::lowering`],
//! because Stage D §4 is explicit that target description, target lowering
//! and routing "must not be conflated". That separation is why [`check`]
//! reports problems instead of quietly repairing them, and it is what lets
//! [`check`] serve as lowering's postcondition oracle: a function that
//! repaired what it inspected could not also certify it.
//!
//! One consequence is visible in [`check`]'s behaviour: it reads a circuit's
//! qubit `n` as physical qubit `n`. For an unmapped circuit that is the
//! identity-layout assumption, so a connectivity violation means "this will
//! not run *as written*" rather than "this can never run here". For a circuit
//! that has been through [`crate::lowering::lower`] the operands already
//! *are* physical, and the reading is exact.
//!
//! Decomposition rules are recorded here by identifier only. The data model
//! Stage D §5 describes — source operation, target sequence, parameter
//! transformation, operand mapping, exactness — lives in
//! [`crate::lowering::rules`], where the code that executes it is, and these
//! identifiers are the keys into it.
//!
//! Execution (§9–10) is absent from this layer too, and belongs to the
//! backend contract.

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
pub use topology::{CouplingMode, PhysicalQubit, Topology};
