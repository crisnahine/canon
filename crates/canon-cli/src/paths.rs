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

/// The directory holding one snapshot per repository.
#[must_use]
pub(crate) fn snapshots_dir() -> PathBuf {
    data_dir().join("snapshots")
}

/// The directory holding the per-session lists of touched files.
#[must_use]
pub(crate) fn sessions_dir() -> PathBuf {
    data_dir().join("sessions")
}

/// Delete state canon will never read again.
///
/// Two things accumulate, both slowly and neither bounded. A snapshot is
/// written per repository and nothing removes it when that repository is
/// deleted or renamed. A session's touched-file list is removed when the turn
/// ends, and a session that crashes or is killed never ends, so its list stays
/// forever.
///
/// Swept on the cold path, where a directory listing is free, and by age
/// rather than by reachability: canon cannot tell an abandoned repository from
/// one nobody has opened this month, but a snapshot expires after a day, so
/// anything much older than that is not being used.
///
/// Never fails. This is housekeeping; a permissions problem here must not cost
/// the session its conventions.
pub(crate) fn sweep_stale(now_unix: u64) -> usize {
    let mut removed = 0;
    removed += sweep(&snapshots_dir(), now_unix, SNAPSHOT_RETENTION_SECONDS);
    removed += sweep(&sessions_dir(), now_unix, SESSION_RETENTION_SECONDS);
    removed
}

/// A snapshot older than this belongs to a repository nobody is working in.
/// Well past the one-day freshness window, so an active repository is never
/// touched: its snapshot is rewritten long before this.
const SNAPSHOT_RETENTION_SECONDS: u64 = 30 * 24 * 60 * 60;

/// A turn does not last a day. A touched-file list this old belongs to a
/// session that ended without reaching `Stop`.
const SESSION_RETENTION_SECONDS: u64 = 24 * 60 * 60;

fn sweep(dir: &Path, now_unix: u64, max_age: u64) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let modified = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        if now_unix.saturating_sub(modified) > max_age && std::fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    removed
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

    fn aged_file(dir: &Path, name: &str, age_seconds: u64) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, "{}").unwrap();
        // `sweep` compares against a caller-supplied now, so age is expressed
        // by moving the clock rather than by touching the file.
        let _ = age_seconds;
        path
    }

    #[test]
    fn a_snapshot_nobody_has_used_in_a_month_is_swept() {
        let base = std::env::temp_dir().join("canon-sweep-old");
        let _ = std::fs::remove_dir_all(&base);
        let snaps = base.join("snapshots");
        let old = aged_file(&snaps, "abandoned.json", 0);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        // A month in the future: everything present is now a month old.
        let removed =
            sweep(&snaps, now + SNAPSHOT_RETENTION_SECONDS + 1, SNAPSHOT_RETENTION_SECONDS);
        assert_eq!(removed, 1);
        assert!(!old.exists());
    }

    #[test]
    fn a_snapshot_in_use_is_left_alone() {
        let base = std::env::temp_dir().join("canon-sweep-fresh");
        let _ = std::fs::remove_dir_all(&base);
        let snaps = base.join("snapshots");
        let current = aged_file(&snaps, "active.json", 0);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        assert_eq!(sweep(&snaps, now, SNAPSHOT_RETENTION_SECONDS), 0);
        assert!(current.exists(), "an active repository must keep its snapshot");
    }

    #[test]
    fn a_touched_list_from_a_session_that_never_ended_is_swept() {
        // `take_touched` removes these when a turn ends. A session that is
        // killed never ends, and its list would otherwise stay forever.
        let base = std::env::temp_dir().join("canon-sweep-session");
        let _ = std::fs::remove_dir_all(&base);
        let sessions = base.join("sessions");
        let orphan = aged_file(&sessions, "abandoned.touched", 0);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        assert_eq!(
            sweep(&sessions, now + SESSION_RETENTION_SECONDS + 1, SESSION_RETENTION_SECONDS),
            1
        );
        assert!(!orphan.exists());
    }

    #[test]
    fn sweeping_a_directory_that_does_not_exist_is_not_an_error() {
        assert_eq!(sweep(Path::new("/nonexistent/canon/snapshots"), 0, 1), 0);
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
