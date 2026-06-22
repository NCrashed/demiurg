//! A tiny, dependency-free store of recently opened documents, persisted to a
//! plain-text file in the user's config dir (one absolute path per line, most
//! recent first). It also backs the file dialog's "remember the last folder"
//! behaviour: the newest entry's parent directory is where the next Open /
//! Save dialog starts.
//!
//! The format is deliberately trivial — a leading version comment plus one path
//! per line — so it needs no serde and is safe to hand-edit. A missing or
//! unreadable file just yields an empty list; we never error out over recents.

use std::path::{Path, PathBuf};

/// How many entries to keep (older ones drop off the end).
const MAX: usize = 12;

/// Header written as the first line, so the file is self-describing and a
/// future format bump can be detected.
const HEADER: &str = "# demiurg recent files v1";

/// Path of the recent-files store: `$XDG_CONFIG_HOME/demiurg/recent` (or
/// `$HOME/.config/demiurg/recent`), falling back to the OS temp dir if neither
/// is set.
fn store_path() -> PathBuf {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(std::env::temp_dir);
    dir.join("demiurg").join("recent")
}

/// Load the recent list (most recent first). Blank lines and the header are
/// skipped; a missing / unreadable file is an empty list.
pub fn load() -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(store_path()) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(PathBuf::from)
        .take(MAX)
        .collect()
}

/// Record `path` as the most recently used document: move it to the front
/// (de-duplicating), cap the list, and persist. Returns the updated list.
/// Persistence failures are logged and otherwise ignored — recents are a
/// convenience, never a hard dependency.
pub fn push(list: &[PathBuf], path: &Path) -> Vec<PathBuf> {
    // Canonicalize so the same file reached by different relative paths
    // de-dupes; fall back to the path as-given if it can't be resolved.
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut out = Vec::with_capacity(list.len() + 1);
    out.push(key.clone());
    for p in list {
        if *p != key {
            out.push(p.clone());
        }
    }
    out.truncate(MAX);
    save(&out);
    out
}

/// Persist the list to disk (creating the parent dir). Best-effort.
fn save(list: &[PathBuf]) {
    let path = store_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("demiurg: recent: create {}: {e}", parent.display());
            return;
        }
    }
    let mut body = String::from(HEADER);
    body.push('\n');
    for p in list {
        body.push_str(&p.to_string_lossy());
        body.push('\n');
    }
    if let Err(e) = std::fs::write(&path, body) {
        eprintln!("demiurg: recent: write {}: {e}", path.display());
    }
}

/// Forget the entire recent list (persists an empty file).
pub fn clear() {
    save(&[]);
}

/// The directory a file dialog should open in: the newest recent entry's parent
/// (the last folder the user worked in). `None` when there's no usable entry.
pub fn last_dir(list: &[PathBuf]) -> Option<PathBuf> {
    list.iter()
        .find_map(|p| p.parent().filter(|d| d.is_dir()).map(Path::to_path_buf))
}
