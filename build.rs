//! Captures the git commit at build time, for execution provenance.
//!
//! Stage C §9 requires every execution result to be attributable to a git
//! commit, among other things. Reading it here rather than shelling out at
//! runtime keeps the compiler's own execution path free of process spawning,
//! and means a released binary carries the commit it was built from even if
//! the checkout is long gone.
//!
//! Failure is not an error. A crate built from a tarball, a vendored copy or
//! a registry download has no git metadata, and refusing to build in that
//! case would be absurd. The commit becomes `"unknown"` — which is honest,
//! and which the provenance record reports as such rather than omitting the
//! field and letting a reader assume it was never recorded.

use std::process::Command;

fn main() {
    let commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|hash| hash.trim().to_string())
        .filter(|hash| !hash.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=OQCI_GIT_COMMIT={commit}");

    // Rebuild when HEAD moves, so a stale commit cannot be baked in. Both
    // paths are emitted unconditionally; cargo ignores ones that do not exist.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");
}
