//! Shared server configuration.

use std::path::PathBuf;

#[derive(Clone)]
pub struct AppState {
    /// The backend a watched file is compiled against, unless the client
    /// specifies otherwise. Must name one of `oqci::compile::available_backends()`.
    pub default_backend: String,
    /// Paths in a `watch_file` request are resolved relative to this root,
    /// and must stay within it (no `..`, no absolute escape) — the one
    /// input-validation rule this local dev-companion enforces, since it is
    /// reading arbitrary files named over a WebSocket.
    pub root: PathBuf,
}
