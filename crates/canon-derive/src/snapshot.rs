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
pub const SNAPSHOT_VERSION: u32 = 1;

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
