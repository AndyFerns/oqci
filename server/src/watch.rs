//! Independent, server-owned file watching.
//!
//! Uses the same technique `oqci watch` (`src/cli/watch.rs`) already uses —
//! watch the parent directory rather than the file (editors often save by
//! writing a temp file and renaming it over the original, which changes the
//! inode and would make a file-level watch go silent after the first save),
//! debounce the resulting burst of events, and keep going through an
//! individual error. Reimplemented standalone here rather than by touching
//! or importing from `src/cli/`: watching a filesystem path is not compiler
//! logic, and the CLI's `watch` module is CLI-internal and out of scope to
//! modify or depend on.

use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher};
use tokio::sync::broadcast;

const DEBOUNCE: Duration = Duration::from_millis(150);

/// Starts watching `path`, returning a broadcast receiver that fires once
/// immediately (so a new subscriber compiles without waiting for an edit)
/// and again after every debounced change, until the returned watcher
/// thread's sender is dropped.
///
/// One watcher is started per call — acceptable for a single-user local
/// dev-companion tool (multiple browser tabs on the same file each get
/// their own OS watch), and simpler than a shared-watcher registry that
/// Phase 0's scope does not need.
pub fn watch(path: PathBuf) -> notify::Result<broadcast::Receiver<()>> {
    let (tx, rx) = broadcast::channel(16);
    let target = path.canonicalize().ok();
    let directory = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let (raw_tx, raw_rx) = std_mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = raw_tx.send(event);
    })?;
    watcher.watch(&directory, RecursiveMode::NonRecursive)?;

    let watch_tx = tx.clone();
    std::thread::spawn(move || {
        // Keeps the watcher alive for the life of this thread.
        let _watcher = watcher;
        loop {
            let Ok(event) = raw_rx.recv() else {
                return;
            };
            if !is_relevant(&event, target.as_deref(), &path) {
                continue;
            }

            let deadline = Instant::now() + DEBOUNCE;
            while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
                match raw_rx.recv_timeout(remaining) {
                    Ok(_) => {}
                    Err(std_mpsc::RecvTimeoutError::Timeout) => break,
                    Err(std_mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }

            // A send failure only means every receiver dropped, i.e. the
            // last interested WebSocket connection closed.
            let _ = watch_tx.send(());
        }
    });

    let _ = tx.send(());
    Ok(rx)
}

fn is_relevant(
    event: &notify::Result<notify::Event>,
    canonical_target: Option<&Path>,
    path: &Path,
) -> bool {
    let Ok(event) = event else {
        return false;
    };
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return false;
    }
    event.paths.iter().any(|changed| {
        match (changed.canonicalize().ok(), canonical_target) {
            (Some(a), Some(b)) if a == b => true,
            _ => changed.file_name() == path.file_name(),
        }
    })
}
