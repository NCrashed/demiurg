//! Stamps the build with the git commit it was built from, exposed as the
//! `DEMIURG_GIT_COMMIT` compile-time env var (see `BUILD_INFO` in `ui.rs`).
//! Best effort: a source tarball (no `.git`) builds fine, just "unknown".

use std::path::Path;
use std::process::Command;

fn main() {
    let commit = git_short_hash().unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=DEMIURG_GIT_COMMIT={commit}");

    // Recompute when HEAD (or the branch it points at) moves, so the stamp
    // tracks commits/checkouts. The paths are relative to this crate's
    // manifest dir; on a `.git`-less build they simply don't exist.
    let git = Path::new("../../.git");
    if git.join("HEAD").exists() {
        println!("cargo:rerun-if-changed=../../.git/HEAD");
        if let Some(reference) = head_ref() {
            println!("cargo:rerun-if-changed=../../.git/{reference}");
        }
    }
}

/// `git rev-parse --short HEAD`, or `None` if git is unavailable / not a repo.
fn git_short_hash() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let hash = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!hash.is_empty()).then_some(hash)
}

/// The ref `HEAD` points at (e.g. `refs/heads/master`), or `None` when
/// detached or unreadable.
fn head_ref() -> Option<String> {
    let head = std::fs::read_to_string("../../.git/HEAD").ok()?;
    head.strip_prefix("ref: ").map(|r| r.trim().to_string())
}
