//! Walking the repository, and deciding how much each file's vote is worth.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use canon_core::Settings;
use serde::{Deserialize, Serialize};

/// Files above this are generated, vendored, or data. Parsing them costs more
/// than they inform, and their shape is nobody's convention.
///
/// Public because the write path has to honour the same bound. A file the
/// index skipped never voted on any rule, so refusing it is refusing a file
/// nothing was counted over — and a 630 KB tracked, committed service object
/// was refused on every edit by a rule it had never been allowed to break.
pub const MAX_FILE_BYTES: u64 = 512 * 1024;

/// Depth past which a tree is pathological rather than deep. Bounded so a
/// symlink loop the walker did not catch cannot run forever inside a hook.
const MAX_DEPTH: usize = 32;

/// File count past which this is not a repository, it is a disk.
///
/// Past the cap the walk gives up entirely rather than returning what it has.
/// A truncated index is worse than none: it would derive confident conventions
/// from whichever arbitrary prefix of the tree happened to be enumerated first.
///
/// Measured need: a directory holding several checkouts, one of which had a
/// 24 GB tool cache, did not finish indexing inside a minute. `SessionStart`
/// would have been killed by its own timeout with nothing to show.
const MAX_WALKED_FILES: usize = 150_000;

/// One file in the index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Repository-relative path, forward slashes on every platform.
    pub rel: String,
    /// Parent directory of `rel`, empty at the repository root.
    pub dir: String,
    /// Lowercased extension, empty when there is none.
    pub ext: String,
    /// File name without its extension.
    pub stem: String,
    /// Size in bytes.
    pub bytes: u64,
    /// How much this file's vote counts, in `0.05..=1.0`.
    pub weight: f32,
    /// Modification time, seconds since the epoch, for picking exemplars.
    pub modified_unix: u64,
}

/// Build entries for a known list of repository-relative paths.
///
/// The preferred source. Tracked files are what the repository *is*: build
/// output, caches, vendored dependencies and someone's scratch directory are
/// all in the working tree and none of them are anybody's convention.
///
/// This is not a tuning detail. Pointed at a real Rails repository, the
/// filesystem walk below found 498,419 files where git tracks 5,445, because a
/// single tool's cache directory held 909,661 of them. No hand-maintained
/// exclude list keeps up with that; the ignore rules the team already wrote
/// do.
#[must_use]
pub fn entries_for(root: &Path, settings: &Settings, rels: &[String]) -> Vec<FileEntry> {
    let now = unix_now();
    let mut out: Vec<FileEntry> = rels
        .iter()
        .filter_map(|rel| {
            let path = root.join(rel);
            let meta = std::fs::metadata(&path).ok()?;
            if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
                return None;
            }
            Some(entry_from(rel, &meta, now, settings))
        })
        .collect();
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

/// Collect every indexable file under `root` by walking the filesystem.
///
/// The fallback, for a directory that is not a git repository. Never fails: a
/// directory that cannot be read is skipped, because a hook that reports a
/// permissions problem is a hook that interrupts someone over something they
/// cannot act on mid-edit.
#[must_use]
pub fn walk(root: &Path, settings: &Settings) -> Vec<FileEntry> {
    let now = unix_now();
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];

    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            // `file_type` does not follow symlinks, which is what stops a link
            // pointing at its own ancestor from turning the walk into a loop.
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                if !settings.excludes_segment(name) {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.len() > MAX_FILE_BYTES {
                continue;
            }
            let Some(rel) = relative(root, &path) else { continue };
            out.push(entry_from(&rel, &meta, now, settings));

            if out.len() > MAX_WALKED_FILES {
                return Vec::new();
            }
        }
    }

    // Stable order regardless of filesystem enumeration order, so derivation
    // is reproducible and exemplar ties break the same way twice.
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

fn entry_from(rel: &str, meta: &std::fs::Metadata, now: u64, settings: &Settings) -> FileEntry {
    let name = rel.rsplit_once('/').map_or(rel, |(_, n)| n);
    let modified_unix = meta
        .modified()
        .ok()
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map_or(now, |d| d.as_secs());
    FileEntry {
        rel: rel.to_string(),
        dir: rel.rsplit_once('/').map_or(String::new(), |(d, _)| d.to_string()),
        ext: name.rsplit_once('.').map_or(String::new(), |(_, e)| e.to_ascii_lowercase()),
        stem: name.rsplit_once('.').map_or(name.to_string(), |(s, _)| s.to_string()),
        bytes: meta.len(),
        weight: recency_weight(now, modified_unix, settings.recency_half_life_days),
        modified_unix,
    }
}

/// How much a file's vote counts, by age.
///
/// Exponential decay on a configurable half-life. A file untouched for three
/// years still counts, at an eighth of the weight of one touched today, which
/// is the point: old files encode conventions the team has already left, and
/// letting them outvote current code is how a tool teaches a dead style.
///
/// Floored rather than allowed to reach zero, so an old but unanimous
/// directory still produces a convention instead of vanishing.
#[must_use]
pub(crate) fn recency_weight(now: u64, modified: u64, half_life_days: f32) -> f32 {
    if half_life_days <= 0.0 {
        return 1.0;
    }
    // Seconds into days as f32: a repository would need to be older than the
    // universe before the mantissa mattered, and the result feeds an
    // exponential that is then clamped.
    #[allow(clippy::cast_precision_loss)]
    let age_days = now.saturating_sub(modified) as f32 / 86_400.0;
    let decayed = 0.5_f32.powf(age_days / half_life_days);
    decayed.clamp(0.05, 1.0)
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let text = rel.to_str()?;
    Some(if std::path::MAIN_SEPARATOR == '/' {
        text.to_string()
    } else {
        text.replace(std::path::MAIN_SEPARATOR, "/")
    })
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;

    #[test]
    fn paths_are_relative_with_forward_slashes_and_split_into_parts() {
        let root = fixture::build("walk-basic", &[("app/services/create.rb", "class A; end\n")]);
        let files = walk(&root, &Settings::default());
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.rel, "app/services/create.rb");
        assert_eq!(f.dir, "app/services");
        assert_eq!(f.ext, "rb");
        assert_eq!(f.stem, "create");
    }

    #[test]
    fn a_file_at_the_root_has_an_empty_directory() {
        let root = fixture::build("walk-root", &[("Rakefile", "task :a\n")]);
        let files = walk(&root, &Settings::default());
        assert_eq!(files[0].dir, "");
        assert_eq!(files[0].stem, "Rakefile");
        assert_eq!(files[0].ext, "");
    }

    #[test]
    fn excluded_directories_are_not_walked() {
        let root = fixture::build(
            "walk-exclude",
            &[("app/a.rb", "x"), ("node_modules/pkg/b.js", "x"), ("target/debug/c.rs", "x")],
        );
        let files = walk(&root, &Settings::default());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].rel, "app/a.rb");
    }

    #[test]
    fn oversized_files_are_skipped() {
        let big = "x".repeat(usize::try_from(MAX_FILE_BYTES + 1).expect("fits"));
        let root = fixture::build("walk-big", &[("app/small.rb", "x"), ("app/huge.rb", &big)]);
        let files = walk(&root, &Settings::default());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].stem, "small");
    }

    #[test]
    fn the_walk_is_ordered_the_same_way_every_time() {
        let root = fixture::build(
            "walk-order",
            &[("b.rb", "x"), ("a.rb", "x"), ("c/d.rb", "x"), ("c/a.rb", "x")],
        );
        let s = Settings::default();
        let first: Vec<String> = walk(&root, &s).into_iter().map(|f| f.rel).collect();
        assert_eq!(first, vec!["a.rb", "b.rb", "c/a.rb", "c/d.rb"]);
        let second: Vec<String> = walk(&root, &s).into_iter().map(|f| f.rel).collect();
        assert_eq!(first, second);
    }

    #[test]
    fn agent_and_build_caches_are_excluded_by_default() {
        // Not a tuning preference: one of these reached 24 GB in a real
        // checkout and made the walk unusable.
        let settings = Settings::default();
        for cache in [".claude", ".chameleon", "node_modules", "target", ".turbo", "Pods"] {
            assert!(settings.excludes_segment(cache), "{cache} must be excluded");
        }
    }

    #[test]
    fn a_walk_past_the_cap_returns_nothing_rather_than_an_arbitrary_prefix() {
        // A truncated index would derive confident conventions from whichever
        // part of the tree happened to be enumerated first.
        let files: Vec<(String, String)> = (0..(MAX_WALKED_FILES + 10))
            .map(|i| (format!("d{}/f{i}.rb", i % 50), String::from("x")))
            .collect();
        let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let root = fixture::build("walk-cap", &refs);
        assert!(walk(&root, &Settings::default()).is_empty());
    }

    #[test]
    fn a_missing_root_yields_an_empty_index_rather_than_an_error() {
        let files = walk(Path::new("/nonexistent/canon/root"), &Settings::default());
        assert!(files.is_empty());
    }

    #[test]
    fn recency_weight_halves_over_one_half_life() {
        let now = 1_000_000_000;
        let day = 86_400;
        assert!((recency_weight(now, now, 365.0) - 1.0).abs() < 0.001);
        assert!((recency_weight(now, now - 365 * day, 365.0) - 0.5).abs() < 0.01);
        assert!((recency_weight(now, now - 730 * day, 365.0) - 0.25).abs() < 0.01);
    }

    #[test]
    fn a_very_old_file_keeps_a_floor_of_influence() {
        // Decaying to zero would let one recent file outvote a unanimous but
        // dormant directory, which is not what the repository says.
        let now = 1_000_000_000;
        let ancient = recency_weight(now, 0, 365.0);
        assert!(ancient >= 0.05, "got {ancient}");
    }

    #[test]
    fn a_file_dated_in_the_future_is_not_given_extra_weight() {
        let now = 1_000_000_000;
        assert!(recency_weight(now, now + 86_400 * 30, 365.0) <= 1.0);
    }
}
