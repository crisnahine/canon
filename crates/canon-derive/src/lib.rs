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
mod subject;
mod tier0;
mod verify;
mod walk;

pub use dup::{DuplicateHit, duplicates_against_siblings};
pub use naming::Style;
pub use render::render_block;
pub use select::for_path;
pub use snapshot::{SNAPSHOT_VERSION, Snapshot};
pub use verify::{Violation, blocking_violations, missing_test, verify_source};
pub use walk::{FileEntry, MAX_FILE_BYTES, entries_for, walk};

use canon_core::{Confidence, Convention, Settings};

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
    roll_up_agreeing_siblings(&mut conventions, settings);
    collapse_redundant(&mut conventions);
    // Last, not first. A rule is derived at several scopes and the narrower
    // copies are removed by `collapse_redundant` because a wider one already
    // says it. Removing the wider one first left the narrower ones standing,
    // so suppressing `naming.repo.txt` produced `naming.api.txt` — the same
    // statement, the same refusal, a new id, and a user with no reason to
    // believe the next suppression would end any differently.
    conventions.retain(|c| !settings.is_suppressed(&c.id));
    // Deterministic order, so an unchanged tree produces a byte-identical
    // snapshot and cache staleness stays observable.
    conventions.sort_by(|a, b| a.id.cmp(&b.id));
    conventions
}

/// The kind of rule an id names, for comparing like with like.
///
/// A child silent about one kind of rule is not dissenting about it, so
/// agreement has to be counted within a kind rather than across all of them.
fn family(id: &str) -> &'static str {
    const FAMILIES: &[&str] = &[
        "shape.public-arity",
        "shape.entrypoint",
        "shape.base",
        "shape.module-arity",
        "shape.collaborator",
        "naming",
        "tests.suffix",
    ];
    FAMILIES.iter().find(|f| id.starts_with(**f)).copied().unwrap_or("other")
}

/// State a rule at the parent when its children all say it.
///
/// Rules are derived per directory, and a rule over the parent is derived from
/// the parent's *files*. Those are different statistics, and the difference
/// matters: a services tree where twelve subdirectories each hold the rule
/// without exception can still sit at 0.82 across all its files, because the
/// files are unevenly distributed between them. The per-file view then rejects
/// a rule that every part of the tree actually follows, and a file written into
/// a thirteenth subdirectory is told nothing.
///
/// So agreement is counted over directories here, not files. When every child
/// of a parent that has an opinion holds the same one, the parent gets the rule
/// with the children as its evidence.
///
/// A dissenting child keeps its own rule: [`collapse_redundant`] only removes a
/// child that repeats its ancestor word for word, and a child that differs does
/// not.
fn roll_up_agreeing_siblings(conventions: &mut Vec<Convention>, settings: &Settings) {
    use std::collections::HashMap;

    // (parent directory, extension, statement) -> the children that hold it.
    let mut votes: HashMap<(String, String, String), Vec<&Convention>> = HashMap::new();
    // (parent, extension, family) -> the children that have an opinion of that
    // kind. Scoped per family, because a child that is silent about arity is
    // not disagreeing about arity. Counting any rule at all as an opinion made
    // twelve subdirectories that unanimously agreed look like a split.
    let mut speakers: HashMap<(String, String, &str), std::collections::HashSet<String>> =
        HashMap::new();

    for c in conventions.iter() {
        let dir = scope_dir(c);
        let ext = scope_ext(c);
        let Some((parent, _)) = dir.rsplit_once('/') else { continue };
        speakers
            .entry((parent.to_string(), ext.clone(), family(&c.id)))
            .or_default()
            .insert(dir.clone());
        votes.entry((parent.to_string(), ext, c.statement.clone())).or_default().push(c);
    }

    let mut rolled: Vec<Convention> = Vec::new();
    for ((parent, ext, statement), holders) in votes {
        let Some(kind) = holders.first().map(|c| family(&c.id)) else { continue };
        let Some(all) = speakers.get(&(parent.clone(), ext.clone(), kind)) else { continue };
        // Two children is a pair, not a pattern.
        if all.len() < 2 {
            continue;
        }
        let holding: std::collections::HashSet<String> =
            holders.iter().map(|c| scope_dir(c)).collect();
        if holding.len() != all.len() {
            continue; // a child dissents, so the parent has no single answer
        }
        // Already stated at the parent, by the per-file derivation.
        if conventions
            .iter()
            .any(|c| c.statement == statement && scope_dir(c) == parent && scope_ext(c) == ext)
        {
            continue;
        }
        // Gate on directories, report on files. The rule is only rolled up
        // because every child directory holds it, but the numbers a reader
        // sees are the files behind those children, so the confidence has to
        // be the one those numbers imply. Reporting 1.00 beside 377/401 is a
        // contradiction on the face of the block.
        let agreeing: usize = holders.iter().map(|c| c.agreeing).sum();
        let total: usize = holders.iter().map(|c| c.total).sum();
        let Some(confidence) = Confidence::derive(agreeing, total) else { continue };
        let Some(widest) = holders.iter().max_by_key(|c| c.total) else { continue };

        rolled.push(Convention {
            id: format!("{}.rollup", widest.id),
            statement,
            scope: canon_core::Scope::DirExt(parent.clone(), ext.clone()),
            confidence,
            agreeing,
            total,
            exemplar: widest.exemplar.clone(),
            evidence: holders.iter().flat_map(|c| c.evidence.clone()).take(12).collect(),
            // A rule assembled from other rules is never enforced. The evidence
            // is real, but it is one inference further from the code than the
            // rules it was built from.
            enforcement: canon_core::Enforcement::Advisory,
        });
    }

    let _ = settings;
    conventions.extend(rolled);
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
    let keyed: Vec<(String, String, String, bool)> = conventions
        .iter()
        .map(|c| {
            (
                c.statement.clone(),
                scope_ext(c),
                scope_dir(c),
                c.id.ends_with(canon_core::ROLLUP_SUFFIX),
            )
        })
        .collect();

    // `is_ancestor` is false for equal directories, so a rule never eliminates
    // itself and no index bookkeeping is needed to skip it.
    //
    // A rolled-up ancestor does not absorb a child, because the two do not say
    // the same thing with the same authority: the child was counted over the
    // files it speaks for and may refuse a write, while the rollup generalises
    // to sibling directories that have not voted and never refuses anything.
    // Dropping the child moved a directory from enforced to advisory without
    // anything in the output saying so.
    let keep: Vec<bool> = keyed
        .iter()
        .map(|(statement, ext, dir, _)| {
            !keyed.iter().any(|(other_statement, other_ext, other_dir, other_rollup)| {
                other_statement == statement
                    && other_ext == ext
                    && !other_rollup
                    && is_ancestor(other_dir, dir)
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
