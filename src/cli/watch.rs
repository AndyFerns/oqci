//! Watch mode: re-run the pipeline whenever the source file changes.
//!
//! This is the live input → output loop. It adds no compiler logic — it calls
//! the same [`crate::cli::pipeline`] functions the one-shot commands call,
//! then re-renders.
//!
//! Two details matter for it to actually feel live:
//!
//! - **Watch the directory, not the file.** Many editors save by writing a
//!   temporary file and renaming it over the original. That replaces the
//!   inode, so a watch on the file itself goes deaf after the first save.
//! - **Keep running after an error.** A file being edited is malformed most
//!   of the time. A parse error prints the diagnostic and waits for the next
//!   save; it does not exit, because exiting exactly when the user is mid-edit
//!   defeats the purpose.

use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher};

use crate::cli::CliError;

/// Coalescing window: editors emit several events per save.
const DEBOUNCE: Duration = Duration::from_millis(150);

/// Watches `path`, calling `render` once immediately and again after every
/// change, until interrupted.
///
/// `render` returns the text to display, or an error to show while continuing
/// to watch.
///
/// # Errors
///
/// [`CliError::Watch`] if the filesystem watch cannot be established — the
/// only failure that ends the loop.
pub fn watch(
    path: &Path,
    mut render: impl FnMut() -> Result<String, CliError>,
) -> Result<(), CliError> {
    let directory = path.parent().filter(|p| !p.as_os_str().is_empty());
    let directory = directory.unwrap_or_else(|| Path::new("."));

    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        // A send failure only means the receiver is gone, i.e. we are exiting.
        let _ = tx.send(event);
    })
    .map_err(|e| CliError::Watch(e.to_string()))?;

    watcher
        .watch(directory, RecursiveMode::NonRecursive)
        .map_err(|e| CliError::Watch(format!("cannot watch {}: {e}", directory.display())))?;

    let target = path.canonicalize().ok();
    println!("watching {} — press Ctrl+C to stop", path.display());
    show(&mut render);

    loop {
        let Ok(event) = rx.recv() else {
            // The watcher was dropped; nothing more will arrive.
            return Ok(());
        };

        if !is_relevant(&event, target.as_deref(), path) {
            continue;
        }

        // Coalesce the burst of events a single save produces.
        let deadline = Instant::now() + DEBOUNCE;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(remaining) {
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }

        show(&mut render);
    }
}

/// Clears the screen and prints the current render, or the current error.
fn show(render: &mut impl FnMut() -> Result<String, CliError>) {
    // ANSI clear + home. Harmless when redirected, and this is the one place
    // escape codes earn their keep: a live view that scrolls is unreadable.
    print!("\u{1b}[2J\u{1b}[H");

    match render() {
        Ok(text) => println!("{text}"),
        // A malformed file mid-edit is expected, not fatal.
        Err(error) => println!("error: {error}"),
    }
}

/// Whether an event concerns the file being watched.
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
        // Compare canonically when possible (handles the save-and-rename
        // dance), and fall back to the filename for files that momentarily
        // do not exist.
        match (changed.canonicalize().ok(), canonical_target) {
            (Some(a), Some(b)) if a == b => true,
            _ => changed.file_name() == path.file_name(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::Event;
    use notify::event::{DataChange, ModifyKind};
    use std::path::PathBuf;

    fn modify_event(path: &str) -> notify::Result<Event> {
        Ok(Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            paths: vec![PathBuf::from(path)],
            attrs: notify::event::EventAttributes::default(),
        })
    }

    #[test]
    fn events_for_the_watched_filename_are_relevant() {
        let event = modify_event("some/dir/bell.qasm");
        assert!(is_relevant(&event, None, Path::new("other/bell.qasm")));
    }

    #[test]
    fn events_for_other_files_are_ignored() {
        let event = modify_event("some/dir/other.qasm");
        assert!(!is_relevant(&event, None, Path::new("bell.qasm")));
    }

    #[test]
    fn access_events_are_ignored() {
        let event = Ok(Event {
            kind: EventKind::Access(notify::event::AccessKind::Read),
            paths: vec![PathBuf::from("bell.qasm")],
            attrs: notify::event::EventAttributes::default(),
        });
        assert!(!is_relevant(&event, None, Path::new("bell.qasm")));
    }

    #[test]
    fn watcher_errors_are_ignored_rather_than_triggering_a_rerun() {
        let event: notify::Result<Event> = Err(notify::Error::generic("permission denied"));
        assert!(!is_relevant(&event, None, Path::new("bell.qasm")));
    }
}
