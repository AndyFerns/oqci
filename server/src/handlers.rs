//! Trivial REST endpoints — thin wrappers over already-public, already-
//! serializable compiler data.

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Serialize)]
pub struct BackendInfo {
    id: String,
    description: String,
}

pub async fn backends() -> Json<Vec<BackendInfo>> {
    Json(
        oqci::compile::available_backends()
            .into_iter()
            .map(|(id, description)| BackendInfo { id, description })
            .collect(),
    )
}

/// The built-in target profiles — already `Serialize` (`BasisProfile`
/// derives it directly), so this endpoint does no view-building of its own.
pub async fn targets() -> Json<Vec<oqci::target::BasisProfile>> {
    Json(oqci::target::builtin::all())
}

#[derive(Serialize)]
pub struct DefaultBackendResponse {
    default_backend: String,
}

pub async fn default_backend(State(state): State<AppState>) -> Json<DefaultBackendResponse> {
    Json(DefaultBackendResponse {
        default_backend: state.default_backend,
    })
}
