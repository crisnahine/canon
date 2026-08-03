//! Where canon keeps its own state.
//!
//! Never inside the repository being indexed. A tool that writes into someone
//! else's working tree shows up in their `git status`, their diffs and their
//! commits, and gets removed for it.

use std::hash::{Hash as _, Hasher as _};
use std::path::{Path, PathBuf};

/// Root of canon's own storage, in falling order of preference.
///
/// `CLAUDE_PLUGIN_DATA` is supplied by the host to plugin hooks and is the
/// correct answer when running as one. The rest are for running the binary by
/// hand.
#[must_use]
pub(crate) fn data_dir() -> PathBuf {
    for key in ["CANON_DATA_DIR", "CLAUDE_PLUGIN_DATA"] {
        if let Some(dir) = std::env::var_os(key).filter(|v| !v.is_empty()) {
            return PathBuf::from(dir);
        }
    }
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join("canon");
    }
    home().map_or_else(|| std::env::temp_dir().join("canon"), |h| h.join(".local/share/canon"))
}

/// The snapshot for one repository.
///
/// Keyed by a hash of the absolute root rather than by its name, so two
/// checkouts of the same project do not share a snapshot and a path containing
/// a slash or a space cannot produce a nested or invalid file name.
#[must_use]
pub(crate) fn snapshot_path(root: &Path) -> PathBuf {
    data_dir().join("snapshots").join(format!("{}.json", key_for(root)))
}

/// The list of files touched during one session, written by the post-write
/// hook and read once at the end of the turn.
#[must_use]
pub(crate) fn touched_path(root: &Path, session_id: &str) -> PathBuf {
    let session = if session_id.is_empty() { "anon" } else { session_id };
    let safe: String =
        session.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    data_dir().join("sessions").join(format!("{}-{}.touched", key_for(root), safe))
}

/// The log file. One per install, not per repository, so a user chasing a
/// problem has one place to look.
#[must_use]
pub(crate) fn log_path() -> PathBuf {
    data_dir().join("canon.log")
}

fn key_for(root: &Path) -> String {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    canonical.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn two_checkouts_of_one_project_do_not_share_a_snapshot() {
        let a = snapshot_path(Path::new("/work/canon"));
        let b = snapshot_path(Path::new("/work/canon-2"));
        assert_ne!(a, b);
    }

    #[test]
    fn the_same_root_always_maps_to_the_same_snapshot() {
        assert_eq!(snapshot_path(Path::new("/work/x")), snapshot_path(Path::new("/work/x")));
    }

    #[test]
    fn a_snapshot_file_name_never_contains_a_path_separator() {
        let path = snapshot_path(Path::new("/deeply/nested/repo with spaces"));
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(!name.contains('/'), "got {name}");
        assert!(std::path::Path::new(name).extension().is_some_and(|e| e == "json"));
    }

    #[test]
    fn a_hostile_session_id_cannot_escape_the_sessions_directory() {
        let path = touched_path(Path::new("/work/x"), "../../etc/passwd");
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(!name.contains(".."), "got {name}");
        assert!(!name.contains('/'), "got {name}");
        assert!(path.parent().unwrap().ends_with("sessions"));
    }

    #[test]
    fn an_empty_session_id_still_produces_a_usable_name() {
        let path = touched_path(Path::new("/work/x"), "");
        assert!(path.file_name().unwrap().to_str().unwrap().contains("anon"));
    }

    #[test]
    fn nothing_canon_writes_lands_inside_the_repository() {
        // The property that keeps canon out of the user's git status.
        let repo = Path::new("/work/some-repo");
        for path in [snapshot_path(repo), touched_path(repo, "s1"), log_path()] {
            assert!(!path.starts_with(repo), "{} is inside the repo", path.display());
        }
    }
}
