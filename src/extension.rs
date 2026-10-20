//! The extension boundary: adding frontends, passes and backends without
//! modifying the compiler core.
//!
//! `final-deliverables-spec.md` §18 asks that "new components can be added
//! without modifying the compiler core": a source language through a defined
//! adapter contract (§18.1), a pass through a stable interface registered
//! with the pass manager (§18.2), and a backend that supplies its own target
//! profile, lowering, execution interface and cost model (§18.3).
//!
//! # Process-local and source-level, deliberately
//!
//! §18.4 allows a dynamic mechanism such as `libloading` only once a
//! safety, versioning and ABI contract is specified, and forbids claiming
//! binary compatibility across Rust compiler versions. None is specified, so
//! this is the alternative §18.4 names: a *source-level* plugin API. An
//! extension is an ordinary crate that depends on `oqci`, implements the
//! public traits — [`FrontendPlugin`], [`Pass`], [`Backend`] — and registers
//! them on a [`Registry`]. A compilation sees them through
//! [`CompilerConfig::extensions`](crate::compile::CompilerConfig::extensions).
//! The shipped `oqci` binary cannot load an extension; a downstream crate
//! builds its own binary with [`crate::cli::run_with`].
//!
//! # What a registry holds
//!
//! Only the extras. The built-in frontends, backends and default pipeline
//! stay where they are, so `Registry::default()` is exactly the compiler
//! without extensions. Every registration is checked when it is made rather
//! than when it is first used:
//!
//! - ids must be non-empty and must not collide with a built-in or with each
//!   other — two passes sharing an id would make `--disable` ambiguous, and a
//!   backend shadowing `simulator` would silently change what that id means;
//! - a backend's cost model must be the one its profile names, and its rule
//!   set must reach its own basis, exactly as the built-ins are tested;
//! - a pass's placement must name a pass that exists.
//!
//! Every extension that took part in a compilation is listed in the result's
//! provenance, so a result can never hide that it came from outside the core.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use crate::analysis::lint::LintRule;
use crate::backend::{self, Backend, BackendError};
use crate::compile::Frontend;
use crate::frontend::{FrontendError, FrontendTiming};
use crate::ir::Circuit;
use crate::lowering::RuleSet;
use crate::pass::{Pass, PassContext, PassError, PassManager, PassOutput};

/// A source language mapped into QC-IR (§18.1).
///
/// The contract is the one every built-in frontend keeps (`docs/adapters.md`):
/// build circuits only through [`crate::ir::CircuitBuilder`], so the I1–I8
/// invariants are checked; resolve gate names through
/// [`crate::frontend::map_gate`]; and report what cannot be represented as
/// [`FrontendError::Unsupported`], naming the construct, rather than
/// approximating it.
pub trait FrontendPlugin: Send + Sync {
    /// Stable identifier, used on the command line (`--frontend`) and in
    /// provenance.
    fn id(&self) -> &str;

    /// One-line description.
    fn description(&self) -> &str;

    /// File extensions (without the dot, case-insensitive) this frontend
    /// claims when no frontend is named explicitly.
    fn file_extensions(&self) -> &[&str] {
        &[]
    }

    /// Reads `source` into a validated circuit named `name`.
    ///
    /// # Errors
    ///
    /// Whatever the language cannot read or represent.
    fn parse(&self, source: &str, name: &str) -> Result<Circuit, FrontendError>;

    /// As [`FrontendPlugin::parse`], with timing. The default reports the
    /// whole call as parsing and does not claim a separate IR-construction
    /// time it did not measure.
    ///
    /// # Errors
    ///
    /// As [`FrontendPlugin::parse`].
    fn parse_timed(
        &self,
        source: &str,
        name: &str,
    ) -> Result<(Circuit, FrontendTiming), FrontendError> {
        let started = std::time::Instant::now();
        let circuit = self.parse(source, name)?;
        Ok((
            circuit,
            FrontendTiming {
                parse: started.elapsed(),
                ir_construction: None,
            },
        ))
    }
}

/// Who supplied an extension, recorded in provenance.
///
/// Supplied by the extension crate as `ExtensionOrigin::new(env!("CARGO_PKG_NAME"),
/// env!("CARGO_PKG_VERSION"))`, so a result names the exact crate and version
/// that took part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionOrigin {
    /// The supplying crate's name.
    pub crate_name: String,
    /// The supplying crate's version.
    pub version: String,
}

impl ExtensionOrigin {
    /// An origin from a crate name and version.
    #[must_use]
    pub fn new(crate_name: impl Into<String>, version: impl Into<String>) -> Self {
        ExtensionOrigin {
            crate_name: crate_name.into(),
            version: version.into(),
        }
    }

    /// The origin of a target file loaded at runtime, rather than a crate.
    #[must_use]
    pub fn target_file(path: impl Into<String>) -> Self {
        ExtensionOrigin::new(format!("target-file:{}", path.into()), "-")
    }

    fn label(&self, kind: &str, id: &str) -> String {
        if self.version == "-" {
            format!("{} {kind}:{id}", self.crate_name)
        } else {
            format!("{}@{} {kind}:{id}", self.crate_name, self.version)
        }
    }
}

/// Where an extension pass goes in the pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassPlacement {
    /// Immediately before the first pass with this id.
    BeforeFirst(String),
    /// Immediately after the last pass with this id.
    AfterLast(String),
    /// At the very end.
    End,
}

impl Default for PassPlacement {
    /// Before `schedule`, which is read-only and should measure the circuit
    /// that actually comes out of optimization — including what an extension
    /// pass did to it.
    fn default() -> Self {
        PassPlacement::BeforeFirst("schedule".to_string())
    }
}

/// Why a registration was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ExtensionError {
    /// The id is empty.
    #[error("a {kind} extension must have a non-empty id")]
    EmptyId {
        /// `frontend`, `pass`, `backend` or `lint`.
        kind: &'static str,
    },
    /// The id is already taken, by a built-in or another extension.
    #[error("{kind} id `{id}` is already registered")]
    DuplicateId {
        /// `frontend`, `pass`, `backend` or `lint`.
        kind: &'static str,
        /// The id.
        id: String,
    },
    /// A pass placement names a pass that is not in the pipeline.
    #[error("pass `{pass}` is placed relative to `{anchor}`, which is not in the pipeline")]
    UnknownAnchor {
        /// The pass being placed.
        pass: String,
        /// The missing anchor.
        anchor: String,
    },
    /// A backend's own description is unusable.
    #[error(transparent)]
    Backend(#[from] BackendError),
}

/// The extensions one compilation may use.
#[derive(Clone, Default)]
pub struct Registry {
    frontends: Vec<(ExtensionOrigin, Arc<dyn FrontendPlugin>)>,
    backends: Vec<(ExtensionOrigin, Arc<dyn Backend>)>,
    passes: Vec<(ExtensionOrigin, PassPlacement, Arc<dyn Pass>)>,
    lint_rules: Vec<(ExtensionOrigin, Arc<dyn LintRule>)>,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field(
                "frontends",
                &self
                    .frontends
                    .iter()
                    .map(|(_, p)| p.id())
                    .collect::<Vec<_>>(),
            )
            .field(
                "backends",
                &self
                    .backends
                    .iter()
                    .map(|(_, b)| b.id())
                    .collect::<Vec<_>>(),
            )
            .field(
                "passes",
                &self
                    .passes
                    .iter()
                    .map(|(_, _, p)| p.id())
                    .collect::<Vec<_>>(),
            )
            .field(
                "lint_rules",
                &self
                    .lint_rules
                    .iter()
                    .map(|(_, r)| r.id())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// A built-in text frontend seen through the plugin trait.
struct BuiltinFrontend(Frontend);

impl FrontendPlugin for BuiltinFrontend {
    fn id(&self) -> &str {
        self.0.id()
    }

    fn description(&self) -> &str {
        match self.0 {
            Frontend::OpenQasm3 => "the documented OpenQASM 3 subset",
            Frontend::CudaQ => "the documented CUDA-Q kernel subset",
        }
    }

    fn parse(&self, source: &str, name: &str) -> Result<Circuit, FrontendError> {
        self.0.parse(source, name)
    }

    fn parse_timed(
        &self,
        source: &str,
        name: &str,
    ) -> Result<(Circuit, FrontendTiming), FrontendError> {
        self.0.parse_timed(source, name)
    }
}

/// An extension pass shared between registries and pipelines.
struct SharedPass(Arc<dyn Pass>);

impl Pass for SharedPass {
    fn id(&self) -> &'static str {
        self.0.id()
    }

    fn description(&self) -> &'static str {
        self.0.description()
    }

    fn run(&self, circuit: &Circuit, context: &PassContext<'_>) -> Result<PassOutput, PassError> {
        self.0.run(circuit, context)
    }
}

impl Registry {
    /// No extensions: exactly the built-in compiler.
    #[must_use]
    pub fn new() -> Self {
        Registry::default()
    }

    /// `true` if nothing has been registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frontends.is_empty()
            && self.backends.is_empty()
            && self.passes.is_empty()
            && self.lint_rules.is_empty()
    }

    // --- Frontends -------------------------------------------------------

    /// Registers a frontend.
    ///
    /// # Errors
    ///
    /// [`ExtensionError::EmptyId`] or [`ExtensionError::DuplicateId`],
    /// including a collision with `openqasm3` or `cudaq`.
    pub fn register_frontend(
        &mut self,
        origin: ExtensionOrigin,
        frontend: Arc<dyn FrontendPlugin>,
    ) -> Result<&mut Self, ExtensionError> {
        let id = frontend.id().to_string();
        check_id("frontend", &id)?;
        if self.frontend_ids().contains(&id) {
            return Err(ExtensionError::DuplicateId {
                kind: "frontend",
                id,
            });
        }
        self.frontends.push((origin, frontend));
        Ok(self)
    }

    /// Every frontend id, built-ins first.
    #[must_use]
    pub fn frontend_ids(&self) -> Vec<String> {
        Frontend::ALL
            .iter()
            .map(|f| f.id().to_string())
            .chain(self.frontends.iter().map(|(_, p)| p.id().to_string()))
            .collect()
    }

    /// Looks a frontend up by id, built-in or extension.
    #[must_use]
    pub fn frontend(&self, id: &str) -> Option<Arc<dyn FrontendPlugin>> {
        if let Some(builtin) = Frontend::from_id(id) {
            return Some(Arc::new(BuiltinFrontend(builtin)));
        }
        self.frontends
            .iter()
            .find(|(_, p)| p.id() == id)
            .map(|(_, p)| Arc::clone(p))
    }

    /// The frontend a file's extension implies: an extension that claims it
    /// first, otherwise the built-in rule ([`Frontend::for_path`]).
    #[must_use]
    pub fn frontend_for_path(&self, path: &Path) -> Arc<dyn FrontendPlugin> {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            for (_, plugin) in &self.frontends {
                if plugin
                    .file_extensions()
                    .iter()
                    .any(|claimed| claimed.eq_ignore_ascii_case(ext))
                {
                    return Arc::clone(plugin);
                }
            }
        }
        Arc::new(BuiltinFrontend(Frontend::for_path(path)))
    }

    // --- Backends --------------------------------------------------------

    /// Registers a backend.
    ///
    /// # Errors
    ///
    /// [`ExtensionError::EmptyId`], [`ExtensionError::DuplicateId`], or
    /// [`ExtensionError::Backend`] if its cost model disagrees with its
    /// profile or its rule set cannot reach its own basis — the checks every
    /// built-in backend is tested against.
    pub fn register_backend(
        &mut self,
        origin: ExtensionOrigin,
        backend: Arc<dyn Backend>,
    ) -> Result<&mut Self, ExtensionError> {
        let id = backend.id().to_string();
        check_id("backend", &id)?;
        if self.backend_ids().contains(&id) {
            return Err(ExtensionError::DuplicateId {
                kind: "backend",
                id,
            });
        }
        if backend.cost_model().id() != backend.profile().cost_model_id() {
            return Err(BackendError::InvalidTarget {
                backend: id,
                detail: format!(
                    "the cost model is `{}` but the profile names `{}`",
                    backend.cost_model().id(),
                    backend.profile().cost_model_id()
                ),
            }
            .into());
        }
        RuleSet::new(backend.profile()).map_err(|source| BackendError::InvalidTarget {
            backend: id.clone(),
            detail: format!("decomposition rules do not reach the basis: {source}"),
        })?;
        self.backends.push((origin, backend));
        Ok(self)
    }

    /// Loads a serialized target description and registers it as a backend
    /// ([`backend::from_target_json`]), returning the backend's id.
    ///
    /// # Errors
    ///
    /// As [`backend::from_target_json`] and [`Registry::register_backend`].
    pub fn register_target_json(
        &mut self,
        origin: ExtensionOrigin,
        json: &str,
        kind: Option<&str>,
        id: Option<&str>,
    ) -> Result<String, ExtensionError> {
        let backend: Arc<dyn Backend> = Arc::from(backend::from_target_json(json, kind, id)?);
        let id = backend.id().to_string();
        self.register_backend(origin, backend)?;
        Ok(id)
    }

    /// Every backend id, built-ins first.
    #[must_use]
    pub fn backend_ids(&self) -> Vec<String> {
        backend::all()
            .iter()
            .map(|b| b.id().to_string())
            .chain(self.backends.iter().map(|(_, b)| b.id().to_string()))
            .collect()
    }

    /// Every backend as `(id, description)`, built-ins first.
    #[must_use]
    pub fn available_backends(&self) -> Vec<(String, String)> {
        backend::all()
            .iter()
            .map(|b| (b.id().to_string(), b.description().to_string()))
            .chain(
                self.backends
                    .iter()
                    .map(|(_, b)| (b.id().to_string(), b.description().to_string())),
            )
            .collect()
    }

    /// Looks a backend up by id: an extension, or a built-in.
    #[must_use]
    pub fn backend(&self, id: &str) -> Option<Arc<dyn Backend>> {
        self.backends
            .iter()
            .find(|(_, b)| b.id() == id)
            .map(|(_, b)| Arc::clone(b))
            .or_else(|| backend::by_id(id).map(Arc::from))
    }

    // --- Passes ----------------------------------------------------------

    /// Registers a pass at a position in the default pipeline.
    ///
    /// # Errors
    ///
    /// [`ExtensionError::EmptyId`], [`ExtensionError::DuplicateId`] (a
    /// collision would make pass selection apply to both), or
    /// [`ExtensionError::UnknownAnchor`].
    pub fn register_pass(
        &mut self,
        origin: ExtensionOrigin,
        placement: PassPlacement,
        pass: Arc<dyn Pass>,
    ) -> Result<&mut Self, ExtensionError> {
        let id = pass.id().to_string();
        check_id("pass", &id)?;
        let existing = self.pass_ids();
        if existing.contains(&id) {
            return Err(ExtensionError::DuplicateId { kind: "pass", id });
        }
        if let PassPlacement::BeforeFirst(anchor) | PassPlacement::AfterLast(anchor) = &placement
            && !existing.contains(anchor)
        {
            return Err(ExtensionError::UnknownAnchor {
                pass: id,
                anchor: anchor.clone(),
            });
        }
        self.passes.push((origin, placement, pass));
        Ok(self)
    }

    /// The default pipeline with every extension pass spliced in, in
    /// registration order.
    #[must_use]
    pub fn pass_manager(&self) -> PassManager {
        let mut manager = PassManager::default_pipeline();
        for (_, placement, pass) in &self.passes {
            let ids = manager.pass_ids();
            let index = match placement {
                PassPlacement::BeforeFirst(anchor) => ids.iter().position(|id| id == anchor),
                PassPlacement::AfterLast(anchor) => {
                    ids.iter().rposition(|id| id == anchor).map(|i| i + 1)
                }
                PassPlacement::End => Some(ids.len()),
            }
            .expect("anchors are checked at registration");
            manager.insert(index, Box::new(SharedPass(Arc::clone(pass))));
        }
        manager
    }

    /// The ids of the composed pipeline, in execution order (with repeats) —
    /// the ids pass selection accepts.
    #[must_use]
    pub fn pass_ids(&self) -> Vec<String> {
        self.pass_manager()
            .pass_ids()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    // --- Lint rules ------------------------------------------------------

    /// Registers a lint rule (§13.5's modular analysis boundary).
    ///
    /// # Errors
    ///
    /// [`ExtensionError::EmptyId`] or [`ExtensionError::DuplicateId`],
    /// including a collision with a built-in rule.
    pub fn register_lint_rule(
        &mut self,
        origin: ExtensionOrigin,
        rule: Arc<dyn LintRule>,
    ) -> Result<&mut Self, ExtensionError> {
        let id = rule.id().to_string();
        check_id("lint", &id)?;
        if self
            .linter()
            .rule_ids()
            .iter()
            .any(|existing| *existing == id)
        {
            return Err(ExtensionError::DuplicateId { kind: "lint", id });
        }
        self.lint_rules.push((origin, rule));
        Ok(self)
    }

    /// The built-in lint rules plus every registered one.
    #[must_use]
    pub fn linter(&self) -> crate::analysis::lint::Linter {
        let mut linter = crate::analysis::lint::Linter::default_rules();
        for (_, rule) in &self.lint_rules {
            linter.register(Arc::clone(rule));
        }
        linter
    }

    // --- Provenance ------------------------------------------------------

    /// The provenance labels of the extensions a compilation used: its
    /// frontend if that was an extension, every extension pass that ran, and
    /// its backend if that was an extension. Sorted, so the list is
    /// deterministic.
    #[must_use]
    pub fn labels_for(
        &self,
        frontend: Option<&str>,
        passes_run: &[String],
        backend: Option<&str>,
    ) -> Vec<String> {
        let mut labels = Vec::new();
        if let Some(frontend) = frontend {
            for (origin, plugin) in &self.frontends {
                if plugin.id() == frontend {
                    labels.push(origin.label("frontend", frontend));
                }
            }
        }
        for (origin, _, pass) in &self.passes {
            if passes_run.iter().any(|id| id == pass.id()) {
                labels.push(origin.label("pass", pass.id()));
            }
        }
        if let Some(backend) = backend {
            for (origin, registered) in &self.backends {
                if registered.id() == backend {
                    labels.push(origin.label("backend", backend));
                }
            }
        }
        labels.sort();
        labels
    }
}

fn check_id(kind: &'static str, id: &str) -> Result<(), ExtensionError> {
    if id.trim().is_empty() {
        Err(ExtensionError::EmptyId { kind })
    } else {
        Ok(())
    }
}
