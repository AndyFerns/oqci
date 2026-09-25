//! The `/api/watch` WebSocket handler.
//!
//! Two request shapes (see `events::ClientMessage`): `watch_file` drives a
//! live loop off a filesystem watcher (`watch.rs`), `compile_source` compiles
//! once per message the client sends — the seam a later hosted-playground
//! phase would build on, already implemented rather than stubbed, since it
//! needs nothing a watched-file compile doesn't already have.
//!
//! Every push this handler sends is computed by `compile::compile` (the
//! ground truth) and `replay::{replay_passes, replay_lowering}` (self-
//! validating reconstructions of it). This module's only job is wiring:
//! read a message, call those functions, serialize, send.

use std::path::{Path, PathBuf};

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use oqci::pass::{PassContext, PassSelection};

use crate::compile::{CompileRequest, CompileResult};
use crate::events::{ClientMessage, ServerMessage};
use crate::replay;
use crate::state::AppState;

type Sender = SplitSink<WebSocket, Message>;
type Receiver = SplitStream<WebSocket>;

pub async fn watch_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut sequence: u64 = 0;

    let Some(Ok(Message::Text(first))) = receiver.next().await else {
        return;
    };
    let Ok(first) = serde_json::from_str::<ClientMessage>(&first) else {
        send_error(&mut sender, &mut sequence, "could not parse subscribe message").await;
        return;
    };

    match first {
        ClientMessage::WatchFile { path } => {
            watch_loop(&mut sender, &mut receiver, &state, &mut sequence, path).await;
        }
        ClientMessage::CompileSource { source, name } => {
            source_loop(&mut sender, &mut receiver, &state, &mut sequence, source, name).await;
        }
    }
}

async fn watch_loop(
    sender: &mut Sender,
    receiver: &mut Receiver,
    state: &AppState,
    sequence: &mut u64,
    requested_path: String,
) {
    let Some(resolved) = resolve_path(&state.root, &requested_path) else {
        send_error(sender, sequence, "path escapes the server root").await;
        return;
    };

    let mut changes = match crate::watch::watch(resolved.clone()) {
        Ok(rx) => rx,
        Err(e) => {
            send_error(
                sender,
                sequence,
                &format!("could not watch `{}`: {e}", resolved.display()),
            )
            .await;
            return;
        }
    };

    loop {
        tokio::select! {
            changed = changes.recv() => {
                if changed.is_err() {
                    return;
                }
                compile_and_send(sender, state, sequence, &resolved).await;
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Err(_)) => return,
                    _ => {}
                }
            }
        }
    }
}

async fn source_loop(
    sender: &mut Sender,
    receiver: &mut Receiver,
    state: &AppState,
    sequence: &mut u64,
    mut source: String,
    mut name: String,
) {
    loop {
        compile_source_and_send(sender, state, sequence, &source, &name).await;

        match receiver.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientMessage>(&text) {
                Ok(ClientMessage::CompileSource { source: s, name: n }) => {
                    source = s;
                    name = n;
                }
                Ok(ClientMessage::WatchFile { .. }) => {
                    send_error(
                        sender,
                        sequence,
                        "switching from compile_source to watch_file mid-connection is not supported; reconnect",
                    )
                    .await;
                    return;
                }
                Err(_) => {
                    send_error(sender, sequence, "could not parse message").await;
                }
            },
            Some(Ok(Message::Close(_))) | None => return,
            Some(Err(_)) => return,
            _ => {}
        }
    }
}

fn resolve_path(root: &Path, requested: &str) -> Option<PathBuf> {
    let candidate = root.join(requested);
    let canonical_root = root.canonicalize().ok()?;
    let canonical_candidate = candidate.canonicalize().ok().unwrap_or(candidate);
    if canonical_candidate.starts_with(&canonical_root) {
        Some(canonical_candidate)
    } else {
        None
    }
}

async fn compile_and_send(sender: &mut Sender, state: &AppState, sequence: &mut u64, path: &Path) {
    let source = match tokio::fs::read_to_string(path).await {
        Ok(s) => s,
        Err(e) => {
            send_error(
                sender,
                sequence,
                &format!("could not read `{}`: {e}", path.display()),
            )
            .await;
            return;
        }
    };
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("main")
        .to_string();
    compile_source_and_send(sender, state, sequence, &source, &name).await;
}

async fn compile_source_and_send(
    sender: &mut Sender,
    state: &AppState,
    sequence: &mut u64,
    source: &str,
    name: &str,
) {
    let seq = *sequence;
    *sequence += 1;

    let request = CompileRequest {
        source: source.to_string(),
        name: name.to_string(),
        backend: Some(state.default_backend.clone()),
        bindings: Default::default(),
    };

    let CompileResult { report, artifacts } = match crate::compile::compile(&request) {
        Ok(r) => r,
        Err(e) => {
            send(
                sender,
                &ServerMessage::CompileError {
                    sequence: seq,
                    message: e.to_string(),
                },
            )
            .await;
            return;
        }
    };

    send(
        sender,
        &ServerMessage::PipelineReport {
            sequence: seq,
            report,
        },
    )
    .await;

    let backend = oqci::backend::by_id(&state.default_backend);
    let context = match &backend {
        Some(b) => PassContext::with_profile(b.profile()).and_cost_model(b.cost_model()),
        None => PassContext::none(),
    };
    let pass_replay = replay::replay_passes(
        &artifacts.source_circuit,
        &PassSelection::All,
        &context,
        &artifacts.pass_records,
    );
    send(
        sender,
        &ServerMessage::PassReplay {
            sequence: seq,
            replay: pass_replay,
        },
    )
    .await;

    if let (Some(lowered), Some(backend)) = (&artifacts.lowered, &backend) {
        let lowering_config = oqci::lowering::LoweringConfig::default();
        let lowering_replay =
            replay::replay_lowering(&artifacts.optimized, backend.profile(), &lowering_config, lowered);
        send(
            sender,
            &ServerMessage::LoweringReplay {
                sequence: seq,
                replay: lowering_replay,
            },
        )
        .await;
    }
}

async fn send_error(sender: &mut Sender, sequence: &mut u64, message: &str) {
    let seq = *sequence;
    *sequence += 1;
    send(
        sender,
        &ServerMessage::CompileError {
            sequence: seq,
            message: message.to_string(),
        },
    )
    .await;
}

async fn send(sender: &mut Sender, message: &ServerMessage) {
    if let Ok(text) = serde_json::to_string(message) {
        let _ = sender.send(Message::Text(text)).await;
    }
}
