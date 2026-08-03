//! The one place canon shells out.
//!
//! Only the commit hash, only on the cold path. The hot path never calls this:
//! spawning a process before every write is most of a 50 ms budget, and the
//! answer would be the same one the snapshot already recorded.

use std::path::Path;
use std::process::{Command, Stdio};

/// The current commit, or `None` when this is not a git repository, git is not
/// installed, or the call fails for any other reason.
///
/// A repository with no commits yet returns `None` rather than an error, which
/// is correct: there is no commit to key a snapshot on, so age becomes the only
/// freshness signal.
#[must_use]
pub(crate) fn head_sha(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?.trim().to_string();
    // Guard against a git that prints something unexpected on success.
    (sha.len() >= 7 && sha.chars().all(|c| c.is_ascii_hexdigit())).then_some(sha)
}

/// Every file git tracks, as repository-relative paths.
///
/// `-z` matters: a path containing a newline is legal on every platform canon
/// runs on, and splitting on newlines would silently corrupt the index for the
/// repository unlucky enough to contain one.
#[must_use]
pub(crate) fn tracked_files(root: &Path) -> Option<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--cached"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let files: Vec<String> =
        text.split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect();
    // A repository with no commits yet answers successfully and says nothing.
    // Treating that as "the repository contains no files" would silently
    // produce no conventions, so hand back `None` and let the caller walk.
    (!files.is_empty()).then_some(files)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_that_is_not_a_repository_yields_none() {
        let dir = std::env::temp_dir().join("canon-git-none");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(head_sha(&dir), None);
    }

    #[test]
    fn a_missing_directory_yields_none_rather_than_panicking() {
        assert_eq!(head_sha(Path::new("/nonexistent/canon/repo")), None);
    }

    #[test]
    fn a_non_repository_lists_no_tracked_files() {
        let dir = std::env::temp_dir().join("canon-git-untracked");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(tracked_files(&dir).is_none());
    }

    #[test]
    fn a_repository_with_nothing_committed_falls_back_rather_than_reporting_empty() {
        // `git init` with no commits answers successfully and lists nothing.
        // Reading that as "this repository has no files" would leave a fresh
        // checkout with no conventions and no explanation.
        let dir = std::env::temp_dir().join("canon-git-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .arg("init")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            std::fs::write(dir.join("a.rb"), "class A; end\n").unwrap();
            assert!(tracked_files(&dir).is_none(), "an empty index must not read as empty repo");
        }
    }

    #[test]
    fn a_real_repository_lists_only_what_it_tracks() {
        // The property that matters: a working tree holds build output,
        // caches and scratch files, and none of them are conventions.
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        if let Some(files) = tracked_files(here) {
            assert!(files.iter().any(|f| f.ends_with("Cargo.toml")), "got {files:?}");
            assert!(!files.iter().any(|f| f.starts_with("target/")), "target/ is not tracked");
        }
    }

    #[test]
    fn a_real_repository_yields_a_hex_sha() {
        // This workspace is one. Skipped rather than failed where it is not,
        // so a source tarball without .git still passes its own tests.
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        if let Some(sha) = head_sha(here) {
            assert!(sha.chars().all(|c| c.is_ascii_hexdigit()), "got {sha}");
            assert!(sha.len() >= 7);
        }
    }
}
