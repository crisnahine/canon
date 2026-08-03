//! Turning a repository into a set of conventions, and a path into the subset
//! that applies to it.
//!
//! # Two tiers
//!
//! Tier 0 needs no grammar. Naming style, file size, test layout and canonical
//! exemplars come from paths and byte counts, so they work on any text
//! repository in any language on the day it is installed.
//!
//! Tier 1 needs a parser. Public surface shape, entrypoint naming and base
//! classes cannot be derived without knowing what each language means by
//! *public*, which is why [`canon_extract`] is a separate layer.
//!
//! # The cost model
//!
//! Deriving is expensive and happens once per session. Selecting is cheap and
//! happens before every write. They are separate functions over a persisted
//! [`Snapshot`] for exactly that reason: the hot path reads one JSON file and
//! filters a few hundred conventions. It never walks the tree, parses a file,
//! or spawns a process.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::float_cmp,
        clippy::too_many_lines
    )
)]

mod dup;
mod naming;
mod render;
mod select;
mod semantic;
mod snapshot;
mod tier0;
mod verify;
mod walk;

pub use dup::{DuplicateHit, duplicates_against_siblings};
pub use naming::Style;
pub use render::render_block;
pub use select::for_path;
pub use snapshot::{SNAPSHOT_VERSION, Snapshot};
pub use verify::{Violation, blocking_violations, verify_source};
pub use walk::{FileEntry, entries_for, walk};

use canon_core::{Convention, Settings};

/// Walk `root` and derive every convention it supports.
///
/// The expensive half. Runs once per session, off the hot path.
#[must_use]
pub fn derive_all(
    root: &std::path::Path,
    settings: &Settings,
) -> (Vec<FileEntry>, Vec<Convention>) {
    let files = walk::walk(root, settings);
    let conventions = derive_from(root, settings, &files);
    (files, conventions)
}

/// Derive from an index someone else assembled.
///
/// Separate from [`derive_all`] so the caller chooses where the file list comes
/// from. The binary asks git, which is both faster and more correct than any
/// walk; this crate stays free of subprocesses so its rules stay testable
/// without one on `PATH`.
#[must_use]
pub fn derive_from(
    root: &std::path::Path,
    settings: &Settings,
    files: &[FileEntry],
) -> Vec<Convention> {
    let mut conventions = tier0::derive(files, settings);
    let facts = semantic::gather(files, root);
    conventions.extend(semantic::derive(&facts, settings));
    conventions.retain(|c| !settings.is_suppressed(&c.id));
    collapse_redundant(&mut conventions);
    // Deterministic order, so an unchanged tree produces a byte-identical
    // snapshot and cache staleness stays observable.
    conventions.sort_by(|a, b| a.id.cmp(&b.id));
    conventions
}

/// Drop a rule that an ancestor already states in the same words.
///
/// Rules are derived at every ancestor directory so that a new folder inherits
/// something, and the cost is repetition: a repository that is `snake_case`
/// throughout produces the identical sentence at every level. On a real Rails
/// codebase that was 147 naming rules where 6 carry all the information.
///
/// Only an ancestor makes a rule redundant. Two unrelated directories that
/// happen to share a sentence each speak for their own files, and dropping
/// either would leave those files with no rule at all.
fn collapse_redundant(conventions: &mut Vec<Convention>) {
    let keyed: Vec<(String, String, String)> =
        conventions.iter().map(|c| (c.statement.clone(), scope_ext(c), scope_dir(c))).collect();

    // `is_ancestor` is false for equal directories, so a rule never eliminates
    // itself and no index bookkeeping is needed to skip it.
    let keep: Vec<bool> = keyed
        .iter()
        .map(|(statement, ext, dir)| {
            !keyed.iter().any(|(other_statement, other_ext, other_dir)| {
                other_statement == statement && other_ext == ext && is_ancestor(other_dir, dir)
            })
        })
        .collect();

    let mut decisions = keep.into_iter();
    conventions.retain(|_| decisions.next().unwrap_or(true));
}

fn scope_ext(c: &Convention) -> String {
    match &c.scope {
        canon_core::Scope::Ext(e) | canon_core::Scope::DirExt(_, e) => e.clone(),
        canon_core::Scope::Repo | canon_core::Scope::Dir(_) => String::new(),
    }
}

fn scope_dir(c: &Convention) -> String {
    match &c.scope {
        canon_core::Scope::Dir(d) | canon_core::Scope::DirExt(d, _) => d.clone(),
        canon_core::Scope::Repo | canon_core::Scope::Ext(_) => String::new(),
    }
}

/// Whether `ancestor` strictly contains `descendant`.
///
/// The empty directory is the repository root and contains everything. Path
/// boundaries are respected, so `app/service` does not contain
/// `app/services`.
fn is_ancestor(ancestor: &str, descendant: &str) -> bool {
    if ancestor == descendant {
        return false;
    }
    ancestor.is_empty() || descendant.starts_with(&format!("{ancestor}/"))
}

#[cfg(test)]
pub(crate) mod fixture {
    use std::fs;
    use std::path::PathBuf;

    /// Build a throwaway repository on disk and return its root.
    pub(crate) fn build(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("canon-fixture-{name}"));
        let _ = fs::remove_dir_all(&root);
        for (rel, body) in files {
            let path = root.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("mkdir");
            }
            fs::write(&path, body).expect("write");
        }
        root
    }

    /// `n` sibling files sharing a body template, for agreement tests.
    pub(crate) fn agreeing(dir: &str, ext: &str, n: usize, body: &str) -> Vec<(String, String)> {
        (0..n)
            .map(|i| (format!("{dir}/item{i}.{ext}"), body.replace("$N", &i.to_string())))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(v: &[(String, String)]) -> Vec<(&str, &str)> {
        v.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect()
    }

    #[test]
    fn a_repository_of_agreeing_service_objects_yields_the_expected_conventions() {
        let files = fixture::agreeing(
            "app/services/enrolments",
            "rb",
            6,
            "class Item$N < ApplicationService\n  def call\n    :ok\n  end\n\n  private\n\n  def helper\n  end\nend\n",
        );
        let root = fixture::build("services", &refs(&files));

        let (entries, convs) = derive_all(&root, &Settings::default());
        assert_eq!(entries.len(), 6);

        let joined = convs.iter().map(|c| c.statement.as_str()).collect::<Vec<_>>().join(" | ");
        assert!(joined.contains("exactly 1 public method"), "got: {joined}");
        assert!(joined.contains("`call`"), "got: {joined}");
        assert!(joined.contains("ApplicationService"), "got: {joined}");
    }

    #[test]
    fn a_split_repository_yields_no_shape_convention() {
        // Five files with one public method, five with four. There is no
        // convention here, and inventing one is worse than silence.
        let mut files =
            fixture::agreeing("app/services", "rb", 5, "class A$N\n  def call; end\nend\n");
        files.extend((0..5).map(|i| {
            (
                format!("app/services/b{i}.rb"),
                format!(
                    "class B{i}\n  def a; end\n  def b; end\n  def c; end\n  def d; end\nend\n"
                ),
            )
        }));
        let root = fixture::build("split", &refs(&files));

        let (_, convs) = derive_all(&root, &Settings::default());
        assert!(
            !convs.iter().any(|c| c.id.starts_with("shape.public-arity")),
            "a 50/50 split must not become a convention: {:?}",
            convs.iter().map(|c| &c.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn deriving_twice_over_an_unchanged_tree_gives_identical_output() {
        let files = fixture::agreeing("app", "rb", 6, "class A$N\n  def call; end\nend\n");
        let root = fixture::build("stable", &refs(&files));
        let s = Settings::default();
        assert_eq!(derive_all(&root, &s).1, derive_all(&root, &s).1);
    }

    #[test]
    fn suppression_removes_a_convention_by_id_prefix() {
        let files = fixture::agreeing("app", "rb", 6, "class A$N < Base\n  def call; end\nend\n");
        let root = fixture::build("suppress", &refs(&files));

        let (_, before) = derive_all(&root, &Settings::default());
        assert!(before.iter().any(|c| c.id.starts_with("shape.base")));

        let muted = Settings { suppress: vec!["shape.base.*".into()], ..Settings::default() };
        let (_, after) = derive_all(&root, &muted);
        assert!(!after.iter().any(|c| c.id.starts_with("shape.base")));
    }

    #[test]
    fn a_rule_an_ancestor_already_states_is_dropped() {
        // Rules are derived at every level, so a repository that is snake_case
        // throughout otherwise produces the same sentence dozens of times.
        let files = fixture::agreeing(
            "app/services/enrolments",
            "rb",
            6,
            "class ItemA$N\n  def call; end\nend\n",
        );
        let root = fixture::build("collapse", &refs(&files));
        let (_, convs) = derive_all(&root, &Settings::default());
        let naming: Vec<&str> = convs
            .iter()
            .filter(|c| c.id.starts_with("naming."))
            .map(|c| c.statement.as_str())
            .collect();
        assert_eq!(naming.len(), naming.iter().collect::<std::collections::HashSet<_>>().len());
    }

    #[test]
    fn two_unrelated_directories_sharing_a_rule_both_keep_it() {
        // The dedupe must be about ancestry, not about the sentence. Dropping
        // one of these would leave its files with no rule at all.
        let mut files =
            fixture::agreeing("src/pages", "tsx", 6, "export const ItemA$N = () => 1;\n");
        files.extend(fixture::agreeing(
            "src/widgets",
            "tsx",
            6,
            "export const ItemB$N = () => 1;\n",
        ));
        let root = fixture::build("collapse-siblings", &refs(&files));
        let (_, convs) = derive_all(&root, &Settings::default());

        let pages = convs.iter().any(|c| c.scope.matches("src/pages/NewThing.tsx"));
        let widgets = convs.iter().any(|c| c.scope.matches("src/widgets/NewThing.tsx"));
        assert!(pages && widgets, "both directories must keep coverage: {convs:#?}");
    }

    #[test]
    fn an_empty_repository_yields_no_conventions_and_does_not_fail() {
        let root = fixture::build("empty", &[("README.md", "# hi\n")]);
        let (files, convs) = derive_all(&root, &Settings::default());
        assert_eq!(files.len(), 1);
        assert!(convs.is_empty());
    }

    #[test]
    fn a_typescript_component_directory_yields_a_module_surface_convention() {
        let files =
            fixture::agreeing("src/components", "tsx", 6, "export const Item$N = () => <div/>;\n");
        let root = fixture::build("components", &refs(&files));
        let (_, convs) = derive_all(&root, &Settings::default());
        let joined = convs.iter().map(|c| c.statement.as_str()).collect::<Vec<_>>().join(" | ");
        assert!(joined.contains("export exactly 1"), "got: {joined}");
    }
}
