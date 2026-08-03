//! The one place canon shells out.
//!
//! Only on the cold path. The hot path never calls this: spawning a process
//! before every write is most of a 50 ms budget, and the answer would be the
//! one the snapshot already recorded.
//!
//! # Directories that hold repositories
//!
//! A workspace root is often not a repository itself. Opening an editor at a
//! folder containing `api/`, `client/` and `wordpress/`, each its own checkout,
//! is an ordinary way to work on a system rather than a service.
//!
//! Treating that as "not a git repository" and walking the filesystem is not a
//! degraded answer, it is an unusable one: measured on such a folder, one
//! child's tool-cache directory held 24 GB and indexing did not finish inside a
//! minute. So when the root is not a repository, its immediate children are
//! asked instead and their answers are combined under their directory names.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Child directories consulted when the root is not itself a repository.
///
/// One level only, and bounded. Deeper scanning turns a wrong guess about the
/// root into an unbounded search, which is the failure this exists to avoid.
const MAX_CHILD_REPOS: usize = 32;

/// The current commit.
///
/// For a directory of repositories this is the children's commits joined, so a
/// commit in any one of them invalidates the snapshot. The value is only ever
/// compared for equality, never parsed.
///
/// `None` when nothing here is a repository, git is not installed, or there are
/// no commits yet. Age then becomes the only freshness signal.
#[must_use]
pub(crate) fn head_sha(root: &Path) -> Option<String> {
    if let Some(sha) = rev_parse(root) {
        return Some(sha);
    }
    let combined: Vec<String> = child_repos(root)
        .into_iter()
        .filter_map(|(name, path)| rev_parse(&path).map(|sha| format!("{name}:{sha}")))
        .collect();
    (!combined.is_empty()).then(|| combined.join(","))
}

/// Every file git tracks, as paths relative to `root`.
///
/// For a directory of repositories, each child's files are prefixed with its
/// directory name, so `app/services/x.rb` in `api/` arrives as
/// `api/app/services/x.rb` and every scope below behaves exactly as it would
/// in a single repository.
#[must_use]
pub(crate) fn tracked_files(root: &Path) -> Option<Vec<String>> {
    if let Some(files) = ls_files(root) {
        return Some(files);
    }
    let mut combined = Vec::new();
    for (name, path) in child_repos(root) {
        if let Some(files) = ls_files(&path) {
            combined.extend(files.into_iter().map(|f| format!("{name}/{f}")));
        }
    }
    (!combined.is_empty()).then_some(combined)
}

fn rev_parse(root: &Path) -> Option<String> {
    let output = git(root, &["rev-parse", "HEAD"])?;
    let sha = String::from_utf8(output).ok()?.trim().to_string();
    // Guard against a git that prints something unexpected on success.
    (sha.len() >= 7 && sha.chars().all(|c| c.is_ascii_hexdigit())).then_some(sha)
}

/// `-z` matters: a path containing a newline is legal on every platform canon
/// runs on, and splitting on newlines would silently corrupt the index for the
/// repository unlucky enough to contain one.
fn ls_files(root: &Path) -> Option<Vec<String>> {
    let output = git(root, &["ls-files", "-z", "--cached"])?;
    let text = String::from_utf8(output).ok()?;
    let files: Vec<String> =
        text.split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect();
    // A repository with no commits yet answers successfully and says nothing.
    // Reading that as "this repository has no files" would silently produce no
    // conventions, so hand back `None` and let the caller decide.
    (!files.is_empty()).then_some(files)
}

fn git(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// Immediate child directories that are themselves repositories.
///
/// Sorted, so a combined index and a combined commit string are reproducible
/// rather than dependent on filesystem enumeration order.
fn child_repos(root: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut found: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            if name.starts_with('.') {
                return None;
            }
            let path = e.path();
            // `.git` is a directory in a normal clone and a file in a worktree
            // or submodule, so test for existence rather than for a directory.
            path.join(".git").exists().then_some((name, path))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found.truncate(MAX_CHILD_REPOS);
    found
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

    /// Build `root/<name>/` as a committed repository holding `files`.
    fn child_repo(root: &Path, name: &str, files: &[(&str, &str)]) -> bool {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for (rel, body) in files {
            let path = dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        run(&["init"])
            && run(&["config", "user.email", "t@example.com"])
            && run(&["config", "user.name", "t"])
            && run(&["add", "-A"])
            && run(&["commit", "-m", "init"])
    }

    #[test]
    fn a_directory_of_repositories_is_indexed_through_its_children() {
        // The workspace layout: a folder holding several checkouts, itself not
        // a repository. Walking it instead is not a degraded answer, it is an
        // unusable one.
        let root = std::env::temp_dir().join("canon-git-mono");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let built = child_repo(&root, "api", &[("app/services/create.rb", "class A; end\n")])
            && child_repo(&root, "client", &[("src/App.tsx", "export const A = () => 1;\n")]);
        if !built {
            return; // no usable git here
        }

        assert!(rev_parse(&root).is_none(), "precondition: the root is not a repository");

        let files = tracked_files(&root).expect("children answer for it");
        assert!(files.contains(&"api/app/services/create.rb".to_string()), "got {files:?}");
        assert!(files.contains(&"client/src/App.tsx".to_string()), "got {files:?}");
    }

    #[test]
    fn a_directory_of_repositories_has_a_commit_string_that_changes_with_any_child() {
        let root = std::env::temp_dir().join("canon-git-mono-sha");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        if !child_repo(&root, "api", &[("a.rb", "class A; end\n")]) {
            return;
        }

        let before = head_sha(&root).expect("a combined commit string");
        assert!(before.starts_with("api:"), "got {before}");

        assert!(child_repo(&root, "client", &[("b.ts", "export const b = 1;\n")]));
        let after = head_sha(&root).expect("a combined commit string");
        assert_ne!(before, after, "adding a checkout must invalidate the snapshot");
    }

    #[test]
    fn a_directory_with_no_repositories_below_it_yields_nothing() {
        let root = std::env::temp_dir().join("canon-git-mono-empty");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("just-a-folder")).unwrap();
        std::fs::write(root.join("just-a-folder/a.rb"), "class A; end\n").unwrap();
        assert!(tracked_files(&root).is_none());
        assert!(head_sha(&root).is_none());
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
