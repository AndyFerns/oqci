//! The backend contract: what a device must expose for OQCI to compile for it
//! and run on it.
//!
//! Stage C is the locked decision behind this module: "Backend execution is
//! governed by an explicit OQCI backend contract; IBM gets a dedicated
//! target-lowering stage." Its §6 rules out the alternative in so many words —
//! the generic compiler must not contain `if IBM … else if Cirq …` branching
//! scattered through optimization logic. So backend-specific behaviour lives
//! behind [`Backend`], and the compiler core never names a vendor.
//!
//! # Four stages, kept apart
//!
//! §10 requires the abstraction to distinguish compilation/target preparation,
//! execution, and result retrieval. The trait splits them further, into four
//! methods that can each fail for their own reasons:
//!
//! | Stage | Method | Fails when |
//! |---|---|---|
//! | Target lowering | [`Backend::lower`] | the circuit cannot run on this device at all |
//! | Validation | [`Backend::validate`] | lowering produced something the target rejects |
//! | Preparation | [`Backend::prepare`] | the circuit cannot be turned into an executable artifact |
//! | Execution | [`Backend::execute`] | the job could not be run |
//!
//! Separating preparation from execution is what makes the IBM path honest:
//! everything up to and including a validated executable artifact is pure
//! Rust and fully testable, and only the final submission needs credentials
//! and a network.
//!
//! # What "execution" means here, and what it does not
//!
//! Neither built-in backend executes in this process, and both say so with a
//! typed [`BackendError::ExecutionNotAvailableInProcess`] rather than a
//! `todo!()`, an empty result, or a silent success.
//!
//! - **[`SimulatorBackend`]** prepares artifacts for a real simulator. The
//!   project's non-goals explicitly rule out "a custom quantum simulator
//!   replacing established simulator frameworks", so execution belongs to
//!   Qiskit Aer through the Python adapter, not to a simulator written here.
//! - **[`IbmBackend`]** prepares artifacts for IBM hardware. Submission needs
//!   `qiskit-ibm-runtime` and an account, neither of which exists in this
//!   environment — and §33.4 forbids implementing a vendor API from memory
//!   when the SDK cannot be checked. **No claim of IBM executability is made**
//!   (§33.12); what is claimed is that the artifact has been lowered and
//!   validated against the supplied target description.
//!
//! An error that names the boundary is more useful than a stub: a caller can
//! match on it, and nobody can mistake it for a result.

pub mod executable;
pub mod ibm;
pub mod result;
pub mod simulator;

use crate::ir::Circuit;
use crate::lowering::{Lowered, LoweringConfig, LoweringError};
use crate::target::{BasisProfile, CostModel, LegalityReport};

pub use executable::{Executable, ExecutableOp};
pub use ibm::IbmBackend;
pub use result::{ExecutionResult, ExecutionSettings, Provenance};
pub use simulator::SimulatorBackend;

/// Why a backend could not do what was asked.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum BackendError {
    /// The circuit cannot be lowered onto this backend's target.
    #[error(transparent)]
    Lowering {
        /// What lowering refused.
        #[from]
        source: LoweringError,
    },
    /// A lowered circuit could not be turned into an executable artifact.
    #[error("instruction {index} is not executable: {detail}")]
    NotExecutable {
        /// Program index of the offending operation.
        index: usize,
        /// Why.
        detail: String,
    },
    /// This backend cannot run circuits from inside the compiler process.
    ///
    /// Not a failure and not a stub — a boundary. The artifact is ready; it
    /// simply has to cross into another runtime to be run. Typed so a caller
    /// can route around it, and so it can never be mistaken for a result.
    #[error("backend `{backend}` cannot execute in this process: {reason}")]
    ExecutionNotAvailableInProcess {
        /// Which backend.
        backend: String,
        /// Where execution actually happens.
        reason: String,
    },
    /// The backend's own target description is unusable.
    #[error("backend `{backend}` has an invalid target: {detail}")]
    InvalidTarget {
        /// Which backend.
        backend: String,
        /// What is wrong.
        detail: String,
    },
}

/// What OQCI needs from a device to compile for it.
///
/// Every capability Stage C §3 lists is reachable here: identity through
/// [`Backend::id`], physical resources, connectivity, basis, operation,
/// measurement, reset and parameter constraints through
/// [`Backend::profile`], cost through [`Backend::cost_model`], validation
/// through [`Backend::validate`], target lowering through [`Backend::lower`],
/// the executable representation through [`Backend::prepare`], and execution
/// and its structured result through [`Backend::execute`].
pub trait Backend: Send + Sync {
    /// Stable identifier.
    fn id(&self) -> &str;

    /// One-line description, for `oqci backends`.
    fn description(&self) -> &str;

    /// What this device accepts.
    fn profile(&self) -> &BasisProfile;

    /// What this device finds expensive.
    ///
    /// Supplied by the backend rather than chosen by the optimizer, which is
    /// the whole of Stage E: the target defines cost, and the compiler only
    /// chooses among transformations.
    fn cost_model(&self) -> &dyn CostModel;

    /// Target lowering — §8.6–8.8 and Stage C §4.
    ///
    /// # Errors
    ///
    /// [`BackendError::Lowering`] when the circuit cannot be made legal for
    /// this device.
    fn lower(&self, circuit: &Circuit, config: &LoweringConfig) -> Result<Lowered, BackendError> {
        Ok(crate::lowering::lower(circuit, self.profile(), config)?)
    }

    /// Re-validates a lowered circuit against this target.
    ///
    /// Stage C exit criterion 5 requires target validity to be checked before
    /// submission. `lower` already checks its own output, so this is a second
    /// look at a boundary the caller controls — it also covers a `Lowered`
    /// that arrived from somewhere else, such as a cached artifact or one
    /// lowered for a different profile.
    fn validate(&self, lowered: &Lowered) -> LegalityReport {
        crate::target::check(&lowered.circuit, self.profile())
    }

    /// Turns a validated circuit into this backend's executable
    /// representation.
    ///
    /// # Errors
    ///
    /// [`BackendError::NotExecutable`] if the circuit still contains
    /// something no device can run, and [`BackendError::InvalidTarget`] if
    /// validation fails.
    fn prepare(
        &self,
        lowered: &Lowered,
        settings: &ExecutionSettings,
        provenance: Provenance,
    ) -> Result<Executable, BackendError>;

    /// Runs an executable.
    ///
    /// # Errors
    ///
    /// [`BackendError::ExecutionNotAvailableInProcess`] for every backend
    /// currently shipped — see the module documentation for why that is a
    /// boundary rather than a gap.
    fn execute(&self, executable: &Executable) -> Result<ExecutionResult, BackendError>;
}

/// Checks a lowered circuit before preparing it, the way every backend should.
///
/// Shared so that "validate before you prepare" is one implementation rather
/// than a convention each backend re-implements and one of them eventually
/// forgets.
///
/// # Errors
///
/// [`BackendError::InvalidTarget`] listing every violation found.
pub fn validated(backend: &dyn Backend, lowered: &Lowered) -> Result<(), BackendError> {
    let report = backend.validate(lowered);

    // An unbound parameter is genuinely a violation — no execution API takes
    // a symbol — but it is a *program* problem with a known remedy, not a
    // target-compatibility one. `check` reports it here only because that is
    // where parameter domains live. Refusing it at this point would produce
    // "circuit is not legal for ideal-simulator@1: [UnboundParameter ...]",
    // which tells a user nothing they can act on.
    //
    // `Executable::from_lowered` refuses the same circuit a moment later with
    // the message that actually helps: bind the parameter. So this step lets
    // it through, and the artifact still never gets built.
    let blocking: Vec<&crate::target::Violation> = report
        .violations
        .iter()
        .filter(|violation| !matches!(violation, crate::target::Violation::UnboundParameter { .. }))
        .collect();

    if blocking.is_empty() {
        return Ok(());
    }
    Err(BackendError::InvalidTarget {
        backend: backend.id().to_string(),
        detail: format!(
            "circuit is not legal for `{}`: {}",
            backend.profile().qualified_id(),
            describe(&blocking)
        ),
    })
}

/// A readable one-line summary of what is wrong.
///
/// `{:?}` on a `Vec<Violation>` is unreadable at a terminal, and the whole
/// point of reporting every violation rather than the first is that someone
/// is going to read the list.
fn describe(violations: &[&crate::target::Violation]) -> String {
    use crate::target::Violation;

    violations
        .iter()
        .map(|violation| match violation {
            Violation::UnsupportedOperation { index, mnemonic } => {
                format!("instruction {index}: `{mnemonic}` is not in the basis")
            }
            Violation::QubitOutOfRange {
                index, qubit_count, ..
            } => format!("instruction {index}: qubit beyond the device's {qubit_count}"),
            Violation::ConnectivityViolation {
                index,
                control,
                target,
            } => {
                format!("instruction {index}: {control} and {target} are not coupled in that order")
            }
            Violation::ParameterOutOfRange {
                index,
                mnemonic,
                value,
                min,
                max,
            } => format!(
                "instruction {index}: `{mnemonic}` parameter {value} outside [{min}, {max}]"
            ),
            Violation::UnboundParameter {
                index,
                mnemonic,
                symbol,
            } => format!("instruction {index}: `{mnemonic}` parameter `{symbol}` is unbound"),
            Violation::MeasurementUnsupported { index } => {
                format!("instruction {index}: the device cannot measure")
            }
            Violation::MidCircuitMeasurementUnsupported { index, .. } => {
                format!("instruction {index}: the device cannot measure mid-circuit")
            }
            Violation::ResetUnsupported { index } => {
                format!("instruction {index}: the device cannot reset")
            } // Deliberately no wildcard: a new `Violation` should break this
              // match and make someone write a sentence for it, rather than
              // reaching a user as a `Debug` dump.
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Every backend this build knows about.
///
/// A registry rather than a match on a vendor name, so §6's ban on
/// `if IBM … else if …` holds structurally: selecting a backend is a lookup,
/// and adding one does not touch the compiler core.
#[must_use]
pub fn all() -> Vec<Box<dyn Backend>> {
    vec![
        Box::new(SimulatorBackend::ideal()),
        Box::new(SimulatorBackend::linear_nisq(5)),
        Box::new(IbmBackend::illustrative()),
    ]
}

/// Looks a backend up by identifier.
#[must_use]
pub fn by_id(id: &str) -> Option<Box<dyn Backend>> {
    all().into_iter().find(|backend| backend.id() == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_backend_is_addressable_by_its_own_id() {
        for backend in all() {
            let id = backend.id().to_string();
            assert_eq!(
                by_id(&id).map(|b| b.id().to_string()),
                Some(id.clone()),
                "`{id}` is not resolvable"
            );
        }
        assert!(by_id("no-such-backend").is_none());
    }

    #[test]
    fn every_backend_names_a_resolvable_cost_model() {
        // A backend reporting a cost model that nothing can resolve would
        // make the provenance it records false.
        for backend in all() {
            assert_eq!(
                backend.cost_model().id(),
                backend.profile().cost_model_id(),
                "backend `{}` and its profile disagree about the cost model",
                backend.id()
            );
        }
    }

    #[test]
    fn every_backend_has_a_lowerable_target() {
        // A profile whose rules cannot reach its own basis would fail on the
        // first circuit rather than here.
        for backend in all() {
            assert!(
                crate::lowering::RuleSet::new(backend.profile()).is_ok(),
                "backend `{}` has an unusable rule set",
                backend.id()
            );
        }
    }

    #[test]
    fn no_shipped_backend_claims_to_execute_in_process() {
        // The boundary, asserted rather than assumed. If a backend ever does
        // gain in-process execution, this test is where that gets noticed.
        let mut b = crate::ir::CircuitBuilder::new("t");
        let q = b.alloc_qubits(2);
        let c = b.alloc_clbits(1);
        b.h(q[0]).cx(q[0], q[1]).measure(q[0], c[0]);
        let circuit = b.build().unwrap();

        for backend in all() {
            let lowered = backend.lower(&circuit, &LoweringConfig::default()).unwrap();
            let provenance = simulator::provenance_for(
                backend.as_ref(),
                &circuit,
                &lowered,
                &[],
                &ExecutionSettings::default(),
            );
            let executable = backend
                .prepare(&lowered, &ExecutionSettings::default(), provenance)
                .unwrap();
            let err = backend.execute(&executable).unwrap_err();
            assert!(
                matches!(err, BackendError::ExecutionNotAvailableInProcess { .. }),
                "backend `{}` returned {err}",
                backend.id()
            );
        }
    }

    #[test]
    fn preparing_an_illegal_circuit_is_refused() {
        // `validated` is the shared "check before you prepare" step. Here a
        // circuit lowered for one device is offered to another with a
        // narrower topology.
        let backend = SimulatorBackend::linear_nisq(3);
        let mut b = crate::ir::CircuitBuilder::new("t");
        let q = b.alloc_qubits(3);
        b.cx(q[0], q[2]);
        let circuit = b.build().unwrap();

        // Lowered for an all-to-all device, so no routing happened.
        let lowered = crate::lowering::lower(
            &circuit,
            &crate::target::builtin::ideal_simulator(),
            &LoweringConfig::default(),
        )
        .unwrap();

        let err = validated(&backend, &lowered).unwrap_err();
        assert!(
            matches!(err, BackendError::InvalidTarget { .. }),
            "got {err}"
        );
    }
}
