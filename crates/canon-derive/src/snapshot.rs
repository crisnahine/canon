//! The artifact the expensive half writes and the hot path reads.
//!
//! Deriving walks the tree and parses thousands of files. Injecting happens
//! before every write and must not. The snapshot is the seam: one JSON file
//! holding the finished conventions, so `canon inject` is a read and a filter.

use std::hash::{Hash as _, Hasher as _};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use canon_core::{Convention, Settings};
use serde::{Deserialize, Serialize};

/// Bumped whenever the on-disk shape changes.
///
/// A snapshot from a different version is discarded rather than migrated. It
/// is a cache of something cheap to recompute, and migration code for a cache
/// is a permanent liability for a temporary gain.
///
/// Bumped for content, not only for shape. Freshness is decided by the commit,
/// the age and the settings, so a new binary at an unchanged commit goes on
/// serving the old binary's conventions — and version 9 held rules that later
/// versions refuse to derive. A `kebab-case` rule pinned by a vendored
/// `jquery-3.4.1.min.js` was graded `Blocking` from the stored snapshot and
/// went on refusing writes after the release that stopped deriving it.
pub const SNAPSHOT_VERSION: u32 = 15;
// 15: a directory is counted over its own files as well as over its whole
//     subtree, and the grouping now reaches eight directories deep instead of
//     four. A v14 snapshot has no rule for a directory a subdirectory of
//     another kind outvoted, and none at all below the fourth level — and it
//     holds the wider rules that were graded while those votes were diluted.
// 14: `interfaces` now derives a `shape.contract` rule for PHP, TypeScript,
//     Tsx and Rust, stating what a directory's types implement rather than
//     what they compose. A v13 snapshot has no rule for such a directory,
//     because the reading it was built from never asked the question.
// 13: Ruby class-body `include`/`extend`/`prepend` and PHP in-class trait
//     `use` are read into `mixins`, and a directory that agrees on one now
//     derives a `shape.mixin` rule. A v12 snapshot has no rule for such a
//     directory, because the reading it was built from never saw those
//     modules or traits at all.
// 12: a directory that agrees on a kind of base but not on one exact spelling
//     now derives a `shape.family` rule instead of nothing at all. A v11
//     snapshot has no rule for such a directory, because the reading it was
//     built from never asked the question.
// 11: a type records every base it names, in order, and Python resolves the
//     last positional one as its base rather than the first, with the earlier
//     ones landing in a new `mixins` list kept apart from `interfaces`. A v10
//     snapshot holds Python base rules derived while a mixin was being read as
//     the parent, and those rules were graded from a different reading of the
//     same code.
// 10: `format`, `shape.export` and `shape.namespace` are derived, and a
//      naming rule pinned by a version-numbered vendor file no longer is. A
//      v9 snapshot holds that rule, graded `Blocking`, and freshness alone
//      would go on serving it after the release that stopped deriving it.
// 9: Ruby class methods, Python dotted and generic bases, and Rust trait sets
//    are all visible now, and the file's subject is resolved differently for
//    every language whose file names are not snake_case. A snapshot from before
//    this was derived from a different reading of the same code.
// 8: conventions carry the complete set of top-level directories their sample
//    came from. Inferring it from the capped evidence was wrong in both
//    directions, and the guard written to compensate switched itself off as
//    soon as the sample spanned two directories.
// 7: a name no style accepts is excluded from naming rules rather than counted
//    against them, which changes which files voted. A snapshot from before
//    this holds rules derived while `[id].tsx` and `+page.ts` were being read
//    as style violations, and those rules are enforced.
// 6: suppression is applied after roll-up rather than before it, so a snapshot
//    from before this can hold a narrower copy of a rule the user suppressed;
//    and `languages` now names the languages the conventions came from rather
//    than every wired language with a file in the tree.
// 5: naming rules are gated on distinct names and read the root of a
//    multi-part file name, colocation no longer pairs data files, and the
//    Python, Go and Rust extractors see decorated, generic and module-nested
//    declarations. A snapshot from before this holds naming rules derived from
//    one repeated name — `Cargo.toml` ten times over — and those are enforced,
//    so keeping one would go on refusing ordinary writes until the commit
//    changed.
// 4: import and colocation conventions exist, and ERB and Vue contribute facts,
//    so a snapshot from before them is missing whole families of rule.
// 3: conventions carry an enforcement decision, rules are rolled up from
//    agreeing siblings, and the agreement a wide scope must clear went up. A
//    snapshot from before all three holds different rules and marks every one
//    of them advisory, so an upgraded install would keep refusing nothing and
//    keep the over-broad rules until the commit changed.
// 2: the query layer added call and raise facts, so snapshots built before it
//    contain no layering rules. Discarding is right: re-deriving costs seconds
//    and keeping one would leave an upgraded install running on the old
//    binary's conventions until the commit changed.

/// Size past which a file at the snapshot path is not a snapshot canon wrote.
///
/// The largest measured is 158 KB, on a 9,557-file repository with 146
/// conventions. Two orders of magnitude of headroom, and still a bound.
const MAX_SNAPSHOT_BYTES: u64 = 16 * 1024 * 1024;

/// Age past which a snapshot is rebuilt even if nothing else changed.
///
/// The commit SHA is the primary key, but a long-lived branch accumulates
/// uncommitted work, and after a day of it the conventions on disk no longer
/// describe the tree.
const MAX_AGE_SECONDS: u64 = 24 * 60 * 60;

/// A derived view of one repository at one commit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// On-disk format version.
    pub version: u32,
    /// Commit the tree was at, when it is a git repository.
    pub git_sha: Option<String>,
    /// Fingerprint of the settings used, so a config change invalidates.
    pub settings_fingerprint: u64,
    /// When this was built.
    pub generated_unix: u64,
    /// Files considered.
    pub file_count: usize,
    /// Languages that contributed parsed facts.
    pub languages: Vec<String>,
    /// Everything derived.
    pub conventions: Vec<Convention>,
}

impl Snapshot {
    /// Build from a finished derivation.
    #[must_use]
    pub fn new(
        git_sha: Option<String>,
        settings: &Settings,
        file_count: usize,
        languages: Vec<String>,
        conventions: Vec<Convention>,
    ) -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            git_sha,
            settings_fingerprint: fingerprint(settings),
            generated_unix: now(),
            file_count,
            languages,
            conventions,
        }
    }

    /// Read a snapshot, or `None` for any reason at all.
    ///
    /// Absent, unreadable, truncated, from another version: all the same
    /// outcome, because the only correct response to each is to rebuild.
    #[must_use]
    pub fn load(path: &Path) -> Option<Self> {
        // The first read every hook performs, before the config and before
        // anything else, so a FIFO here hangs all of them and a character
        // device grows until the process is killed. A snapshot has a knowable
        // size — 158 KB on a 9,557-file repository — and one larger than the
        // cap is not one canon wrote.
        let meta = std::fs::symlink_metadata(path).ok()?;
        if !meta.is_file() || meta.len() > MAX_SNAPSHOT_BYTES {
            return None;
        }
        let text = std::fs::read_to_string(path).ok()?;
        let snapshot: Self = serde_json::from_str(&text).ok()?;
        (snapshot.version == SNAPSHOT_VERSION).then_some(snapshot)
    }

    /// Write atomically.
    ///
    /// Through a temporary file and a rename, because `inject` may be reading
    /// this exact path while a session-start refresh writes it, and a half
    /// written snapshot would be parsed as no conventions at all.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }

    /// Whether this snapshot still describes the tree.
    #[must_use]
    pub fn is_fresh(&self, git_sha: Option<&str>, settings: &Settings) -> bool {
        if self.settings_fingerprint != fingerprint(settings) {
            return false;
        }
        if now().saturating_sub(self.generated_unix) > MAX_AGE_SECONDS {
            return false;
        }
        match (self.git_sha.as_deref(), git_sha) {
            (Some(a), Some(b)) => a == b,
            // Not a git repository: age is the only signal available.
            (None, None) => true,
            _ => false,
        }
    }

    /// One line for `canon check` and for the session-start manifest.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{} conventions from {} files ({})",
            self.conventions.len(),
            self.file_count,
            if self.languages.is_empty() {
                "no parsed languages".to_string()
            } else {
                self.languages.join(", ")
            }
        )
    }
}

fn fingerprint(settings: &Settings) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // Through the serialised form: adding a field to Settings then changes the
    // fingerprint without anyone remembering to update this function.
    serde_json::to_string(settings).unwrap_or_default().hash(&mut hasher);
    hasher.finish()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use canon_core::{Confidence, Enforcement, Scope};

    fn conv() -> Convention {
        Convention {
            id: "shape.public-arity.app.rb".into(),
            statement: "Types here expose exactly 1 public method".into(),
            scope: Scope::DirExt("app".into(), "rb".into()),
            confidence: Confidence::derive(47, 52).unwrap(),
            agreeing: 47,
            total: 52,
            exemplar: Some("app/a.rb".into()),
            evidence: vec![],
            sample_roots: vec![],
            enforcement: Enforcement::Advisory,
        }
    }

    fn snap() -> Snapshot {
        Snapshot::new(
            Some("abc123".into()),
            &Settings::default(),
            52,
            vec!["Ruby".into()],
            vec![conv()],
        )
    }

    #[test]
    fn a_snapshot_round_trips_through_disk() {
        let path = std::env::temp_dir().join("canon-snap-roundtrip/s.json");
        let original = snap();
        original.save(&path).expect("save");
        assert_eq!(Snapshot::load(&path).expect("load"), original);
    }

    #[test]
    fn a_snapshot_from_another_version_is_discarded_rather_than_migrated() {
        let path = std::env::temp_dir().join("canon-snap-version/s.json");
        let mut old = snap();
        old.version = SNAPSHOT_VERSION + 1;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string(&old).unwrap()).unwrap();
        assert!(Snapshot::load(&path).is_none());
    }

    #[test]
    fn a_truncated_snapshot_reads_as_absent_rather_than_as_no_conventions() {
        let path = std::env::temp_dir().join("canon-snap-truncated/s.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"version":1,"conventions":[{"id":"a""#).unwrap();
        assert!(Snapshot::load(&path).is_none());
    }

    #[test]
    fn a_missing_snapshot_is_none_rather_than_an_error() {
        assert!(Snapshot::load(Path::new("/nonexistent/canon/s.json")).is_none());
    }

    #[test]
    fn a_different_commit_makes_a_snapshot_stale() {
        let s = snap();
        assert!(s.is_fresh(Some("abc123"), &Settings::default()));
        assert!(!s.is_fresh(Some("def456"), &Settings::default()));
    }

    #[test]
    fn changing_settings_makes_a_snapshot_stale() {
        let s = snap();
        let changed = Settings { min_files: 9, ..Settings::default() };
        assert!(!s.is_fresh(Some("abc123"), &changed));
    }

    #[test]
    fn an_old_snapshot_is_stale_even_at_the_same_commit() {
        let mut s = snap();
        s.generated_unix = now().saturating_sub(MAX_AGE_SECONDS + 1);
        assert!(!s.is_fresh(Some("abc123"), &Settings::default()));
    }

    #[test]
    fn a_non_git_tree_falls_back_to_age_alone() {
        let s = Snapshot::new(None, &Settings::default(), 1, vec![], vec![]);
        assert!(s.is_fresh(None, &Settings::default()));
        assert!(!s.is_fresh(Some("abc123"), &Settings::default()), "gaining git must invalidate");
    }

    #[test]
    fn the_summary_names_the_languages_that_contributed() {
        assert_eq!(snap().summary(), "1 conventions from 52 files (Ruby)");
    }
}
