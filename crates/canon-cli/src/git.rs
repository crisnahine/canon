//! The one place canon shells out to `git`.
//!
//! The bound every spawned child runs under lives in [`crate::child`], which
//! this module used to own and now shares with the one other thing canon
//! spawns.
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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

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
    let files = nul_separated(&output);
    // A repository with no commits yet answers successfully and says nothing.
    // Reading that as "this repository has no files" would silently produce no
    // conventions, so hand back `None` and let the caller decide.
    (!files.is_empty()).then_some(files)
}

/// The paths in a NUL-separated git listing.
///
/// Lossy, the way [`log_times`] already reads its own output. A path that is
/// not valid UTF-8 is legal on Linux, and decoding the listing strictly threw
/// all of it away for one of them: the caller then walked the filesystem,
/// which is the answer this module exists to avoid, taken silently and on the
/// repositories least able to afford it. One unreadable name arrives spelled
/// with replacement characters, fails to open, and is dropped on its own.
fn nul_separated(out: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(out).split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect()
}

fn git(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    git_within(root, args, GIT_TIMEOUT)
}

/// [`git`] against a caller-supplied bound, so a test can prove the bound is
/// applied without waiting one out.
fn git_within(root: &Path, args: &[&str], timeout: Duration) -> Option<Vec<u8>> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root).args(args);
    bounded_output(&mut cmd, timeout)
}

/// How long any one git call may run before this process stops waiting on it.
///
/// Nothing in this module may block a session, and every call here can hang:
/// a `git log` over the whole history costs what a repository's age costs
/// rather than what its size costs, and `ls-files` waits on an index that a
/// network filesystem or another process holding the lock can stall
/// indefinitely. `ls_files` runs on every `reconcile`, which is the end of
/// every turn that touched a file.
///
/// The caller degrades in each case rather than failing: to the filesystem
/// mtime for a commit time, to the filesystem walk for a file list.
const GIT_TIMEOUT: Duration = Duration::from_secs(20);

/// [`crate::child::bounded`] with the answer `git` wants from it.
///
/// A `git` that exited non-zero has said nothing worth reading, so its output
/// is discarded here rather than in the helper: `curl` needs the opposite,
/// and the two callers are the reason the helper reports the status instead
/// of deciding for them.
fn bounded_output(cmd: &mut Command, timeout: Duration) -> Option<Vec<u8>> {
    crate::child::bounded(cmd, timeout, None, usize::MAX)
        .filter(|out| out.success)
        .map(|out| out.stdout)
}

/// When each tracked file was last committed, seconds since the epoch.
///
/// The filesystem mtime records when the working tree was last written, not
/// when a file last changed. Measured: a fresh clone shows one distinct mtime
/// across 400 files, and a real working checkout does better only by
/// accident, at 31 distinct values across several thousand. A recency half
/// life computed from either number weighs almost nothing, and exemplar
/// selection falls through to whichever path sorts first alphabetically.
///
/// One `git log` walk for the whole repository rather than one call per file:
/// the per-file form is a process per file and takes minutes on a large tree.
///
/// For a directory of repositories the children are asked instead and their
/// paths prefixed with their directory names, exactly as [`tracked_files`]
/// does — the two have to agree, or the entries the index is keyed by could
/// never find their own time.
///
/// `None` when git is unavailable, the repository has no commits yet, or the
/// walk does not finish inside [`GIT_TIMEOUT`]. The caller falls
/// back to mtime in every case, so a repository whose history is too large to
/// walk degrades rather than hanging a session.
#[must_use]
pub(crate) fn commit_times(root: &Path) -> Option<HashMap<String, u64>> {
    walk_commit_times(root, None)
}

/// The same, reading only the most recent [`RECENT_COMMITS`] commits.
///
/// For a caller that orders files by recency rather than weighting every one
/// of them. A file older than the cap is absent and falls back to its mtime,
/// which is what every file did before commit times existed.
#[must_use]
pub(crate) fn recent_commit_times(root: &Path) -> Option<HashMap<String, u64>> {
    walk_commit_times(root, Some(RECENT_COMMITS))
}

/// How many commits a capped walk reads.
///
/// Large enough that everything touched in the last few months of an active
/// repository is inside it, small enough that the walk is a bounded cost
/// rather than one that grows with the repository's age.
const RECENT_COMMITS: usize = 2_000;

/// One [`GIT_TIMEOUT`] budget for the whole walk, root and every
/// child combined.
///
/// `bounded_output` already stops one hung `git log`, but a directory of
/// repositories tries up to `MAX_CHILD_REPOS` children in sequence, and a
/// timeout applied fresh to each one lets the walk as a whole run for their
/// sum: up to 32 times [`GIT_TIMEOUT`] on a hook that runs at the
/// end of every turn. Striking one deadline before the first attempt and
/// spending it across every child, root included, keeps the walk's total cost
/// the same whether the repository holds one checkout or thirty-two.
fn walk_commit_times(root: &Path, limit: Option<usize>) -> Option<HashMap<String, u64>> {
    walk_commit_times_by(root, limit, Instant::now() + GIT_TIMEOUT)
}

/// [`walk_commit_times`] against a caller-supplied deadline, so a test can
/// prove the budget is shared across children without waiting out a real
/// twenty-second timeout.
fn walk_commit_times_by(
    root: &Path,
    limit: Option<usize>,
    deadline: Instant,
) -> Option<HashMap<String, u64>> {
    if let Some(times) = log_times(root, limit, deadline) {
        return Some(times);
    }
    let mut combined: HashMap<String, u64> = HashMap::new();
    for (name, path) in child_repos(root) {
        // Checked before spawning rather than left to `bounded_output`: once
        // the shared deadline has passed, a child gets no attempt at all
        // instead of a share of a budget that is already zero.
        if Instant::now() >= deadline {
            break;
        }
        if let Some(times) = log_times(&path, limit, deadline) {
            combined.extend(times.into_iter().map(|(rel, at)| (format!("{name}/{rel}"), at)));
        }
    }
    (!combined.is_empty()).then_some(combined)
}

/// The walk for one repository, over the whole log or the newest `limit`
/// commits.
///
/// `-z` matters for the same reason it does in [`ls_files`]: a path containing
/// a newline is legal on every platform canon runs on, and splitting the log
/// on newlines records two paths that do not exist and loses the one that
/// does.
///
/// Empty is `None`, so a root that is a repository with nothing to say hands
/// the question to its children rather than answering for them.
fn log_times(root: &Path, limit: Option<usize>, deadline: Instant) -> Option<HashMap<String, u64>> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return None;
    }
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root).args(["log", "--no-renames", "-z", "--format=%ct", "--name-only"]);
    if let Some(n) = limit {
        cmd.arg(format!("-n{n}"));
    }
    // Commits that touched nothing under `root` are not this index's history,
    // and reading them spends a capped walk's budget on files it will discard.
    cmd.args(["--", "."]);
    let out = bounded_output(&mut cmd, remaining)?;
    let times = rebase_on_cwd(parse_commit_times(&String::from_utf8_lossy(&out)), root)?;
    (!times.is_empty()).then_some(times)
}

/// Respell top-relative log paths the way `ls_files` spells them.
///
/// `ls-files` answers relative to the directory git runs in and
/// `log --name-only` relative to the repository top, and both feed one map
/// keyed one way and looked up the other. A session started in `crates/cli/`
/// therefore missed on every file, and a non-empty map of unusable keys is
/// worse than none: it clamped every file's mtime to a single value.
///
/// `None` when the prefix cannot be read, so the caller falls back to mtime
/// rather than keying the index on a basis it could not confirm.
fn rebase_on_cwd(times: HashMap<String, u64>, root: &Path) -> Option<HashMap<String, u64>> {
    let raw = git(root, &["rev-parse", "--show-prefix"])?;
    // A trailing newline only; a directory name may legitimately end in a
    // space, and `--show-prefix` writes the path unquoted.
    let prefix = String::from_utf8(raw).ok()?.trim_end_matches('\n').to_string();
    if prefix.is_empty() {
        return Some(times);
    }
    Some(
        times
            .into_iter()
            .filter_map(|(rel, at)| rel.strip_prefix(&prefix).map(|r| (r.to_string(), at)))
            .collect(),
    )
}

/// Turn `git log -z --no-renames --format=%ct --name-only` output into each
/// path's most recent commit time.
///
/// The log walks newest first, so the first time a path appears is its most
/// recent commit. `entry().or_insert()` is load-bearing here: a plain
/// `insert` would let a later, older record overwrite the newer one and
/// backdate every file to its oldest commit, which is worse than the mtime
/// this replaces.
///
/// A record that reads as an integer is a commit stamp. A file whose entire
/// name is a decimal integer would be read as one; the alternative is
/// trusting the newline git writes between a commit's header and its name
/// list, and that newline is exactly what `-z` exists to stop this from
/// trusting.
fn parse_commit_times(text: &str) -> HashMap<String, u64> {
    let mut times: HashMap<String, u64> = HashMap::new();
    let mut current: u64 = 0;
    let mut after_stamp = false;
    for record in text.split('\0') {
        // `-z` makes NUL the separator, but git still writes the newline that
        // divides a commit's header from its name list, and it arrives glued
        // to the first path. It belongs to the separator, and it is stripped
        // only there: a path may legitimately begin with a newline, which is
        // the whole reason this reads NUL-separated output.
        let record = if after_stamp { record.strip_prefix('\n').unwrap_or(record) } else { record };
        after_stamp = false;
        if record.is_empty() {
            continue;
        }
        if let Ok(stamp) = record.parse::<u64>() {
            current = stamp;
            after_stamp = true;
            continue;
        }
        times.entry(record.to_string()).or_insert(current);
    }
    times
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
    use std::process::Stdio;

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

    #[test]
    fn the_first_record_for_a_path_wins_because_the_log_walks_newest_first() {
        // git log walks newest first, so a's first appearance below is its most
        // recent commit. An `insert` here instead of `entry().or_insert()`
        // would let the second, older record overwrite it and backdate a.rb to
        // its oldest commit, which is worse than the mtime this replaces.
        let text = "1700000000\0\na.rb\0\u{0}1000000000\0\na.rb\0b.rb\0";
        let times = parse_commit_times(text);
        assert_eq!(times.get("a.rb"), Some(&1_700_000_000));
        assert_eq!(times.get("b.rb"), Some(&1_000_000_000));
    }

    #[test]
    fn empty_records_between_commits_are_not_mistaken_for_paths() {
        let text = "1700000000\0\na.rb\0\0";
        let times = parse_commit_times(text);
        assert_eq!(times.len(), 1);
        assert_eq!(times.get("a.rb"), Some(&1_700_000_000));
    }

    #[test]
    fn a_path_containing_a_newline_survives_the_walk() {
        // Why the walk is NUL-separated, the same reason `ls_files` is: a path
        // with a newline in it is legal on every platform canon runs on, and
        // splitting the log on newlines recorded two paths that do not exist
        // and lost the one that does.
        let text = "1700000000\0\nweird\nname.rb\0plain.rb\0";
        let times = parse_commit_times(text);
        assert_eq!(times.get("weird\nname.rb"), Some(&1_700_000_000));
        assert_eq!(times.get("plain.rb"), Some(&1_700_000_000));
    }

    #[test]
    fn commit_times_of_a_non_repository_is_none() {
        let dir = std::env::temp_dir().join("canon-git-commit-times-none");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(commit_times(&dir).is_none());
    }

    #[test]
    fn a_directory_of_repositories_has_commit_times_from_its_children() {
        // The workspace layout again. `head_sha` and `tracked_files` both
        // answer for it through its children, and this answered `None`, so
        // every file in every checkout silently fell back to its mtime — one
        // distinct value across a fresh clone.
        let root = std::env::temp_dir().join("canon-git-mono-times");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let built = child_repo(&root, "api", &[("app/services/create.rb", "class A; end\n")])
            && child_repo(&root, "client", &[("src/App.tsx", "export const A = () => 1;\n")]);
        if !built {
            return; // no usable git here
        }
        assert!(rev_parse(&root).is_none(), "precondition: the root is not a repository");

        let times = commit_times(&root).expect("the children answer for it");
        // Prefixed the way `tracked_files` prefixes them, or the entries the
        // index is keyed by would never find their own time.
        assert!(times.contains_key("api/app/services/create.rb"), "got {times:?}");
        assert!(times.contains_key("client/src/App.tsx"), "got {times:?}");
    }

    #[test]
    fn a_session_started_below_the_repository_top_still_has_commit_times() {
        // `ls-files` answers relative to the directory it runs in and
        // `log --name-only` relative to the repository top. Nothing resolved
        // the top level, so an index built from a subdirectory keyed every
        // entry one way and looked it up the other: every lookup missed, and
        // the non-empty map that resulted clamped every file's mtime to a
        // single value instead.
        let root = std::env::temp_dir().join("canon-git-subdir-times");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let built = child_repo(
            &root,
            "api",
            &[("app/services/create.rb", "class A; end\n"), ("Rakefile", "task :a\n")],
        );
        if !built {
            return; // no usable git here
        }

        let sub = root.join("api/app");
        let files = tracked_files(&sub).expect("a subdirectory lists its own files");
        assert_eq!(files, vec!["services/create.rb".to_string()]);

        let times = commit_times(&sub).expect("and has a time for each of them");
        assert!(times.contains_key("services/create.rb"), "got {times:?}");
        // And nothing from outside the subdirectory, which `ls-files` would
        // never have named and so could never look up.
        assert!(!times.keys().any(|k| k.contains("Rakefile")), "got {times:?}");
    }

    #[test]
    fn a_capped_walk_reads_only_the_commits_it_asked_for() {
        // What the write path pays for. The full walk's cost scales with how
        // long a repository has existed, and `reconcile` runs at the end of
        // every turn that touched a file.
        let root = std::env::temp_dir().join("canon-git-capped");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        if !child_repo(&root, "api", &[("old.rb", "class A; end\n")]) {
            return;
        }
        let repo = root.join("api");
        std::fs::write(repo.join("new.rb"), "class B; end\n").unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        if !(run(&["add", "-A"]) && run(&["commit", "-m", "second"])) {
            return;
        }

        let deadline = Instant::now() + GIT_TIMEOUT;
        let full = log_times(&repo, None, deadline).expect("a full walk");
        assert!(full.contains_key("old.rb") && full.contains_key("new.rb"), "got {full:?}");

        let capped = log_times(&repo, Some(1), deadline).expect("a capped walk");
        assert!(capped.contains_key("new.rb"), "got {capped:?}");
        assert!(!capped.contains_key("old.rb"), "the cap read more than it asked for: {capped:?}");
    }

    #[test]
    fn a_deadline_already_past_stops_the_walk_before_any_child_is_tried() {
        // `child_repos` allows up to 32, so a timeout applied fresh to each
        // one lets the walk run for their sum instead of one shared bound. A
        // deadline that has already passed is the deterministic way to prove
        // the budget is aggregate: two real, fast, otherwise-answerable
        // repositories must still both go untried, because a per-child
        // timeout would have let at least the first one run.
        let root = std::env::temp_dir().join("canon-git-deadline-past");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let built = child_repo(&root, "api", &[("a.rb", "class A; end\n")])
            && child_repo(&root, "client", &[("b.ts", "export const b = 1;\n")]);
        if !built {
            return; // no usable git here
        }
        let expired = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        assert!(
            walk_commit_times_by(&root, None, expired).is_none(),
            "a deadline already past must not let any child run"
        );
    }

    #[test]
    fn log_times_spends_a_shared_deadline_rather_than_a_fresh_timeout_per_call() {
        // The mechanism the aggregate bound rests on: before this, every call
        // measured its own twenty seconds from `GIT_TIMEOUT`, so an
        // already-late call in a long walk still got a full fresh allowance.
        // A repository real and fast enough to answer inside any ordinary
        // timeout must still come back empty once the shared deadline it is
        // handed has already passed.
        let parent = std::env::temp_dir();
        let name = "canon-git-log-times-expired";
        let _ = std::fs::remove_dir_all(parent.join(name));
        if !child_repo(&parent, name, &[("a.rb", "class A; end\n")]) {
            return;
        }
        let expired = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        assert!(
            log_times(&parent.join(name), None, expired).is_none(),
            "an expired deadline must not grant a fresh per-call timeout"
        );
    }

    #[test]
    fn commit_times_of_a_real_repository_names_a_tracked_file_with_a_plausible_stamp() {
        // This workspace is one. Skipped rather than failed where it is not.
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        if let Some(times) = commit_times(here) {
            let cargo_toml =
                times.iter().find(|(rel, _)| rel.ends_with("Cargo.toml")).map(|(_, t)| *t);
            if let Some(stamp) = cargo_toml {
                // Newer than git's own 2005 origin, older than the moment this
                // assertion runs: a sanity bound, not a precise one.
                assert!(stamp > 1_100_000_000, "got {stamp}");
                assert!(stamp <= now_unix(), "got {stamp}");
            }
        }
    }

    fn now_unix() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap()
    }

    #[test]
    fn one_path_that_is_not_utf8_does_not_discard_the_tracked_list() {
        // A path that is not valid UTF-8 is legal on Linux. Decoding the
        // listing strictly threw the whole of it away for one of them, and the
        // caller then walked the filesystem — the answer this module exists to
        // avoid, taken silently and only on the repositories least able to
        // afford it.
        let files = nul_separated(b"a.rb\0bad\xff\xfename.rb\0b.rb\0");
        assert_eq!(files.len(), 3, "got {files:?}");
        assert!(files.contains(&"a.rb".to_string()), "got {files:?}");
        assert!(files.contains(&"b.rb".to_string()), "got {files:?}");
    }

    #[test]
    fn a_git_call_is_bounded_rather_than_run_to_completion() {
        // `ls_files` runs on every `reconcile`, which is the end of every turn
        // that touched a file, and `Command::output` waits on its child with
        // no bound at all. A timeout that cannot elapse is the deterministic
        // way to prove the bound is applied: the same call answers with a real
        // one.
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        if git_within(here, &["rev-parse", "HEAD"], GIT_TIMEOUT).is_none() {
            return; // no usable git here
        }
        assert!(
            git_within(here, &["rev-parse", "HEAD"], Duration::ZERO).is_none(),
            "a git call ran past the bound it was given"
        );
    }
}
