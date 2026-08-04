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
        if names_one_directory(c) {
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
    let keep: Vec<bool> = keyed
        .iter()
        .map(|k| {
            k.one_directory
                || !keyed.iter().any(|other| {
                    other.statement == k.statement
                        && other.ext == k.ext
                        && !other.rollup
                        && is_ancestor(&other.dir, &k.dir)
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
        canon_core::Scope::Dir(d) | canon_core::Scope::DirExt(d, _) => d,
        canon_core::Scope::Repo | canon_core::Scope::Ext(_) => "",
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
