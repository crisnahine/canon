//! Near-duplicate detection: the DRY half.
//!
//! Scoped deliberately narrowly. The realistic failure is copying the sibling
//! file and editing three lines, not rediscovering an identical algorithm four
//! directories away. Comparing against siblings only keeps this cheap enough
//! to run inside a hook and keeps the false positive rate low, which matters
//! more here than anywhere else: duplication is the one signal where being
//! wrong is actively insulting.
//!
//! The method is line shingling. Lines are normalised, grouped into
//! overlapping windows, and hashed. Two files sharing many windows share
//! structure regardless of how the identifiers were renamed.

use std::collections::HashSet;
use std::hash::{Hash as _, Hasher as _};
use std::path::Path;

use crate::walk::FileEntry;

/// Lines per window. Four is long enough that boilerplate does not match by
/// accident and short enough to survive an edited line in the middle.
const WINDOW: usize = 4;

/// Below this many shared windows, the overlap is imports and closing braces.
const MIN_SHARED: usize = 3;

/// Overlap ratio at which a file is worth mentioning.
const MIN_RATIO: f32 = 0.30;

/// Sibling files compared against. A directory larger than this is a bucket
/// rather than a module, and reading all of it inside a hook is not worth it.
const MAX_SIBLINGS: usize = 200;

/// One file that substantially overlaps the one just written.
#[derive(Debug, Clone, PartialEq)]
pub struct DuplicateHit {
    /// The existing file.
    pub rel: String,
    /// How many normalised windows the two share.
    pub shared_windows: usize,
    /// Share of the new file's windows that already exist there.
    pub ratio: f32,
}

impl DuplicateHit {
    /// One line, phrased as an observation rather than an accusation.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn render(&self) -> String {
        format!("{}% of this file already exists in {}", (self.ratio * 100.0) as usize, self.rel)
    }
}

/// Find existing files that substantially overlap `source`.
///
/// Compares against files in the same directory with the same extension, most
/// recently modified first.
#[must_use]
pub fn duplicates_against_siblings(
    root: &Path,
    target_rel: &str,
    source: &str,
    index: &[FileEntry],
) -> Vec<DuplicateHit> {
    let target_windows = windows(source);
    if target_windows.len() < MIN_SHARED {
        return Vec::new();
    }

    let (dir, ext) = split(target_rel);
    let mut siblings: Vec<&FileEntry> =
        index.iter().filter(|f| f.rel != target_rel && f.dir == dir && f.ext == ext).collect();
    siblings.sort_by_key(|f| std::cmp::Reverse(f.modified_unix));
    siblings.truncate(MAX_SIBLINGS);

    let mut hits: Vec<DuplicateHit> = siblings
        .into_iter()
        .filter_map(|f| {
            let other = std::fs::read_to_string(root.join(&f.rel)).ok()?;
            let shared = target_windows.intersection(&windows(&other)).count();
            if shared < MIN_SHARED {
                return None;
            }
            #[allow(clippy::cast_precision_loss)]
            let ratio = shared as f32 / target_windows.len() as f32;
            (ratio >= MIN_RATIO).then_some(DuplicateHit {
                rel: f.rel.clone(),
                shared_windows: shared,
                ratio,
            })
        })
        .collect();

    hits.sort_by(|a, b| {
        b.ratio.partial_cmp(&a.ratio).unwrap_or(std::cmp::Ordering::Equal).then(a.rel.cmp(&b.rel))
    });
    hits
}

fn split(rel: &str) -> (String, String) {
    let dir = rel.rsplit_once('/').map_or(String::new(), |(d, _)| d.to_string());
    let name = rel.rsplit_once('/').map_or(rel, |(_, n)| n);
    let ext = name.rsplit_once('.').map_or(String::new(), |(_, e)| e.to_ascii_lowercase());
    (dir, ext)
}

/// Hashed windows of normalised lines.
fn windows(source: &str) -> HashSet<u64> {
    let lines: Vec<String> = source.lines().filter_map(normalise).collect();
    if lines.len() < WINDOW {
        return HashSet::new();
    }
    lines
        .windows(WINDOW)
        .map(|w| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            w.hash(&mut hasher);
            hasher.finish()
        })
        .collect()
}

/// Collapse a line to its structure, or drop it.
///
/// Blank lines, comments and lone delimiters are dropped: they are identical
/// across every file in the language and would make everything look like a
/// copy of everything else.
fn normalise(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("//")
        || trimmed.starts_with('#')
        || trimmed.starts_with("/*")
        || trimmed.starts_with('*')
    {
        return None;
    }
    if trimmed.chars().all(|c| matches!(c, '}' | ')' | ']' | '{' | '(' | '[' | ';' | ',')) {
        return None;
    }
    if matches!(trimmed, "end" | "else" | "endif" | "fi" | "done") {
        return None;
    }
    Some(trimmed.split_whitespace().collect::<Vec<_>>().join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use crate::walk::walk;
    use canon_core::Settings;

    const BODY: &str = "\
class PayoutProcessor
  def call
    validate_account
    compute_fees
    transfer_funds
    notify_customer
    record_audit_entry
  end
end
";

    fn index_of(root: &Path) -> Vec<FileEntry> {
        walk(root, &Settings::default())
    }

    #[test]
    fn a_copied_sibling_is_found() {
        let root = fixture::build(
            "dup-copy",
            &[("app/a.rb", BODY), ("app/b.rb", "class Other\n  def x\n    1\n  end\nend\n")],
        );
        let hits = duplicates_against_siblings(&root, "app/new.rb", BODY, &index_of(&root));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].rel, "app/a.rb");
        assert!(hits[0].ratio > 0.9, "got {}", hits[0].ratio);
    }

    #[test]
    fn an_unrelated_file_is_not_reported() {
        let root = fixture::build(
            "dup-unrelated",
            &[("app/a.rb", "class Other\n  def x\n    compute\n    render\n  end\nend\n")],
        );
        assert!(
            duplicates_against_siblings(&root, "app/new.rb", BODY, &index_of(&root)).is_empty()
        );
    }

    #[test]
    fn renaming_the_class_does_not_hide_the_copy() {
        let renamed = BODY.replace("PayoutProcessor", "RefundProcessor");
        let root = fixture::build("dup-renamed", &[("app/a.rb", BODY)]);
        let hits = duplicates_against_siblings(&root, "app/new.rb", &renamed, &index_of(&root));
        assert_eq!(hits.len(), 1, "one renamed line must not hide six identical ones");
    }

    #[test]
    fn shared_boilerplate_alone_is_not_duplication() {
        // Imports and closing delimiters are identical in every file. If these
        // counted, every file would duplicate every other file.
        let a = "import React from 'react';\nimport { z } from 'zod';\n\nexport const A = () => {\n  return 1;\n};\n";
        let b = "import React from 'react';\nimport { z } from 'zod';\n\nexport const B = () => {\n  return computeSomethingEntirelyDifferent();\n};\n";
        let root = fixture::build("dup-boilerplate", &[("src/a.ts", a)]);
        let hits = duplicates_against_siblings(&root, "src/b.ts", b, &index_of(&root));
        assert!(hits.is_empty(), "got {hits:?}");
    }

    #[test]
    fn comments_do_not_make_two_files_look_alike() {
        let a = "# same comment\n# same comment\n# same comment\n# same comment\nputs 1\n";
        let b = "# same comment\n# same comment\n# same comment\n# same comment\nputs 2\n";
        let root = fixture::build("dup-comments", &[("app/a.rb", a)]);
        assert!(duplicates_against_siblings(&root, "app/b.rb", b, &index_of(&root)).is_empty());
    }

    #[test]
    fn a_file_is_never_compared_against_itself() {
        let root = fixture::build("dup-self", &[("app/a.rb", BODY)]);
        assert!(duplicates_against_siblings(&root, "app/a.rb", BODY, &index_of(&root)).is_empty());
    }

    #[test]
    fn only_same_extension_siblings_are_compared() {
        let root = fixture::build("dup-ext", &[("app/a.py", BODY)]);
        assert!(
            duplicates_against_siblings(&root, "app/new.rb", BODY, &index_of(&root)).is_empty()
        );
    }

    #[test]
    fn a_short_file_is_never_flagged() {
        let root = fixture::build("dup-short", &[("app/a.rb", "puts 1\n")]);
        assert!(
            duplicates_against_siblings(&root, "app/b.rb", "puts 1\n", &index_of(&root)).is_empty()
        );
    }

    /// Long enough that dropping two lines still leaves an overlap above the
    /// floor. `BODY` normalises to seven lines, so removing two would leave
    /// two windows and be filtered as noise, which is correct behaviour but
    /// says nothing about ordering.
    const LONG_BODY: &str = "\
class PayoutProcessor
  def call
    validate_account
    compute_fees
    transfer_funds
    notify_customer
    record_audit_entry
    reconcile_ledger
    emit_metrics
    schedule_followup
  end
end
";

    #[test]
    fn hits_are_ordered_by_how_much_they_overlap() {
        let partial =
            LONG_BODY.replace("    schedule_followup\n", "").replace("    emit_metrics\n", "");
        let root = fixture::build(
            "dup-order",
            &[("app/exact.rb", LONG_BODY), ("app/partial.rb", &partial)],
        );
        let hits = duplicates_against_siblings(&root, "app/new.rb", LONG_BODY, &index_of(&root));
        assert_eq!(hits.len(), 2, "got {hits:?}");
        assert_eq!(hits[0].rel, "app/exact.rb");
        assert!(hits[0].ratio >= hits[1].ratio);
    }

    #[test]
    fn an_overlap_of_only_two_windows_is_below_the_noise_floor() {
        // Documents the constant the ordering test had to work around.
        let short =
            BODY.replace("    notify_customer\n", "").replace("    record_audit_entry\n", "");
        let root = fixture::build("dup-floor", &[("app/a.rb", BODY)]);
        assert!(
            duplicates_against_siblings(&root, "app/new.rb", &short, &index_of(&root)).is_empty()
        );
    }

    #[test]
    fn the_rendered_line_reads_as_an_observation() {
        let hit = DuplicateHit { rel: "app/a.rb".into(), shared_windows: 5, ratio: 0.83 };
        assert_eq!(hit.render(), "83% of this file already exists in app/a.rb");
    }
}
