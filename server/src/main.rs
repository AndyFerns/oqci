//! `oqci-server` — a local, read-only visualization companion for the OQCI
//! compiler.
//!
//! This binary never decides anything about compilation. It calls the real
//! `oqci` library — the same functions the `oqci` CLI calls — and streams
//! the result to a browser. See `docs/visualization.md`.

mod compile;
mod error;
mod events;
mod handlers;
mod replay;
mod state;
mod watch;
mod ws;

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::Router;
use axum::routing::get;
use clap::Parser;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use state::AppState;

#[derive(Parser)]
#[command(about = "Local live-visualization server for the OQCI compiler")]
struct Args {
    /// Directory `watch_file` requests are resolved (and confined) within.
    #[arg(long, default_value = ".")]
    root: PathBuf,

    /// Port to listen on.
    #[arg(long, default_value_t = 4173)]
    port: u16,

    /// Backend a watched file is compiled against. Must be one of
    /// `oqci backends`.
    #[arg(long, default_value = "simulator-nisq")]
    backend: String,

    /// Directory holding the built frontend, served at `/`. Omit to run the
    /// API only.
    #[arg(long)]
    static_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("oqci_server=info".parse()?))
        .init();

    let args = Args::parse();

    if oqci::backend::by_id(&args.backend).is_none() {
        let available: Vec<String> = oqci::compile::available_backends()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        anyhow::bail!(
            "unknown backend `{}`; available: {}",
            args.backend,
            available.join(", ")
        );
    }

    let root = args.root.canonicalize().unwrap_or(args.root);
    let state = AppState {
        default_backend: args.backend,
        root,
    };

    let mut app = Router::new()
        .route("/api/health", get(handlers::health))
        .route("/api/backends", get(handlers::backends))
        .route("/api/targets", get(handlers::targets))
        .route("/api/config", get(handlers::default_backend))
        .route("/api/watch", get(ws::watch_handler))
        .with_state(state.clone())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    if let Some(static_dir) = args.static_dir {
        app = app.fallback_service(ServeDir::new(static_dir));
    }

    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    tracing::info!("oqci-server listening on http://{addr}, root {}", state.root.display());
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
