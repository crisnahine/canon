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

mod docs;
mod dup;
mod naming;
mod render;
mod select;
mod semantic;
mod snapshot;
mod subject;
mod tier0;
mod verify;
mod vocabulary;
mod walk;

pub use dup::{DuplicateHit, duplicates_against_siblings};
pub use naming::Style;
pub use render::render_block;
pub use select::for_path;
/// Whether `rel` is offered this rule, beyond the question its scope answers.
///
/// Two rules match a path and are still withheld from it: how tests are named
/// is only about the file being written when that file *is* a test, and a rule
/// that names one directory speaks for that directory alone.
///
/// Injection and `canon explain` both ask this, through this one function.
/// Asked separately they drift, and the drift is silent: a page that lists a
/// rule the injected block withheld sends someone looking for the rule that
/// refused them to a sentence which governs nothing. The scope itself is not
/// asked here, because both callers already filter on it and one of them means
/// something looser by a directory query.
#[must_use]
pub fn offered_for_path(convention: &Convention, rel: &str) -> bool {
    if convention.id.starts_with("tests.suffix") && !tier0::is_test_path(rel) {
        return false;
    }
    speaks_for_this_directory(convention, rel)
}
pub use snapshot::{SNAPSHOT_VERSION, Snapshot};
pub use verify::{Violation, blocking_violations, missing_test, verify_source};
pub use walk::{FileEntry, MAX_FILE_BYTES, entries_for, read_indexable, walk};

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

/// Ancestor depth past which grouping stops paying for itself.
///
/// Every rule below is derived at each ancestor directory as well as at the
/// leaf, so the number of groups grows with the cap and the derivation pays for
/// each one. Four was the original guess and it cost a quarter of a Rails
/// repository's Ruby files and half of a React repository's TypeScript files
/// any rule of their own: a snapshot's scope-depth histogram stopped dead at 4
/// on every repository measured, which is what a cap looks like when it is
/// binding rather than generous.
///
/// The sharpest case is the layout canon documents as a feature. A workspace
/// holding several checkouts prefixes every path with the checkout name, so
/// `api/app/services/billing` is already at the cap and nothing below it exists:
/// opening the workspace root derived 152 rules where the two checkouts opened
/// separately derived 285 between them.
///
/// Eight, chosen by measurement rather than by argument. Raising the cap from 4
/// to 6, 8 and 10 was measured on eight real repositories; 6 recovers most of
/// the loss, 8 recovers effectively all of it, and 10 adds two rules on one
/// repository and three on a workspace for the same derivation cost. Eight is
/// the knee. The cost is
/// bounded from the other side anyway: a group below `min_files` derives
/// nothing however deep it sits, so a deeper cap buys groups only where a real
/// directory holds real files.
pub(crate) const MAX_GROUP_DEPTH: usize = 8;

/// How far down the tree a group's rule speaks.
///
/// Two groupings over the same directory, and the difference is the whole of
/// [`Scope::DirChildrenExt`]'s reason for existing: a directory holding a
/// subdirectory of another kind counts both kinds in one vote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Reach {
    /// Every file below the directory, which is how a rule reaches a folder
    /// created after indexing.
    Subtree,
    /// The directory's own files and no others.
    Children,
}

/// Every `(directory, reach)` group a file in `dir` belongs to.
///
/// The ancestor keys carry [`Reach::Subtree`], and the file's own directory
/// carries a second, [`Reach::Children`] key — but only when the cap did not
/// stop short of it. Past the cap a file has no group of its own in either
/// reach, so the two groupings agree about where the tree ends.
pub(crate) fn group_keys(dir: &str) -> Vec<(String, Reach)> {
    let mut keys = vec![(String::new(), Reach::Subtree)];
    let mut acc = String::new();
    for segment in dir.split('/').filter(|s| !s.is_empty()).take(MAX_GROUP_DEPTH) {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(segment);
        keys.push((acc.clone(), Reach::Subtree));
    }
    if acc == dir && !dir.is_empty() {
        keys.push((acc, Reach::Children));
    }
    keys
}

/// The scope a group of files speaks through.
pub(crate) fn scope_reaching(dir: &str, ext: &str, reach: Reach) -> canon_core::Scope {
    match reach {
        Reach::Subtree if dir.is_empty() => canon_core::Scope::Ext(ext.to_string()),
        Reach::Subtree => canon_core::Scope::DirExt(dir.to_string(), ext.to_string()),
        Reach::Children => canon_core::Scope::DirChildrenExt(dir.to_string(), ext.to_string()),
    }
}

/// Drop a directory's own-files rule when its subtree already answered.
///
/// The two groupings state the same sentence about overlapping sets of files,
/// and for a directory with no subdirectories they state it about the *same*
/// files. Keeping both would double the rules a leaf directory produces, and
/// where they disagree it would put two contradictory lines in one injected
/// block — "types here expose exactly 1 public method (900/950)" beside "types
/// here expose exactly 2 (20/20)" — with nothing to tell a reader which one
/// their file is.
///
/// So the subtree scope wins wherever it has an answer, and the narrower one is
/// kept only where there was no answer at all. That is what makes this change
/// monotone: every rule derived before is derived still, with the same counts
/// and the same grade, and the new ones sit in holes.
///
/// The subtree scope wins rather than the narrower one because it reaches a
/// subdirectory created after indexing, which is the property the whole
/// ancestor derivation exists for. A direct-children rule cannot: it names one
/// directory, and a folder added tomorrow inherits nothing from it.
///
/// `shape.base` and `shape.family` answer one question between them — what a
/// type here inherits — and `base_family` already yields to `base_class` inside
/// a group. Keyed apart they would stop yielding across groupings, and a
/// directory whose subtree agreed on a suffix while its own files agree on an
/// exact base would state both about the same file.
///
/// A rolled-up rule is a subtree answer like any other, which is why this runs
/// after [`roll_up_agreeing_siblings`] and keys on the scope rather than on the
/// id. Run before it, a directory's own files displaced the rule its
/// subdirectories had unanimously agreed on — trading a rule that reaches a
/// folder created tomorrow, and is deliberately never enforced, for a narrower
/// one that refuses writes.
fn keep_only_gap_filling_children(conventions: &mut Vec<Convention>) {
    let question = |c: &Convention| {
        let kind = match family(&c.id) {
            "shape.family" => "shape.base",
            other => other,
        };
        (kind, scope_dir_of(c).to_string(), scope_ext(c))
    };
    let answered: std::collections::HashSet<(&str, String, String)> =
        conventions.iter().filter(|c| !counted_over_one_directory(c)).map(&question).collect();
    conventions.retain(|c| !counted_over_one_directory(c) || !answered.contains(&question(c)));
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
    conventions.extend(docs::derive(files, root, settings));
    let facts = semantic::gather(files, root);
    conventions.extend(semantic::derive(&facts, settings));
    roll_up_agreeing_siblings(&mut conventions, settings);
    // After the roll-up, which is a subtree answer for its parent directory
    // like any other and so decides whether that directory has a gap left.
    keep_only_gap_filling_children(&mut conventions);
    collapse_redundant(&mut conventions);
    // Over the finished set, not inside the vote that produces each rule. A
    // wide rule spans more files and so sits at lower agreement than the narrow
    // rules inside it, and applying the floor during derivation killed the
    // parent first: with no parent left to absorb them, every narrow child
    // survived, and narrow children at total agreement are exactly what grades
    // `Blocking`. Raising the floor to 1.00 multiplied the rules that may
    // refuse a write by four. Filtering last is also the only place that
    // catches a rolled-up rule, whose confidence is built from its children
    // and never passed through the vote at all.
    conventions.retain(|c| c.confidence.value() >= settings.confidence_floor);
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
        "shape.family",
        "shape.mixin",
        "shape.contract",
        "shape.module-arity",
        "shape.collaborator",
        "shape.macros",
        "shape.export",
        "shape.namespace",
        "shape.import",
        "shape.annotation",
        "naming",
        "format",
        "tests.suffix",
        "tests.colocation",
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

    /// A (parent directory, extension, statement) and the children holding it.
    type Vote<'a> = ((String, String, String), Vec<&'a Convention>);

    // (parent directory, extension, statement) -> the children that hold it.
    let mut votes: HashMap<(String, String, String), Vec<&Convention>> = HashMap::new();
    // (parent, extension, family) -> the children that have an opinion of that
    // kind. Scoped per family, because a child that is silent about arity is
    // not disagreeing about arity. Counting any rule at all as an opinion made
    // twelve subdirectories that unanimously agreed look like a split.
    let mut speakers: HashMap<(String, String, &str), std::collections::HashSet<String>> =
        HashMap::new();

    for c in conventions.iter() {
        // A rule that speaks for one directory cannot be stated at the parent
        // by definition. Rolled up, a namespace shared by two subdirectories
        // reappeared as a third rule naming the parent, which is the ancestor
        // derivation `namespace_per_directory` exists to prevent — and it
        // arrived with a statement no file in the parent had voted for.
        //
        // A rule counted over one directory's own files is excluded for a
        // second reason on top of that one. It exists only because the
        // directory's own subtree could not agree; assembling a subtree claim
        // about the *parent* out of children that each failed to make one about
        // themselves states at the wider scope exactly what the narrower
        // evidence refused.
        if names_one_directory(c) || counted_over_one_directory(c) {
            continue;
        }
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
    // Both the outer order and each child list arrive from a `HashMap`, and
    // everything below reads them: which child lends the rollup its id, which
    // twelve files become its evidence, and how `sample_roots` is spelled.
    // Left alone, an unchanged tree derived a rollup named after a different
    // child each run, carrying a different twelve files.
    let mut ordered: Vec<Vote<'_>> = votes.into_iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));
    for ((parent, ext, statement), mut holders) in ordered {
        holders.sort_by(|a, b| a.id.cmp(&b.id));
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
        // Already stated at the parent, by the per-file derivation. A rule
        // counted over the parent's own files does not count as stating it:
        // that rule reaches none of the subdirectories this one was assembled
        // from, and `keep_only_gap_filling_children` drops it in favour of this
        // one immediately afterwards.
        if conventions.iter().any(|c| {
            c.statement == statement
                && scope_dir(c) == parent
                && scope_ext(c) == ext
                && !counted_over_one_directory(c)
        }) {
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
        // Widest sample, then lowest id. The tie is common — sibling
        // directories of equal size are the normal case — and left to
        // `max_by_key` it resolved to whichever child the iteration happened
        // to visit last, which renamed the rule between two runs.
        let Some(widest) =
            holders.iter().max_by(|a, b| a.total.cmp(&b.total).then_with(|| b.id.cmp(&a.id)))
        else {
            continue;
        };

        rolled.push(Convention {
            id: format!("{}.rollup", widest.id),
            statement,
            scope: canon_core::Scope::DirExt(parent.clone(), ext.clone()),
            confidence,
            agreeing,
            total,
            exemplar: widest.exemplar.clone(),
            evidence: holders.iter().flat_map(|c| c.evidence.clone()).take(12).collect(),
            sample_roots: holders.iter().flat_map(|c| c.sample_roots.clone()).collect(),
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
    struct Keyed {
        statement: String,
        ext: String,
        dir: String,
        rollup: bool,
        /// Whether the rule was counted over one directory's own files.
        children: bool,
        /// Whether the rule speaks for its own directory and no other.
        one_directory: bool,
    }
    let keyed: Vec<Keyed> = conventions
        .iter()
        .map(|c| Keyed {
            statement: c.statement.clone(),
            ext: scope_ext(c),
            dir: scope_dir(c),
            rollup: c.id.ends_with(canon_core::ROLLUP_SUFFIX),
            children: counted_over_one_directory(c),
            one_directory: names_one_directory(c),
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
    // Nor does an ancestor absorb a rule that speaks for one directory only.
    // A namespace rule stated at `src/Legacy` says nothing about
    // `src/Legacy/Http` even when both directories declare the same namespace,
    // so dropping the child left it with no rule in either half — not injected
    // before the write, not checked after it — while every tracked file in the
    // tree disagreed with what got written.
    //
    // A rule counted over one directory's own files absorbs nothing either, for
    // the same reason the rollup does not: `app/models/*.rb` reaches no file in
    // `app/models/concerns`, so letting it swallow that directory's own rule
    // would leave the subdirectory with nothing. It is still absorbed *by* an
    // ancestor that states the same sentence, which is what keeps the narrower
    // scope to the holes it was derived to fill.
    let keep: Vec<bool> = keyed
        .iter()
        .map(|k| {
            k.one_directory
                || !keyed.iter().any(|other| {
                    other.statement == k.statement
                        && other.ext == k.ext
                        && !other.rollup
                        && !other.children
                        && is_ancestor(&other.dir, &k.dir)
                })
        })
        .collect();

    let mut decisions = keep.into_iter();
    conventions.retain(|_| decisions.next().unwrap_or(true));
}

fn scope_ext(c: &Convention) -> String {
    match &c.scope {
        canon_core::Scope::Ext(e)
        | canon_core::Scope::DirExt(_, e)
        | canon_core::Scope::DirChildrenExt(_, e) => e.clone(),
        canon_core::Scope::Repo | canon_core::Scope::Dir(_) => String::new(),
    }
}

fn scope_dir(c: &Convention) -> String {
    scope_dir_of(c).to_string()
}

/// Whether a rule that speaks for exactly one directory is the right one here.
///
/// Almost every rule generalises down the tree, and that is the point: a rule
/// derived at `app/services` is meant to reach `app/services/billing`, so a
/// folder created after indexing still inherits something. `shape.namespace` is
/// the exception. PSR-4 makes a subdirectory's namespace differ from its
/// parent's by definition, so for a file below it the parent's answer is not a
/// less specific truth — it is false.
///
/// Applied to both halves. Checking must not judge a correct file against it,
/// and selection must not offer it, or the block injected before the write
/// names one namespace and the report after the write names another.
pub(crate) fn speaks_for_this_directory(convention: &Convention, rel: &str) -> bool {
    if !names_one_directory(convention) {
        return true;
    }
    scope_dir_of(convention) == rel.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// Whether a rule speaks for the directory it names and for no other.
///
/// One family, and it needs its own answer in three places: selection, checking
/// and [`collapse_redundant`], which must not let an ancestor absorb such a
/// rule the way it absorbs the ordinary kind.
fn names_one_directory(convention: &Convention) -> bool {
    convention.id.starts_with("shape.namespace")
}

/// The directory a rule names, or the empty string when it names none.
pub(crate) fn scope_dir_of(c: &Convention) -> &str {
    match &c.scope {
        canon_core::Scope::Dir(d)
        | canon_core::Scope::DirExt(d, _)
        | canon_core::Scope::DirChildrenExt(d, _) => d,
        canon_core::Scope::Repo | canon_core::Scope::Ext(_) => "",
    }
}

/// Whether a rule was counted over one directory's own files and reaches no
/// further down the tree.
///
/// Distinct from [`names_one_directory`], which is about a family whose
/// *statement* cannot be true one level down however it was counted. This one
/// is about the sample: three places have to know that such a rule speaks for
/// nothing below itself, and all three read it off the scope rather than off
/// the id.
pub(crate) fn counted_over_one_directory(c: &Convention) -> bool {
    matches!(c.scope, canon_core::Scope::DirChildrenExt(..))
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
    fn the_grade_a_snapshot_stores_is_the_grade_the_write_path_recomputes() {
        // `enforcement_for` reads its guards off the id: the extension a base
        // rule was derived for, and the rollup suffix. Handed a bare family
        // at the derive site none of them can fire, so a Python base rule at
        // total agreement was stored `Blocking` and recomputed `Advisory` —
        // the snapshot and the write path disagreeing about the one field
        // that decides whether a write can be refused.
        let mut files = fixture::agreeing(
            "app/views",
            "py",
            6,
            "class ItemView$N(BaseService):\n    def call(self):\n        pass\n",
        );
        files.extend(fixture::agreeing(
            "app/services",
            "rb",
            6,
            "class Item$N < ApplicationService\n  def call; end\nend\n",
        ));
        let root = fixture::build("stored-grade", &refs(&files));
        let settings = Settings::default();
        let (_, convs) = derive_all(&root, &settings);

        assert!(
            convs.iter().any(|c| {
                c.id.starts_with("shape.base") && c.id.rsplit('.').next() == Some("py")
            }),
            "the fixture derived no Python base rule: {:?}",
            convs.iter().map(|c| &c.id).collect::<Vec<_>>()
        );
        for c in &convs {
            assert_eq!(
                c.enforcement,
                c.enforcement_now(&settings),
                "`{}` is stored one way and recomputed another",
                c.id
            );
        }
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

    /// Two sibling directories that agree, under a parent that cannot state the
    /// rule itself, which is the shape that produces a rollup.
    fn rollup_tree() -> Vec<(String, String)> {
        let mut files: Vec<(String, String)> = Vec::new();
        for name in [
            "taskList",
            "taskStyles",
            "parseSheet",
            "miscIndex",
            "feedbackSchema",
            "questionSchema",
            "sellerAvailability",
            "valuationIndex",
        ] {
            files.push((
                format!("src/components/TaskList/{name}.ts"),
                "export const x = 1;\n".into(),
            ));
        }
        for name in [
            "buyOrSell",
            "processEmailEvent",
            "emailChecker",
            "onboardingSchema",
            "stepInitial",
            "stepPassword",
            "stepSuccess",
            "stepNew",
        ] {
            files.push((
                format!("src/components/UniversalOnboarding/{name}.ts"),
                "export const x = 1;\n".into(),
            ));
        }
        // Directly in the parent, and in another style, so the parent derives
        // nothing per-file and the rule can only reach it by rolling up.
        for name in ["Legacy", "OtherThing"] {
            files.push((format!("src/components/{name}.ts"), "export const x = 1;\n".into()));
        }
        files
    }

    #[test]
    fn a_rolled_up_rule_carries_the_same_evidence_every_run() {
        // Issue #21. The children were collected in `conventions` order, which
        // a HashMap decides, so the twelve evidence files a rollup kept were a
        // different twelve each run and the snapshot was never byte-identical.
        let files = rollup_tree();
        let root = fixture::build("rollup-stable", &refs(&files));
        let settings = Settings::default();
        let entries = walk::walk(&root, &settings);

        let first = derive_from(&root, &settings, &entries);
        assert!(
            first.iter().any(|c| c.id.ends_with(canon_core::ROLLUP_SUFFIX)),
            "fixture derived no rollup, so it cannot test one: {:?}",
            first.iter().map(|c| &c.id).collect::<Vec<_>>()
        );
        for run in 1..10 {
            assert_eq!(derive_from(&root, &settings, &entries), first, "run {run} differs");
        }
    }

    #[test]
    fn raising_the_floor_states_less_rather_than_more() {
        // Issue #20. The floor was applied inside the majority vote, so it
        // removed the wide rule first and left standing every narrow one that
        // the wide rule would have absorbed. On a 9,557-file repository the
        // maximum floor produced 179 conventions against the default's 138,
        // and 113 rules that may refuse a write against 26 — the opposite of
        // what "only state what you are certain of" promises.
        let mut files = fixture::agreeing(
            "app/services/alpha",
            "rb",
            6,
            "class ItemA$N < ApplicationService\n  def call; end\nend\n",
        );
        files.extend(fixture::agreeing(
            "app/services/beta",
            "rb",
            6,
            "class ItemB$N < ApplicationService\n  def call; end\nend\n",
        ));
        // One dissenter directly in the parent, so the parent holds the rule
        // at 12/13 while each child holds it at 6/6.
        files.push((
            "app/services/odd.rb".to_string(),
            "class Odd < SomethingElse\n  def call; end\nend\n".to_string(),
        ));
        let root = fixture::build("floor-monotonic", &refs(&files));

        let default = derive_all(&root, &Settings::default()).1;
        let strict =
            derive_all(&root, &Settings { confidence_floor: 1.0, ..Settings::default() }).1;

        let blocking = |cs: &[Convention]| {
            cs.iter().filter(|c| c.enforcement == canon_core::Enforcement::Blocking).count()
        };
        assert!(
            strict.len() <= default.len(),
            "a higher floor produced more rules: {} against {}",
            strict.len(),
            default.len()
        );
        assert!(
            blocking(&strict) <= blocking(&default),
            "a higher floor produced more refusals: {} against {}",
            blocking(&strict),
            blocking(&default)
        );
        for c in &strict {
            assert!(
                default.iter().any(|d| d.id == c.id && d.statement == c.statement),
                "`{}` is stated only at the higher floor",
                c.id
            );
        }
    }

    #[test]
    fn a_directory_keeps_its_namespace_rule_when_an_ancestor_shares_the_answer() {
        // A namespace rule speaks for one directory, so an ancestor stating the
        // same sentence does not cover the child the way an ancestor normally
        // does. Absorbing it left the child with no rule in either half: not
        // injected, not checked, in a tree where every tracked file disagrees
        // with the file being written. Legacy PHP that predates PSR-4 spans one
        // namespace over a subtree, which is exactly this shape.
        let mut files = fixture::agreeing(
            "src/Legacy",
            "php",
            6,
            "<?php\nnamespace App\\Legacy;\nclass ItemA$N { public function handle() {} }\n",
        );
        files.extend(fixture::agreeing(
            "src/Legacy/Http",
            "php",
            6,
            "<?php\nnamespace App\\Legacy;\nclass ItemB$N { public function handle() {} }\n",
        ));
        let root = fixture::build("collapse-namespace", &refs(&files));
        let (_, convs) = derive_all(&root, &Settings::default());

        let rel = "src/Legacy/Http/Client.php";
        assert!(
            select::for_path(&convs, rel, 4000).iter().any(|c| c.id.starts_with("shape.namespace")),
            "the child directory was left with no namespace rule: {:?}",
            convs.iter().map(|c| &c.id).collect::<Vec<_>>()
        );
        let wrong = "<?php\nnamespace App\\Other;\nclass Client { public function handle() {} }\n";
        assert!(
            !verify::verify_source(rel, wrong, &convs).is_empty(),
            "a file disagreeing with all twelve tracked files was not reported"
        );
    }

    #[test]
    fn a_namespace_shared_by_two_subdirectories_is_not_stated_at_their_parent() {
        // Rolling a rule up to the parent is how a directory with no rule of
        // its own inherits one, and it is exactly wrong for a namespace: the
        // parent's files declare their own, and the rolled-up sentence names a
        // namespace not one of them voted for. Two rules then reach one file
        // and name two different namespaces.
        let mut files = fixture::agreeing(
            "src/Legacy",
            "php",
            6,
            "<?php\nnamespace App\\Legacy;\nclass ItemA$N { public function handle() {} }\n",
        );
        for child in ["Http", "Api"] {
            files.extend(fixture::agreeing(
                &format!("src/Legacy/{child}"),
                "php",
                6,
                &format!(
                    "<?php\nnamespace App\\Shared;\nclass Item{child}$N {{ public function handle() {{}} }}\n"
                ),
            ));
        }
        let root = fixture::build("rollup-namespace", &refs(&files));
        let (_, convs) = derive_all(&root, &Settings::default());

        assert!(
            !convs.iter().any(|c| c.id.starts_with("shape.namespace")
                && c.id.ends_with(canon_core::ROLLUP_SUFFIX)),
            "a namespace was assembled for a parent whose files never voted for it: {:?}",
            convs.iter().map(|c| &c.id).collect::<Vec<_>>()
        );
        let offered: Vec<&str> = select::for_path(&convs, "src/Legacy/New.php", 4000)
            .iter()
            .filter(|c| c.id.starts_with("shape.namespace"))
            .map(|c| c.statement.as_str())
            .collect();
        assert_eq!(offered.len(), 1, "two namespaces offered for one file: {offered:?}");
        assert!(offered[0].ends_with("`App\\Legacy`"), "got {offered:?}");
    }

    #[test]
    fn every_kind_of_rule_is_counted_in_its_own_bucket() {
        // A child silent about one kind is not dissenting about it, which is
        // why agreement is counted per kind. Two kinds were missing from the
        // table and both fell to `other`, so a directory holding a colocation
        // rule and no import rule read as an import dissenter and its siblings
        // lost the rollup — for the family the code calls the highest-value
        // thing canon derives.
        let kinds = [
            "shape.public-arity.a.rb",
            "shape.entrypoint.a.rb",
            "shape.base.a.rb",
            "shape.family.a.rb",
            "shape.mixin.a.rb",
            "shape.contract.a.php",
            "shape.module-arity.a.ts",
            "shape.collaborator.a.rb",
            "shape.macros.a.vue",
            "shape.export.a.tsx",
            "shape.namespace.a.php",
            "shape.import.a.ts",
            "shape.annotation.a.ts",
            "naming.a.rb",
            "format.a.erb",
            "tests.suffix.rb",
            "tests.colocation.a.rb",
        ];
        let seen: std::collections::HashSet<&str> = kinds.iter().map(|id| family(id)).collect();
        assert_eq!(seen.len(), kinds.len(), "two kinds share a bucket: {seen:?}");
        assert!(!seen.contains("other"), "a real kind fell through to `other`: {seen:?}");
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

    /// A models directory holding a concerns subdirectory: the shape of
    /// `app/models` in a real Rails repository, where 123 of 128 models inherit
    /// `ApplicationRecord` and 36 concerns are modules that inherit nothing.
    fn models_beside_concerns() -> Vec<(String, String)> {
        let mut files = fixture::agreeing(
            "app/models",
            "rb",
            16,
            "class Item$N < ApplicationRecord\n  def to_label; end\nend\n",
        );
        files.extend(fixture::agreeing(
            "app/models/concerns",
            "rb",
            10,
            "module Concern$N\n  def helper; end\nend\n",
        ));
        files
    }

    #[test]
    fn a_subdirectory_of_another_kind_no_longer_dilutes_its_parent() {
        // 16 of 26 over the subtree is a coin flip with a lean; 16 of 16 over
        // the directory's own files is the rule the models actually hold. On a
        // real Rails repository this is 123 of 164 against 123 of 128, and all
        // 128 models derived nothing about their base at all.
        let root = fixture::build("models-concerns", &refs(&models_beside_concerns()));
        let (_, convs) = derive_all(&root, &Settings::default());
        let base = convs
            .iter()
            .filter(|c| c.id.starts_with("shape.base"))
            .find(|c| c.statement.contains("ApplicationRecord"))
            .unwrap_or_else(|| panic!("no base rule for the models in {:?}", ids(&convs)));
        assert_eq!(
            base.scope,
            canon_core::Scope::DirChildrenExt("app/models".into(), "rb".into()),
            "the rule was counted over the subtree that outvoted it"
        );
        assert_eq!((base.agreeing, base.total), (16, 16));
        assert!(base.scope.matches("app/models/new_thing.rb"));
        assert!(!base.scope.matches("app/models/concerns/new_validator.rb"));
    }

    #[test]
    fn the_subdirectory_keeps_its_own_rules_beside_the_narrower_parent_rule() {
        // The concerns are not displaced by the rule their exclusion made
        // possible: they are a different kind of file with their own scope.
        let root = fixture::build("models-concerns-keep", &refs(&models_beside_concerns()));
        let (_, convs) = derive_all(&root, &Settings::default());
        assert!(
            convs.iter().any(|c| scope_dir_of(c) == "app/models/concerns"),
            "the subdirectory lost its own rules: {:?}",
            ids(&convs)
        );
        assert!(
            convs.iter().any(|c| c.scope.matches("app/models/concerns/new_validator.rb")),
            "the subdirectory lost its coverage: {:?}",
            ids(&convs)
        );
    }

    #[test]
    fn a_directory_with_no_subdirectories_states_its_rule_once() {
        // The two groupings hold the same files there, so keeping both would
        // double every rule a leaf directory produces. The subtree scope is the
        // one kept: it reaches a subdirectory created after indexing, which is
        // what the whole ancestor derivation exists for.
        let files = fixture::agreeing(
            "app/services",
            "rb",
            6,
            "class Item$N < ApplicationService\n  def call; end\nend\n",
        );
        let root = fixture::build("leaf-once", &refs(&files));
        let (_, convs) = derive_all(&root, &Settings::default());
        let bases: Vec<&canon_core::Scope> =
            convs.iter().filter(|c| c.id.starts_with("shape.base")).map(|c| &c.scope).collect();
        // One, and it reaches the subtree. `collapse_redundant` then keeps
        // whichever ancestor states it, which here is `app` itself.
        assert_eq!(bases.len(), 1, "got {bases:?}");
        assert!(matches!(bases[0], canon_core::Scope::DirExt(..)), "got {bases:?}");
    }

    #[test]
    fn every_derived_rule_has_an_id_no_other_rule_shares() {
        // The two groupings build the same id for the same directory and
        // extension, because they answer the same question about the same
        // place. Only one of them is ever kept, and a user suppressing
        // `shape.base.app.models.rb` has to reach whichever it is.
        let mut files = models_beside_concerns();
        files.extend(fixture::agreeing(
            "app/services",
            "rb",
            6,
            "class Svc$N < ApplicationService\n  def call; end\nend\n",
        ));
        files.extend(fixture::agreeing(
            "src/components",
            "tsx",
            6,
            "export const Item$N = () => <div/>;\n",
        ));
        let root = fixture::build("unique-ids", &refs(&files));
        let (_, convs) = derive_all(&root, &Settings::default());
        let mut seen = std::collections::HashSet::new();
        for c in &convs {
            assert!(seen.insert(c.id.clone()), "`{}` is derived twice", c.id);
        }
    }

    #[test]
    fn a_narrow_rule_does_not_absorb_the_subdirectory_it_cannot_reach() {
        // `collapse_redundant` drops a rule an ancestor states in the same
        // words, and `app/models/*.rb` is an ancestor of `app/models/concerns`
        // by directory while reaching not one file in it. Absorbing the child
        // would leave that subdirectory with no rule in either half: not
        // injected before the write, not checked after it.
        let mut files = fixture::agreeing(
            "app/models",
            "rb",
            16,
            "class Item$N < ApplicationRecord\n  def to_label; end\nend\n",
        );
        // Concerns that keep the parent's own vote split while agreeing with
        // each other on the same sentence the parent's own files hold.
        files.extend(fixture::agreeing(
            "app/models/concerns",
            "rb",
            10,
            "module Concern$N\n  def helper; end\nend\n",
        ));
        let root = fixture::build("no-narrow-absorb", &refs(&files));
        let (_, convs) = derive_all(&root, &Settings::default());
        let narrow: Vec<&Convention> =
            convs.iter().filter(|c| counted_over_one_directory(c)).collect();
        assert!(!narrow.is_empty(), "the fixture derived no narrow rule to test");
        for c in &narrow {
            let dir = scope_dir_of(c);
            assert!(
                !convs.iter().any(|other| other.statement == c.statement
                    && scope_dir_of(other).starts_with(&format!("{dir}/"))
                    && counted_over_one_directory(other)),
                "`{}` may have absorbed a subdirectory it cannot reach",
                c.id
            );
        }
        assert!(convs.iter().any(|c| c.scope.matches("app/models/concerns/other.rb")));
    }

    #[test]
    fn a_narrow_rule_is_never_assembled_into_a_claim_about_a_parent_subtree() {
        // A rule counted over one directory's own files exists because that
        // directory's subtree could not agree. Rolling several of them up
        // states at the parent's subtree exactly what the children's own
        // subtrees refused.
        let mut files: Vec<(String, String)> = Vec::new();
        for child in ["alpha", "beta"] {
            files.extend(fixture::agreeing(
                &format!("app/{child}"),
                "rb",
                16,
                &format!("class Item{child}$N < ApplicationRecord\n  def to_label; end\nend\n"),
            ));
            files.extend(fixture::agreeing(
                &format!("app/{child}/concerns"),
                "rb",
                10,
                &format!("module Concern{child}$N\n  def helper; end\nend\n"),
            ));
        }
        let root = fixture::build("no-narrow-rollup", &refs(&files));
        let (_, convs) = derive_all(&root, &Settings::default());
        assert!(
            convs.iter().any(counted_over_one_directory),
            "the fixture derived no narrow rule to roll up"
        );
        assert!(
            !convs.iter().any(|c| c.id.ends_with(canon_core::ROLLUP_SUFFIX)
                && c.statement.contains("ApplicationRecord")),
            "a subtree claim was assembled from directories whose subtrees refused it: {:?}",
            ids(&convs)
        );
    }

    fn ids(convs: &[Convention]) -> Vec<&str> {
        convs.iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn a_rule_the_subdirectories_agree_on_is_not_displaced_by_the_parents_own_files() {
        // A rolled-up rule is a subtree answer for its parent, and the parent's
        // own files must not take its place: measured on a real PHP repository,
        // an advisory rule counted over 76 files in three subdirectories was
        // replaced by an enforced one counted over the 27 in the parent, which
        // is a refusal appearing where a piece of advice used to be and a
        // subdirectory added tomorrow inheriting nothing.
        let body = "export const Thing = () => <div/>;\n";
        let mut files: Vec<(String, String)> = Vec::new();
        // Two subdirectories that agree, and the parent's own files agreeing
        // with them, all in PascalCase.
        for dir in ["src/components/alpha", "src/components/beta", "src/components"] {
            for name in [
                "UserCard",
                "OrderList",
                "PayoutForm",
                "LoginPanel",
                "NavBar",
                "SideMenu",
                "TopBanner",
                "FooterLinks",
            ] {
                files.push((format!("{dir}/{name}.tsx"), body.to_string()));
            }
        }
        // A third subdirectory whose own names witness no single style, so it
        // derives no rule and is not counted as a dissenter — while its files
        // still sink the parent's per-file vote below the floor. This is what
        // makes the rollup the only way the parent states the rule at all.
        for name in [
            "create_thing",
            "update_thing",
            "cancel_thing",
            "refundThing",
            "approveThing",
            "rejectThing",
            "settle-batch",
            "send-receipt",
            "void-invoice",
            "ChargeCard",
            "SplitPayout",
            "VoidRefund",
        ] {
            files.push((format!("src/components/gamma/{name}.tsx"), body.to_string()));
        }

        let root = fixture::build("rollup-not-displaced", &refs(&files));
        let (_, convs) = derive_all(&root, &Settings::default());
        let at_parent: Vec<&Convention> = convs
            .iter()
            .filter(|c| c.id.starts_with("naming.") && scope_dir_of(c) == "src/components")
            .collect();
        assert_eq!(at_parent.len(), 1, "got {at_parent:#?}");
        assert!(
            at_parent[0].id.ends_with(canon_core::ROLLUP_SUFFIX),
            "the parent's own files displaced what its subdirectories agreed on: `{}`",
            at_parent[0].id
        );
        assert_eq!(at_parent[0].enforcement, canon_core::Enforcement::Advisory);
        assert!(
            at_parent[0].scope.matches("src/components/delta/BrandNew.tsx"),
            "a subdirectory added after indexing inherits nothing"
        );
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
