//! Conventions that need a parser.
//!
//! This is the tier that answers "how do we write code here" rather than
//! "where do files go". Every rule below is a count over facts that only a
//! grammar can produce, and each one is the kind of rule a team states in
//! review and no linter checks.

use std::collections::HashMap;

use canon_core::{Confidence, Convention, Enforcement, Evidence, Settings};
use canon_extract::FileFacts;

use crate::tier0::{id_fragment, scope_for};
use crate::walk::FileEntry;

/// Facts for one file, tagged with where it lives and how much it counts.
pub(crate) struct FactSet {
    pub rel: String,
    pub dir: String,
    pub ext: String,
    pub weight: f32,
    pub modified_unix: u64,
    pub facts: FileFacts,
    /// File name without its extension, for resolving the primary type.
    pub stem: String,
}

impl FactSet {
    /// The type this file is about, if it declares one.
    fn subject(&self) -> Option<&canon_extract::TypeFacts> {
        crate::subject::primary_type(&self.facts, &self.stem)
    }
}

const MAX_EVIDENCE: usize = 12;

/// Parse every file canon has an extractor for.
///
/// A file that fails to parse is skipped rather than fatal. A working tree
/// always contains something mid-edit, and one broken file must not cost the
/// repository its conventions.
pub(crate) fn gather(files: &[FileEntry], root: &std::path::Path) -> Vec<FactSet> {
    files
        .iter()
        .filter_map(|f| {
            let language = canon_extract::lang::from_extension(&f.ext)?;
            let source = std::fs::read_to_string(root.join(&f.rel)).ok()?;
            let facts = canon_extract::extract(language, &source, &f.rel).ok()?;
            if facts.is_empty() {
                return None;
            }
            Some(FactSet {
                rel: f.rel.clone(),
                dir: f.dir.clone(),
                ext: f.ext.clone(),
                weight: f.weight,
                modified_unix: f.modified_unix,
                stem: f.stem.clone(),
                facts,
            })
        })
        .collect()
}

/// Derive every Tier 1 convention.
pub(crate) fn derive(sets: &[FactSet], settings: &Settings) -> Vec<Convention> {
    let mut groups: HashMap<(String, String, crate::Reach), Vec<&FactSet>> = HashMap::new();
    for s in sets {
        for (dir, reach) in crate::group_keys(&s.dir) {
            groups.entry((dir, s.ext.clone(), reach)).or_default().push(s);
        }
    }

    let mut out = Vec::new();
    for ((dir, ext, reach), members) in groups {
        // Shape is a property of a place in the tree, never of a whole
        // repository. Derived repository-wide on a Rails codebase the
        // migrations outnumber everything else, producing "the public method
        // here is named `change`" for every Ruby file in the project.
        if dir.is_empty() {
            continue;
        }
        out.extend(rules_for(&dir, &ext, reach, &members, settings));
    }
    out.extend(namespace_per_directory(sets, settings));
    out
}

/// Every rule one group of files supports.
///
/// The scope is written last, from `reach`, rather than threaded through
/// thirteen rule functions that each build the same one: the two groupings
/// count different files and produce identical sentences about them, and the
/// only thing that differs is which files the sentence speaks for.
fn rules_for(
    dir: &str,
    ext: &str,
    reach: crate::Reach,
    members: &[&FactSet],
    settings: &Settings,
) -> Vec<Convention> {
    let mut out = Vec::new();
    out.extend(public_arity(dir, ext, members, settings));
    out.extend(entrypoint_name(dir, ext, members, settings));
    let base = base_class(dir, ext, members, settings);
    if base.is_none() {
        out.extend(base_family(dir, ext, members, settings));
    }
    let contract = contract(dir, ext, members, settings);
    // A Rust type's `superclass` is its first trait impl, so both rules
    // read one fact and would state it twice — "Types here inherit from
    // `Loggable`" beside "Types here implement `Loggable`", two lines of
    // the injected budget and two claims the checker cannot collapse. A
    // trait impl is not a base class, and `contract` is the sentence that
    // says so, the same way `base_family` yields to `base_class` above.
    if !(contract.is_some() && reads_a_base_from_a_contract(ext)) {
        out.extend(base);
    }
    out.extend(contract);
    out.extend(module_arity(dir, ext, members, settings));
    out.extend(collaborator(dir, ext, members, settings));
    out.extend(macros(dir, ext, members, settings));
    out.extend(import_source(dir, ext, members, settings));
    out.extend(export_style(dir, ext, members, settings));
    out.extend(annotation(dir, ext, members, settings));
    out.extend(mixin(dir, ext, members, settings));
    for c in &mut out {
        c.scope = crate::scope_reaching(dir, ext, reach);
    }
    out
}

/// Namespaces, grouped by the directory a file is actually in.
///
/// Every other rule is derived at each ancestor directory too, so a new folder
/// inherits something from the tree above it. A namespace rule must not be:
/// PSR-4 makes a subdirectory's namespace differ from its parent's by
/// definition, so an ancestor group counts files that are all correct and
/// finds them all in disagreement.
///
/// It also cannot use the shared grouping for a second reason. That grouping
/// stops at [`MAX_GROUP_DEPTH`], and PSR-4 trees routinely run deeper —
/// `wp-content/themes/<theme>/lib/jwt` is five levels down, and its five
/// namespaced files had no group of their own at all.
fn namespace_per_directory(sets: &[FactSet], settings: &Settings) -> Vec<Convention> {
    let mut groups: HashMap<(&str, &str), Vec<&FactSet>> = HashMap::new();
    for s in sets {
        groups.entry((s.dir.as_str(), s.ext.as_str())).or_default().push(s);
    }
    // Sorted, so an unchanged tree derives the same set twice.
    let mut keys: Vec<(&str, &str)> = groups.keys().copied().collect();
    keys.sort_unstable();
    keys.into_iter()
        .filter(|(dir, _)| !dir.is_empty())
        .filter_map(|key| {
            let members = groups.get(&key)?;
            namespace(key.0, key.1, members, settings)
        })
        .collect()
}

/// "Files here export a default."
///
/// The choice a component tree makes once and then holds, and the one that no
/// other rule in the vocabulary can see: a 1,618-file TSX tree had two rules
/// between all of it, because everything else asks about base classes and
/// public method counts and a function component has neither. Getting it wrong
/// is drift that compiles — the module is fine, and every import of it has to
/// be written the other way round.
///
/// Only files that export something vote. A module that exports nothing has
/// made no choice, and counting it as "not a default export" would let a
/// directory of type declarations decide the rule for the components beside
/// them.
///
/// Gated on the language having the concept at all. Counting `false` across a
/// Ruby directory finds total agreement about a thing Ruby has no word for.
fn export_style(
    dir: &str,
    ext: &str,
    members: &[&FactSet],
    settings: &Settings,
) -> Option<Convention> {
    if !canon_extract::lang::from_extension(ext)
        .is_some_and(|l| canon_extract::lang::provider(l).default_exports)
    {
        return None;
    }
    let observations: Vec<(bool, f32, &FactSet)> = members
        .iter()
        .filter(|s| s.facts.default_export || !s.facts.free_functions.is_empty())
        .map(|s| (s.facts.default_export, s.weight, *s))
        .collect();
    let (default_export, confidence, agreeing) = majority(&observations, settings)?;
    Some(Convention {
        id: format!("shape.export.{}.{ext}", id_fragment(dir)),
        statement: if default_export {
            "Files here export a default".to_string()
        } else {
            "Files here use named exports".to_string()
        },
        scope: scope_for(dir, ext),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &default_export),
        evidence: evidence(&observations, &default_export),
        sample_roots: Vec::new(),
        // Advisory. A directory that exports defaults throughout can still
        // legitimately gain the one barrel or constants module that does not,
        // and refusing that is the check being wrong about a correct file.
        enforcement: Enforcement::Advisory,
    })
}

/// "Files here declare namespace `App\Services\Billing`."
///
/// PSR-4 makes a PHP file's namespace agree with its directory, which is a real
/// convention a team holds and the only structural one a procedural plugin file
/// has. 134 tracked PHP files derived nothing at all, because every rule above
/// asks about a class and its base type.
///
/// Counted per directory, where the namespace is a constant. A file that
/// declares none votes too — a directory where PSR-4 holds and one file forgot
/// is exactly the disagreement worth reporting.
///
/// `members` are the files of one directory exactly, never a subtree; see
/// [`namespace_per_directory`]. `check_namespace` applies the matching
/// restriction, because the scope a rule carries still reaches the whole
/// subtree the way every directory scope does.
fn namespace(
    dir: &str,
    ext: &str,
    members: &[&FactSet],
    settings: &Settings,
) -> Option<Convention> {
    let observations: Vec<(Option<String>, f32, &FactSet)> =
        members.iter().map(|s| (s.facts.namespace.clone(), s.weight, *s)).collect();
    let (winner, confidence, agreeing) = majority(&observations, settings)?;
    let declared = winner.clone()?;
    Some(Convention {
        id: format!("shape.namespace.{}.{ext}", id_fragment(dir)),
        statement: format!("Files here declare namespace `{declared}`"),
        scope: scope_for(dir, ext),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &winner),
        evidence: evidence(&observations, &winner),
        sample_roots: Vec::new(),
        enforcement: Enforcement::Advisory,
    })
}

/// The weighted majority of a set of `(value, weight, source)` observations.
///
/// Shared by all four rules below, because they differ only in what they count.
fn majority<T: std::hash::Hash + Eq + Ord + Clone>(
    observations: &[(T, f32, &FactSet)],
    settings: &Settings,
) -> Option<(T, Confidence, usize)> {
    if observations.len() < settings.min_files {
        return None;
    }
    let total: f32 = observations.iter().map(|(_, w, _)| *w).sum();
    let mut tally: HashMap<T, f32> = HashMap::new();
    for (value, weight, _) in observations {
        *tally.entry(value.clone()).or_default() += *weight;
    }
    let (winner, weight) = tally_winner(tally)?;
    // The configured floor is not applied here. Applied during the vote it
    // removed the wide rule before `collapse_redundant` could use it, and every
    // narrow rule the wide one would have absorbed survived instead; see
    // `derive_from`, which filters the finished set.
    let confidence = Confidence::derive_counted(weight, total, observations.len())?;
    let agreeing = observations.iter().filter(|(v, _, _)| *v == winner).count();
    Some((winner, confidence, agreeing))
}

/// The heaviest value in a weight tally, ties broken toward the lowest value.
///
/// A plain `max_by` resolves an exact tie to whichever entry a `HashMap`
/// happens to visit last, and this repository has already shipped one
/// derivation that changed rule sets between runs of the same tree from
/// exactly that: iteration order over a `HashMap` is not the same twice.
/// Breaking every tie the same way regardless of insertion order is what
/// makes a rebuild of an unchanged tree derive the same set every time.
///
/// A tied winner can only reach a caller of [`majority`] once a future change
/// loosens [`Confidence::FLOOR`] below one half — today the floor sits at 0.8,
/// so two values splitting a vote can never both clear it — but the tie-break
/// is written to be correct on its own terms rather than to rely on that.
fn tally_winner<T: Ord>(tally: HashMap<T, f32>) -> Option<(T, f32)> {
    tally.into_iter().max_by(|a, b| {
        a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| b.0.cmp(&a.0))
    })
}

fn exemplar<T: PartialEq>(observations: &[(T, f32, &FactSet)], winner: &T) -> Option<String> {
    observations
        .iter()
        .filter(|(v, _, _)| v == winner)
        .max_by_key(|(_, _, s)| (s.modified_unix, s.rel.clone()))
        .map(|(_, _, s)| s.rel.clone())
}

fn evidence<T: PartialEq>(observations: &[(T, f32, &FactSet)], winner: &T) -> Vec<Evidence> {
    let mut agreeing: Vec<&(T, f32, &FactSet)> =
        observations.iter().filter(|(v, _, _)| v == winner).collect();
    agreeing.sort_by(|a, b| b.2.modified_unix.cmp(&a.2.modified_unix).then(a.2.rel.cmp(&b.2.rel)));
    agreeing
        .into_iter()
        .take(MAX_EVIDENCE)
        .map(|(_, _, s)| Evidence {
            rel: s.rel.clone(),
            line: s.facts.types.first().map_or(0, |t| t.line),
        })
        .collect()
}

/// "Types in `app/services/` expose exactly one public method."
///
/// The convention every style guide states and no linter checks. It is only
/// derivable with a parser, and only correctly with per-language visibility
/// rules, which is why [`canon_extract`] resolves visibility before this sees
/// the facts.
fn public_arity(
    dir: &str,
    ext: &str,
    members: &[&FactSet],
    settings: &Settings,
) -> Option<Convention> {
    let observations: Vec<(usize, f32, &FactSet)> = members
        .iter()
        .filter_map(|s| s.subject().map(|t| (t.public_arity(), s.weight, *s)))
        .collect();
    let (arity, confidence, agreeing) = majority(&observations, settings)?;
    // Zero public methods is not a convention, it is a directory of data
    // classes, and stating it would spend the budget saying nothing.
    if arity == 0 {
        return None;
    }
    // The whole id, not the family. `enforcement_for` reads guards off the
    // rest of it — the extension a base rule was derived for, the rollup
    // suffix — and handed a bare family none of them can fire, so the grade
    // stored in the snapshot disagreed with the one the write path recomputes
    // from the same id.
    let id = format!("shape.public-arity.{}.{ext}", id_fragment(dir));
    let scope = scope_for(dir, ext);
    Some(Convention {
        statement: format!(
            "Types here expose exactly {arity} public method{}",
            if arity == 1 { "" } else { "s" }
        ),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &arity),
        evidence: evidence(&observations, &arity),
        sample_roots: Vec::new(),
        enforcement: canon_core::enforcement_for(&id, &scope, confidence, settings),
        scope,
        id,
    })
}

/// "That public method is named `call`."
///
/// Only meaningful alongside a single-public-method shape, so it is derived
/// over single-method types only. Deriving it across all types would average
/// the entrypoint of a service object together with the fourth method of an
/// unrelated model.
fn entrypoint_name(
    dir: &str,
    ext: &str,
    members: &[&FactSet],
    settings: &Settings,
) -> Option<Convention> {
    let observations: Vec<(String, f32, &FactSet)> = members
        .iter()
        .filter_map(|s| {
            let t = s.subject()?;
            if t.public_arity() != 1 {
                return None;
            }
            Some((t.public_methods.first()?.clone(), s.weight, *s))
        })
        .collect();
    let (name, confidence, agreeing) = majority(&observations, settings)?;
    // The whole id; see `public_arity`.
    let id = format!("shape.entrypoint.{}.{ext}", id_fragment(dir));
    let scope = scope_for(dir, ext);
    Some(Convention {
        statement: format!("That public method is named `{name}`"),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &name),
        evidence: evidence(&observations, &name),
        sample_roots: Vec::new(),
        enforcement: canon_core::enforcement_for(&id, &scope, confidence, settings),
        scope,
        id,
    })
}

/// "Types here inherit from `ApplicationService`."
fn base_class(
    dir: &str,
    ext: &str,
    members: &[&FactSet],
    settings: &Settings,
) -> Option<Convention> {
    // Every file with a subject votes, including those whose subject has no
    // base type at all. Counting only the files that already inherit from
    // something makes "they all inherit from X" true by construction.
    let observations: Vec<(Option<String>, f32, &FactSet)> = members
        .iter()
        .filter_map(|s| s.subject().map(|t| (t.superclass.clone(), s.weight, *s)))
        .collect();
    let (base, confidence, agreeing) = majority(&observations, settings)?;
    let winner = base.clone();
    let base = base?;
    // The whole id, and here it decides the answer: `enforcement_for` reads
    // the extension off the end to withhold a refusal from Rust and Python,
    // whose base is whichever of several the author wrote first. A bare
    // `shape.base` hides that extension, so a Python rule was stored
    // `Blocking` and recomputed `Advisory` on every write.
    let id = format!("shape.base.{}.{ext}", id_fragment(dir));
    let scope = scope_for(dir, ext);
    Some(Convention {
        statement: format!("Types here inherit from `{base}`"),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &winner),
        evidence: evidence(&observations, &winner),
        sample_roots: Vec::new(),
        enforcement: canon_core::enforcement_for(&id, &scope, confidence, settings),
        scope,
        id,
    })
}

/// "Types here inherit from a `*BaseController`."
///
/// The fallback when a directory agrees on a kind of base but not on one base.
/// A Rails API namespaces its controllers, so 95 of 102 files inherit
/// something ending `BaseController` while the largest single spelling is 53:
/// the exact vote finds no winner and the directory derives nothing.
///
/// Only stated when `base_class` found nothing, so the two never both speak
/// for one directory. Advisory always: the check is a suffix comparison
/// against a name the sample never contained.
///
/// Withheld from Rust for the same reason [`reads_a_base_from_a_contract`]
/// withholds `base_class` there: `superclass` is a copy of the type's first
/// trait impl, so a shared suffix over it is a family of trait names, not of
/// base types, and "inherit" is not a verb Rust's shapes support at all —
/// unlike `base_class`, this holds whether or not a `contract` rule also
/// fires for the same directory, because the family only exists when the
/// exact names disagree and `contract` is free to pick a different, agreeing
/// trait from the same type's other impls.
fn base_family(
    dir: &str,
    ext: &str,
    members: &[&FactSet],
    settings: &Settings,
) -> Option<Convention> {
    if matches!(canon_extract::lang::from_extension(ext), Some(canon_extract::Language::Rust)) {
        return None;
    }
    let observations: Vec<(Option<String>, f32, &FactSet)> = members
        .iter()
        .filter_map(|s| s.subject().map(|t| (t.superclass.as_deref().map(family_of), s.weight, *s)))
        .collect();
    let (winner, confidence, agreeing) = majority(&observations, settings)?;
    let family = winner.clone()?;
    Some(Convention {
        id: format!("shape.family.{}.{ext}", id_fragment(dir)),
        statement: format!("Types here inherit from a `*{family}`"),
        scope: scope_for(dir, ext),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &winner),
        evidence: evidence(&observations, &winner),
        sample_roots: Vec::new(),
        enforcement: Enforcement::Advisory,
    })
}

/// The last path segment of a base type, which is the part a family shares.
///
/// `Api::V1::Admin::BaseController` and `Api::V1::BaseController` are two
/// namespaces of one family, and the namespace is exactly what differs.
/// Handles every separator canon's languages spell a qualified name with —
/// `::`, `\` and `.` — and a bare name with none of them is returned whole.
///
/// A call expression is returned whole too, because it is not a qualified
/// name at all: `Struct.new(:street, :city)` builds a distinct anonymous type
/// per argument list, which is why the extractor keeps it whole rather than
/// truncating it at the `(`. Split on its separators it yields the family
/// `city)` — a suffix no type ends with, and one two files with different
/// argument lists could never share.
pub(crate) fn family_of(base: &str) -> String {
    if base.contains('(') {
        return base.to_string();
    }
    base.rsplit([':', '\\', '.']).next().unwrap_or(base).to_string()
}

/// "Files here export exactly one function."
///
/// The rule that matters in a component tree, where the unit is a module
/// rather than a class. A React directory has no classes at all, so every
/// class-shaped rule above produces nothing and this is the only Tier 1
/// signal available.
fn module_arity(
    dir: &str,
    ext: &str,
    members: &[&FactSet],
    settings: &Settings,
) -> Option<Convention> {
    let observations: Vec<(usize, f32, &FactSet)> = members
        .iter()
        .filter(|s| s.subject().is_none())
        .map(|s| (s.facts.free_functions.len(), s.weight, *s))
        .collect();
    let (count, confidence, agreeing) = majority(&observations, settings)?;
    if count == 0 {
        return None;
    }
    Some(Convention {
        id: format!("shape.module-arity.{}.{ext}", id_fragment(dir)),
        statement: format!(
            "Files here export exactly {count} function{}",
            if count == 1 { "" } else { "s" }
        ),
        scope: scope_for(dir, ext),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &count),
        evidence: evidence(&observations, &count),
        sample_roots: Vec::new(),
        enforcement: Enforcement::Advisory,
    })
}

/// "Files here call `ApplicationRecord`."
///
/// Who a directory talks to is a layering rule, and the kind no linter checks:
/// a service that reaches past its collaborators into the database is wrong in
/// a way that compiles, passes tests, and is only caught in review.
///
/// Counted by presence rather than by frequency. One file with fifty calls to a
/// logger would otherwise outvote forty files that each call the real
/// collaborator once.
fn collaborator(
    dir: &str,
    ext: &str,
    members: &[&FactSet],
    settings: &Settings,
) -> Option<Convention> {
    let observations: Vec<(Option<String>, f32, &FactSet)> = members
        .iter()
        .map(|s| {
            let mut receivers: Vec<&String> =
                s.facts.calls.iter().filter_map(|c| c.receiver.as_ref()).collect();
            receivers.sort_unstable();
            receivers.dedup();
            // The receiver this file leans on most, once per file.
            let dominant = receivers
                .into_iter()
                .max_by_key(|r| {
                    s.facts.calls.iter().filter(|c| c.receiver.as_ref() == Some(*r)).count()
                })
                .cloned();
            (dominant, s.weight, *s)
        })
        .collect();

    let (winner, confidence, agreeing) = majority(&observations, settings)?;
    let name = winner.clone()?;
    if !is_collaborator(&name) {
        return None;
    }
    Some(Convention {
        id: format!("shape.collaborator.{}.{ext}", id_fragment(dir)),
        statement: format!("Files here call `{name}`"),
        scope: scope_for(dir, ext),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &winner),
        evidence: evidence(&observations, &winner),
        sample_roots: Vec::new(),
        enforcement: Enforcement::Advisory,
    })
}

/// "Files here use `defineProps`."
///
/// A call with no receiver is a macro in every framework that has one: Rails'
/// `has_many`, Django's field constructors, Vue's `defineProps`, React's
/// hooks, Sidekiq's `sidekiq_options`. The queries have captured these all
/// along and [`collaborator`] discarded every one of them, because it reads
/// the receiver and a macro has none: eight identical `<script setup>` Vue
/// components — a shape 672 of nuxt-ui's 720 `.vue` files use — derived two
/// rules between them, and neither was about the component.
///
/// Counted by presence per file. One file calling `useState` nine times is
/// one vote, or a single large component would decide the rule for its
/// directory.
///
/// The name stated is the one [`presence_winner`] finds, for the reason its
/// doc comment gives: a `<script setup>` block calls each of its compiler
/// macros exactly once, so a per-file pick keyed on within-file repetition
/// ties every name at one and resolves the tie to whichever sorts last. The
/// macro the whole directory shares then loses to whichever of a file's own
/// calls happened to sort after it.
///
/// Withheld from a test or spec directory entirely. Measured on a real Rails
/// repository: 97 `shape.macros` rules, a majority of them `context`,
/// `expect`, `it` and `describe` — `RSpec`'s own vocabulary, true of every spec
/// file in the tree and not a convention the team chose. `tests.suffix` and
/// `tests.colocation` already say a spec directory holds tests, so this rule
/// has nothing to add there, and the budget it would spend saying "files here
/// use `describe`" is budget a real macro elsewhere loses.
///
/// Requiring the winning call to carry an argument was measured as the other
/// lever and rejected: `describe 'x' do`, `it 'x' do` and `context 'x' do`
/// all carry a string argument, the same shape as `has_many :listings`, so an
/// argument requirement admits the `RSpec` vocabulary it was meant to exclude.
fn macros(dir: &str, ext: &str, members: &[&FactSet], settings: &Settings) -> Option<Convention> {
    // Checked against each member's own directory rather than `dir`: this
    // group is also derived at every ancestor, and a `__tests__` folder with
    // nothing else beside it makes every one of those ancestors a group of
    // test files too, not only the exact directory named `__tests__`.
    if members.iter().all(|s| crate::tier0::is_test_directory(&s.dir)) {
        return None;
    }
    // Only among the receiverless calls a receiver would otherwise have
    // excluded, which is the same set the vote is counted over.
    let per_file: Vec<(&FactSet, Vec<String>)> = members
        .iter()
        .map(|s| {
            let names = s
                .facts
                .calls
                .iter()
                .filter(|c| c.receiver.is_none() && is_macro(&c.name))
                .map(|c| c.name.clone())
                .collect();
            (*s, names)
        })
        .collect();
    let winner = presence_winner(per_file.iter().map(|(s, names)| (names.as_slice(), s.weight)))?;

    let observations: Vec<(bool, f32, &FactSet)> =
        per_file.iter().map(|(s, names)| (names.contains(&winner), s.weight, *s)).collect();
    let (agrees, confidence, agreeing) = majority(&observations, settings)?;
    if !agrees {
        return None;
    }
    Some(Convention {
        id: format!("shape.macros.{}.{ext}", id_fragment(dir)),
        statement: format!("Files here use `{winner}`"),
        scope: scope_for(dir, ext),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &true),
        evidence: evidence(&observations, &true),
        sample_roots: Vec::new(),
        enforcement: Enforcement::Advisory,
    })
}

/// Whether a receiverless call is a framework macro rather than plumbing.
///
/// Everything not named below is left in: a helper a directory calls
/// unanimously is as much a convention as a framework macro, and the
/// agreement bar decides which one survives. The two exclusions are excluded
/// for unrelated reasons, so each states its own.
pub(crate) fn is_macro(name: &str) -> bool {
    !is_import_keyword(name) && !is_composition_keyword(name)
}

/// A keyword that is already an import fact.
///
/// `require "json"` parses as a call with an argument list in the same query
/// pass that also records it as an import, and counting it again would find
/// every Ruby directory agreeing that it uses `require`.
fn is_import_keyword(name: &str) -> bool {
    matches!(
        name,
        "require" | "require_relative" | "load" | "import" | "include_once" | "require_once"
    )
}

/// A keyword [`mixin`] already states more precisely.
///
/// `` Types here include `Sidekiq::Worker` `` names the module; `` Files here
/// use `include` `` would name only the keyword every Ruby author already
/// knows.
///
/// Two costs, both accepted.
///
/// `calls` comes from the query pass and is unscoped, while
/// `TypeFacts::mixins` is read from the class body only, so this also
/// silences the macro rule for an `include`/`extend`/`prepend` written
/// outside one, which [`mixin`] never sees and so never replaces. That case
/// is narrow in practice: the common `def self.included(base)` hook calls
/// `extend` on a receiver (`base.extend(ClassMethods)`), so it was never a
/// receiverless call to begin with.
///
/// And the test is the name, not the language, so Django pays for Ruby's
/// keyword: a `urls.py` writes `include('blog.urls')`, a real framework macro
/// that is neither an import nor a mixin, and this silences it. Reading the
/// language instead would mean threading it through the checker as well,
/// which is handed one file rather than one directory — and the Django
/// directories that lose `include` still derive `path`, the receiverless call
/// that sits beside every one of them and characterises the file just as
/// well. Admitting `include` by name, meanwhile, would put `` Files here use
/// `include` `` on every Ruby directory in the tree.
fn is_composition_keyword(name: &str) -> bool {
    matches!(name, "include" | "extend" | "prepend")
}

/// "Files here import from `src/config`."
///
/// The highest-value thing canon can say, and the one it had no word for. A
/// wrong import compiles, type-checks and passes review when a plausible
/// alternative exists, which makes it the most common way generated code
/// drifts from a codebase. Nothing else in the vocabulary catches it.
///
/// Counted per file, not per occurrence: a file importing four names from one
/// module is one vote for that module, or a single barrel import would decide
/// the rule for a whole directory.
///
/// Relative paths are excluded. `./thing` names a different module in every
/// directory, so agreement on the literal string is agreement about nothing.
fn import_source(
    dir: &str,
    ext: &str,
    members: &[&FactSet],
    settings: &Settings,
) -> Option<Convention> {
    let observations: Vec<(Option<String>, f32, &FactSet)> = members
        .iter()
        .map(|s| {
            let mut shared: Vec<&String> =
                s.facts.imports.iter().filter(|i| is_shared_module(i)).collect();
            shared.sort_unstable();
            shared.dedup();
            let dominant = shared
                .into_iter()
                .max_by_key(|i| s.facts.imports.iter().filter(|x| x == i).count())
                .cloned();
            (dominant, s.weight, *s)
        })
        .collect();

    let (winner, confidence, agreeing) = majority(&observations, settings)?;
    let module = winner.clone()?;
    Some(Convention {
        id: format!("shape.import.{}.{ext}", id_fragment(dir)),
        statement: format!("Files here import from `{module}`"),
        scope: scope_for(dir, ext),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &winner),
        evidence: evidence(&observations, &winner),
        sample_roots: Vec::new(),
        enforcement: Enforcement::Advisory,
    })
}

/// "Files here carry `@Injectable`."
///
/// Counted by presence per file rather than by occurrence: a controller with
/// nine `@Get` methods is one vote for `@Get`, or one large file would decide
/// the rule for its directory.
///
/// One rule per directory, naming the single most widely carried annotation.
/// A `NestJS` controller carries four or five, and stating all of them turns a
/// four-line block into a wall of text the reader stops reading.
///
/// Carrying four or five is also why the name stated is the one
/// [`presence_winner`] finds. A controller carries `@Controller` once and
/// `@Get`, `@Post` and `@Delete` once each, so every within-file count ties at
/// one; a per-file pick keyed on that repetition resolves every tie to
/// whichever name sorts last, and the one annotation every file in the
/// directory shares loses to a method decorator only some of them carry.
fn annotation(
    dir: &str,
    ext: &str,
    members: &[&FactSet],
    settings: &Settings,
) -> Option<Convention> {
    let per_file: Vec<(&FactSet, Vec<String>)> = members
        .iter()
        .map(|s| (*s, s.facts.annotations.iter().map(|a| a.name.clone()).collect()))
        .collect();
    let winner = presence_winner(per_file.iter().map(|(s, names)| (names.as_slice(), s.weight)))?;

    let observations: Vec<(bool, f32, &FactSet)> =
        per_file.iter().map(|(s, names)| (names.contains(&winner), s.weight, *s)).collect();
    let (agrees, confidence, agreeing) = majority(&observations, settings)?;
    if !agrees {
        return None;
    }
    Some(Convention {
        id: format!("shape.annotation.{}.{ext}", id_fragment(dir)),
        statement: format!("Files here carry `@{winner}`"),
        scope: scope_for(dir, ext),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &true),
        evidence: evidence(&observations, &true),
        sample_roots: Vec::new(),
        // Advisory. A directory of decorated classes can still legitimately
        // gain the one plain helper class that carries nothing.
        enforcement: Enforcement::Advisory,
    })
}

/// The name the most members declare, weighted the way [`majority`] weighs a
/// vote and with ties broken toward the lowest name, so a rebuild is
/// byte-identical.
///
/// Shared by every rule whose facts tie at one occurrence per file. A type
/// essentially never includes the same module twice or implements the same
/// interface twice; a `NestJS` controller carries each of its decorators
/// once; a Vue `<script setup>` block calls each compiler macro once. A
/// per-file pick keyed on that repetition therefore ties every name in a
/// file's list at one, and `max_by_key` resolves every tie by returning the
/// name that sorts last: four files pairing an interface every file in the
/// directory shares with one only they declare would bury the shared,
/// unanimous one behind whichever of the two happened to sort later — a
/// defect in the tie-break, not in the data. Counting a name once per file
/// that declares it at all, regardless of how many others that file names
/// beside it, is the fact a directory's agreement is actually about.
///
/// [`collaborator`] and [`import_source`] keep their own within-file pick,
/// because a receiver a file calls forty times really is the file's
/// collaborator and repetition there is signal rather than a tie.
fn presence_winner<'a>(lists: impl Iterator<Item = (&'a [String], f32)>) -> Option<String> {
    let mut tally: HashMap<&str, f32> = HashMap::new();
    for (names, weight) in lists {
        let mut seen: Vec<&str> = names.iter().map(String::as_str).collect();
        seen.sort_unstable();
        seen.dedup();
        for name in seen {
            *tally.entry(name).or_default() += weight;
        }
    }
    let mut candidates: Vec<(&str, f32)> = tally.into_iter().collect();
    // Highest weight first, then lowest name, so two names tied on weight
    // resolve the same way on every rebuild rather than however the
    // `HashMap` above happened to iterate.
    candidates.sort_by(|a, b| {
        b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.0.cmp(b.0))
    });
    candidates.into_iter().next().map(|(name, _)| name.to_string())
}

/// "Types here include `Sidekiq::Worker`."
///
/// A Sidekiq worker declares no superclass at all and composes its behaviour
/// with `include Sidekiq::Worker` instead; a Laravel job does the same with a
/// trait `use`. Both are invisible to every base-class rule above, which asks
/// what a type inherits and not what it opts into: measured on a real Rails
/// repository, 485 of 490 files in `app/workers` include `Sidekiq::Worker`
/// and derive nothing at all.
///
/// Counted only over files with a resolvable subject, the same restriction
/// [`base_class`] applies: a namespace module or an unrelated nested type
/// composes nothing and must not vote about what one does. The name stated is
/// the one [`presence_winner`] finds: the module the most files in the
/// directory include, not whichever a within-file tie-break prefers.
fn mixin(dir: &str, ext: &str, members: &[&FactSet], settings: &Settings) -> Option<Convention> {
    let subjects: Vec<(&FactSet, &canon_extract::TypeFacts)> =
        members.iter().filter_map(|s| Some((*s, s.subject()?))).collect();
    let winner = presence_winner(subjects.iter().map(|(s, t)| (t.mixins.as_slice(), s.weight)))?;

    let observations: Vec<(bool, f32, &FactSet)> = subjects
        .iter()
        .map(|(s, t)| (t.mixins.iter().any(|m| m == &winner), s.weight, *s))
        .collect();
    let (agrees, confidence, agreeing) = majority(&observations, settings)?;
    if !agrees {
        return None;
    }
    Some(Convention {
        id: format!("shape.mixin.{}.{ext}", id_fragment(dir)),
        statement: format!("Types here include `{winner}`"),
        scope: scope_for(dir, ext),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &true),
        evidence: evidence(&observations, &true),
        sample_roots: Vec::new(),
        // Advisory. A directory that mostly composes one module can still
        // legitimately hold the one type that opts into another, or into
        // none at all.
        enforcement: Enforcement::Advisory,
    })
}

/// "Types here implement `ShouldQueue`."
///
/// `interfaces` has named an `implements` clause since the extractors that
/// fill it were written, and until now nothing derived a rule from it —
/// only one fallback branch in the checker ever read it, after the base
/// class check had already failed. Measured on a real Laravel repository,
/// 119 of 119 files in `app/Jobs` declare `implements ShouldQueue`, and this
/// said nothing about any of them.
///
/// Gated to the languages whose `interfaces` comes from an `implements`
/// clause, so the statement stays true of what it names. Python's leading
/// bases and Go's extra embeds are composition, not a contract, and already
/// land in [`mixin`]'s field instead: their `interfaces` is empty by
/// construction after that split. The gate reads the language rather than
/// trusting that emptiness, so an extractor change that started recording
/// something there could not silently turn composition into a false
/// "implement" statement.
///
/// Counted only over files with a resolvable subject, the same restriction
/// [`mixin`] applies. The name stated is the one [`presence_winner`] finds:
/// the interface the most files in the directory implement, not whichever a
/// within-file tie-break prefers when one type declares several.
fn contract(dir: &str, ext: &str, members: &[&FactSet], settings: &Settings) -> Option<Convention> {
    if !matches!(
        canon_extract::lang::from_extension(ext),
        Some(
            canon_extract::Language::Php
                | canon_extract::Language::TypeScript
                | canon_extract::Language::Tsx
                | canon_extract::Language::Rust
        )
    ) {
        return None;
    }
    let subjects: Vec<(&FactSet, &canon_extract::TypeFacts)> =
        members.iter().filter_map(|s| Some((*s, s.subject()?))).collect();
    let winner =
        presence_winner(subjects.iter().map(|(s, t)| (t.interfaces.as_slice(), s.weight)))?;

    let observations: Vec<(bool, f32, &FactSet)> = subjects
        .iter()
        .map(|(s, t)| (t.interfaces.iter().any(|i| i == &winner), s.weight, *s))
        .collect();
    let (agrees, confidence, agreeing) = majority(&observations, settings)?;
    if !agrees {
        return None;
    }
    Some(Convention {
        id: format!("shape.contract.{}.{ext}", id_fragment(dir)),
        statement: format!("Types here implement `{winner}`"),
        scope: scope_for(dir, ext),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &true),
        evidence: evidence(&observations, &true),
        sample_roots: Vec::new(),
        // Advisory. A directory that agrees on one interface can still
        // legitimately hold the one type that implements another, or none
        // at all.
        enforcement: Enforcement::Advisory,
    })
}

/// Whether this language's base type is one of the contracts the type
/// declares rather than a second fact about it.
///
/// Only Rust. It has no inheritance at all, so the extractor records the first
/// trait impl as the closest analogue of a base and `superclass` is a copy of
/// an entry in `interfaces`. Every other language reads a base and an
/// `implements` clause from two different pieces of syntax, and both are worth
/// stating.
fn reads_a_base_from_a_contract(ext: &str) -> bool {
    matches!(canon_extract::lang::from_extension(ext), Some(canon_extract::Language::Rust))
}

/// Whether an import names something the whole repository can agree about.
///
/// A relative path resolves differently from every directory, so counting the
/// literal string finds agreement where there is none. A package or an
/// absolute module path means the same thing everywhere.
fn is_shared_module(path: &str) -> bool {
    !path.starts_with('.') && path.len() > 1
}

/// Absence of raising was tried as a convention and removed. On a 9,546-file
/// Rails repository it produced six rules, for `spec/`, `vendor/`, `config/`,
/// `db/` and the whole of `app/`, and every one was arithmetic rather than a
/// choice anyone made. The useful half of the idea, "failure is returned as a
/// value", is a positive fact about what a file calls, and [`collaborator`]
/// already carries it.
///
/// Whether a receiver names a collaborator rather than a local variable.
///
/// `Ledger.record(x)` is a layering fact. `listing.save` is a sentence about
/// one method's local, and on a real Rails repository the unfiltered version
/// produced "files here call `listing`", "call `response`", "call `user`":
/// true of the code, and describing nothing anyone chose.
///
/// The test is that the receiver is written like a type. Every language canon
/// parses spells a class in some capitalised or qualified form, and none of
/// them spell a local that way by convention.
fn is_collaborator(receiver: &str) -> bool {
    if receiver.len() < 2 {
        return false;
    }
    // A qualified path is a collaborator whatever its case: `std::fs`, `a.b`.
    if receiver.contains("::") || receiver.contains('.') {
        return true;
    }
    // `self`, `this` and friends are the file talking to itself.
    if matches!(receiver, "self" | "this" | "super" | "cls" | "me") {
        return false;
    }
    receiver.chars().next().is_some_and(char::is_uppercase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use crate::walk::walk;

    fn derive_from(name: &str, files: &[(String, String)]) -> Vec<Convention> {
        let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let root = fixture::build(name, &refs);
        let settings = Settings::default();
        let entries = walk(&root, &settings);
        derive(&gather(&entries, &root), &settings)
    }

    fn joined(convs: &[Convention]) -> String {
        convs.iter().map(|c| c.statement.as_str()).collect::<Vec<_>>().join(" | ")
    }

    #[test]
    fn the_single_public_call_convention_is_derived_from_source() {
        let files = fixture::agreeing(
            "app/services",
            "rb",
            6,
            "class Item$N < ApplicationService\n  def call; end\n  private\n  def helper; end\nend\n",
        );
        let convs = derive_from("sem-services", &files);
        let text = joined(&convs);
        assert!(text.contains("exactly 1 public method"), "got {text}");
        assert!(text.contains("named `call`"), "got {text}");
        assert!(text.contains("`ApplicationService`"), "got {text}");
    }

    #[test]
    fn disagreement_suppresses_the_shape_convention() {
        let mut files = fixture::agreeing("app/s", "rb", 5, "class A$N\n  def call; end\nend\n");
        files.extend((0..5).map(|i| {
            (
                format!("app/s/b{i}.rb"),
                format!(
                    "class B{i}\n  def a; end\n  def b; end\n  def c; end\n  def d; end\nend\n"
                ),
            )
        }));
        let convs = derive_from("sem-split", &files);
        assert!(
            !convs.iter().any(|c| c.id.starts_with("shape.public-arity")),
            "got {}",
            joined(&convs)
        );
    }

    #[test]
    fn a_typescript_service_directory_derives_the_same_shape_as_ruby() {
        // The abstraction holds across visibility models or it is not one.
        let files = fixture::agreeing(
            "src/services",
            "ts",
            6,
            "export class Item$N extends BaseService {\n  call(): void {}\n  private helper(): void {}\n}\n",
        );
        let convs = derive_from("sem-ts", &files);
        let text = joined(&convs);
        assert!(text.contains("exactly 1 public method"), "got {text}");
        assert!(text.contains("named `call`"), "got {text}");
        assert!(text.contains("`BaseService`"), "got {text}");
    }

    #[test]
    fn a_go_package_derives_the_same_shape_through_capitalisation() {
        let files = fixture::agreeing(
            "internal/service",
            "go",
            6,
            "package service\ntype Item$N struct{}\nfunc (s *Item$N) Call() {}\nfunc (s *Item$N) helper() {}\n",
        );
        let convs = derive_from("sem-go", &files);
        let text = joined(&convs);
        assert!(text.contains("exactly 1 public method"), "got {text}");
        assert!(text.contains("named `Call`"), "got {text}");
    }

    #[test]
    fn a_component_directory_derives_a_module_level_rule_with_no_classes() {
        let files =
            fixture::agreeing("src/components", "tsx", 6, "export const Item$N = () => <div/>;\n");
        let convs = derive_from("sem-components", &files);
        assert!(joined(&convs).contains("export exactly 1 function"), "got {}", joined(&convs));
    }

    #[test]
    fn a_directory_of_zero_method_types_produces_no_arity_rule() {
        let files = fixture::agreeing("app/models", "rb", 6, "class Item$N\nend\n");
        let convs = derive_from("sem-empty-types", &files);
        assert!(
            !convs.iter().any(|c| c.id.starts_with("shape.public-arity")),
            "got {}",
            joined(&convs)
        );
    }

    #[test]
    fn a_shared_collaborator_becomes_a_layering_rule() {
        // Who a directory talks to, which is the rule no linter checks.
        let files = fixture::agreeing(
            "app/services",
            "rb",
            6,
            "class Item$N\n  def call\n    Ledger.record(1)\n    Ledger.settle(2)\n  end\nend\n",
        );
        let convs = derive_from("sem-collaborator", &files);
        assert!(joined(&convs).contains("call `Ledger`"), "got {}", joined(&convs));
    }

    #[test]
    fn one_chatty_file_does_not_outvote_the_directory() {
        // Presence, not frequency: fifty log lines in one file must not become
        // the directory's collaborator.
        let mut files = fixture::agreeing(
            "app/services",
            "rb",
            5,
            "class Item$N\n  def call\n    Ledger.record(1)\n  end\nend\n",
        );
        let mut spam = String::new();
        for i in 0..50 {
            spam.push_str(&format!("    Logger.write({i})\n"));
        }
        files.push((
            "app/services/noisy.rb".to_string(),
            format!("class Noisy\n  def call\n{spam}  end\nend\n"),
        ));
        let convs = derive_from("sem-chatty", &files);
        let text = joined(&convs);
        assert!(!text.contains("call `Logger`"), "one file dominated the count: {text}");
    }

    #[test]
    fn a_shared_import_becomes_a_convention() {
        // Issue #7. A wrong import compiles and type-checks; nothing else in
        // the vocabulary catches it.
        let files = fixture::agreeing(
            "src/queries",
            "ts",
            6,
            "import { client } from 'src/config';\nexport const q$N = () => client;\n",
        );
        let convs = derive_from("sem-import", &files);
        assert!(joined(&convs).contains("import from `src/config`"), "got {}", joined(&convs));
    }

    #[test]
    fn a_relative_import_is_not_a_convention() {
        // `./thing` names a different module in every directory, so agreement
        // on the literal string is agreement about nothing.
        let files = fixture::agreeing(
            "src/queries",
            "ts",
            6,
            "import { a } from './thing';\nexport const q$N = () => a;\n",
        );
        let convs = derive_from("sem-relimport", &files);
        assert!(!joined(&convs).contains("import from"), "got {}", joined(&convs));
    }

    #[test]
    fn a_barrel_import_does_not_decide_the_rule_alone() {
        // Counted per file, not per occurrence: one file importing six names
        // from a module must not outvote five files importing another.
        let mut files: Vec<(String, String)> = (0..5)
            .map(|i| {
                (
                    format!("src/q/a{i}.ts"),
                    "import { x } from 'pkg-common';\nexport const a = () => x;\n".to_string(),
                )
            })
            .collect();
        let mut barrel = String::new();
        for i in 0..6 {
            barrel.push_str(&format!("import {{ n{i} }} from 'pkg-rare';\n"));
        }
        files.push(("src/q/barrel.ts".to_string(), format!("{barrel}export const b = () => 1;\n")));
        let convs = derive_from("sem-barrel", &files);
        assert!(!joined(&convs).contains("pkg-rare"), "one file decided it: {}", joined(&convs));
    }

    #[test]
    fn a_component_directory_derives_the_export_style_it_holds() {
        // Issue #16. Default versus named is the choice a component tree makes
        // and holds, and the one a generated file gets wrong in a way that
        // compiles: every import of it then has to be written the other way.
        let files = fixture::agreeing(
            "src/components",
            "tsx",
            6,
            "const Item$N = () => <div/>;\nexport default Item$N;\n",
        );
        let convs = derive_from("sem-default-export", &files);
        assert!(joined(&convs).contains("export a default"), "got {}", joined(&convs));
    }

    #[test]
    fn a_named_export_directory_derives_the_other_answer() {
        let files =
            fixture::agreeing("src/widgets", "tsx", 6, "export const Item$N = () => <div/>;\n");
        let convs = derive_from("sem-named-export", &files);
        assert!(joined(&convs).contains("named export"), "got {}", joined(&convs));
    }

    #[test]
    fn a_split_between_the_two_styles_states_neither() {
        let mut files =
            fixture::agreeing("src/mixed", "tsx", 5, "export const ItemA$N = () => <div/>;\n");
        files.extend((0..5).map(|i| {
            (
                format!("src/mixed/b{i}.tsx"),
                format!("const ItemB{i} = () => <div/>;\nexport default ItemB{i};\n"),
            )
        }));
        let convs = derive_from("sem-split-export", &files);
        assert!(!convs.iter().any(|c| c.id.starts_with("shape.export")), "got {}", joined(&convs));
    }

    #[test]
    fn a_language_with_no_default_export_says_nothing_about_one() {
        // Ruby has no such concept, so counting `false` across a services
        // directory would state a unanimous rule nobody ever chose.
        let files =
            fixture::agreeing("app/services", "rb", 6, "class Item$N\n  def call; end\nend\n");
        let convs = derive_from("sem-rb-export", &files);
        assert!(!convs.iter().any(|c| c.id.starts_with("shape.export")), "got {}", joined(&convs));
    }

    #[test]
    fn a_php_directory_derives_the_namespace_it_declares() {
        // Issue #16. PSR-4 makes namespace and directory agree, and 134 tracked
        // PHP files derived nothing at all because every rule in the vocabulary
        // asked about base classes and method counts.
        let files = fixture::agreeing(
            "src/Services/Billing",
            "php",
            6,
            "<?php\nnamespace App\\Services\\Billing;\nclass Item$N { public function handle() {} }\n",
        );
        let convs = derive_from("sem-php-namespace", &files);
        assert!(
            joined(&convs).contains("namespace `App\\Services\\Billing`"),
            "got {}",
            joined(&convs)
        );
    }

    #[test]
    fn a_namespace_deeper_than_the_grouping_limit_is_still_derived() {
        // The shared grouping stops four directories down, and PSR-4 trees run
        // deeper: a WordPress theme's `lib/jwt` sits five levels in, and its
        // five namespaced files had no group of their own to be counted in.
        let files = fixture::agreeing(
            "wp-content/themes/site/lib/jwt",
            "php",
            6,
            "<?php\nnamespace Theme\\Lib\\Jwt;\nclass Item$N { public function handle() {} }\n",
        );
        let convs = derive_from("sem-php-deep", &files);
        let rule = convs
            .iter()
            .find(|c| c.id.starts_with("shape.namespace"))
            .unwrap_or_else(|| panic!("no namespace rule in {}", joined(&convs)));
        assert!(rule.statement.ends_with("`Theme\\Lib\\Jwt`"), "got {}", rule.statement);
        // The scope is the half that was wrong, and the half that matters: the
        // shared grouping stopped at the fourth directory, so the rule landed
        // on `.../lib` and named a directory whose files it had never counted.
        // A namespace rule scoped anywhere but its own directory is inert,
        // because both halves now ask it to name exactly that directory.
        assert_eq!(
            rule.scope,
            canon_core::Scope::DirExt("wp-content/themes/site/lib/jwt".into(), "php".into()),
            "the rule names a directory other than the one it counted"
        );
    }

    #[test]
    fn a_parent_namespace_is_not_derived_from_its_children() {
        // Every file here is correct under PSR-4, so there is no disagreement
        // to report and no parent-level answer that would be true of them.
        let mut files = fixture::agreeing(
            "src/Services/Billing",
            "php",
            6,
            "<?php\nnamespace App\\Services\\Billing;\nclass ItemA$N { public function handle() {} }\n",
        );
        files.extend(fixture::agreeing(
            "src/Services/Billing/Invoices",
            "php",
            6,
            "<?php\nnamespace App\\Services\\Billing\\Invoices;\nclass ItemB$N { public function handle() {} }\n",
        ));
        let convs = derive_from("sem-php-nested", &files);
        let namespaces: Vec<&str> = convs
            .iter()
            .filter(|c| c.id.starts_with("shape.namespace"))
            .map(|c| c.statement.as_str())
            .collect();
        assert_eq!(namespaces.len(), 2, "got {namespaces:?}");
        assert!(namespaces.iter().any(|s| s.ends_with("`App\\Services\\Billing`")));
        assert!(namespaces.iter().any(|s| s.ends_with("`App\\Services\\Billing\\Invoices`")));
    }

    #[test]
    fn a_sample_below_the_gate_produces_nothing() {
        let files = fixture::agreeing("app/s", "rb", 3, "class A$N\n  def call; end\nend\n");
        let convs = derive_from("sem-small", &files);
        assert!(convs.is_empty(), "got {}", joined(&convs));
    }

    #[test]
    fn a_tied_tally_breaks_toward_the_lowest_value_on_every_run() {
        // A plain `HashMap` iterates in an order this process randomises at
        // startup, so a `max_by` with no tie-break picks a different winner
        // from one run to the next. Run several independent tallies rather
        // than one, so a tie-break that fell back to iteration order would
        // show it here instead of passing by chance on a single `HashMap`.
        for _ in 0..50 {
            let mut tally: HashMap<&str, f32> = HashMap::new();
            tally.insert("zebra", 5.0);
            tally.insert("apple", 5.0);
            tally.insert("mango", 5.0);
            assert_eq!(tally_winner(tally), Some(("apple", 5.0)));
        }
    }

    #[test]
    fn an_unparseable_file_is_skipped_without_losing_the_others() {
        let mut files = fixture::agreeing("app/s", "rb", 6, "class A$N\n  def call; end\nend\n");
        files.push(("app/s/broken.rb".to_string(), "class \u{0}\u{1} def def".to_string()));
        let convs = derive_from("sem-broken", &files);
        assert!(joined(&convs).contains("exactly 1 public method"), "got {}", joined(&convs));
    }

    #[test]
    fn the_exemplar_points_at_a_file_that_agrees() {
        let files =
            fixture::agreeing("app/s", "rb", 6, "class Item$N < Base\n  def call; end\nend\n");
        let convs = derive_from("sem-exemplar", &files);
        let arity = convs.iter().find(|c| c.id.starts_with("shape.public-arity")).expect("a rule");
        let exemplar = arity.exemplar.as_deref().expect("an exemplar");
        assert!(exemplar.starts_with("app/s/item"), "got {exemplar}");
    }

    #[test]
    fn a_nest_service_directory_derives_its_decorator() {
        let files = fixture::agreeing(
            "src/orders",
            "ts",
            6,
            "import { Injectable } from '@nestjs/common';\n\n@Injectable()\nexport class Svc$N {\n  findAll(): void {}\n}\n",
        );
        let convs = derive_from("sem-nest", &files);
        let text = joined(&convs);
        assert!(text.contains("carry `@Injectable`"), "got {text}");
    }

    #[test]
    fn a_vue_component_directory_derives_its_compiler_macros() {
        // `<script setup>` declares no class, no export and no free function, so
        // every other rule in the vocabulary is silent about it.
        let files = fixture::agreeing(
            "src/components",
            "vue",
            6,
            "<template><div/></template>\n<script setup lang=\"ts\">\nimport { ref } from 'vue'\nconst props = defineProps<{ title: string }>()\nconst open = ref(false)\n</script>\n",
        );
        let convs = derive_from("sem-vue-macros", &files);
        let text = joined(&convs);
        assert!(text.contains("use `defineProps`") || text.contains("use `ref`"), "got {text}");
    }

    #[test]
    fn an_import_keyword_is_not_a_macro() {
        // Ruby's `require "json"` parses as a call with an argument list, and it
        // is already an import fact. Counting it again would make every Ruby
        // directory agree that it "uses `require`".
        let files = fixture::agreeing(
            "app/x",
            "rb",
            6,
            "require 'json'\nclass A$N\n  def call; end\nend\n",
        );
        let convs = derive_from("sem-require", &files);
        assert!(!joined(&convs).contains("use `require`"), "got {}", joined(&convs));
    }

    #[test]
    fn a_shared_macro_is_counted_by_presence_not_occurrence() {
        // One component calling `useState` nine times must be one vote, or a
        // single large file decides its directory's rule.
        let mut files = fixture::agreeing(
            "src/hooks",
            "tsx",
            5,
            "export const Item$N = () => {\n  useState(0);\n  return null;\n};\n",
        );
        let mut spam = String::new();
        for i in 0..50 {
            spam.push_str(&format!("  useMemo({i});\n"));
        }
        files.push((
            "src/hooks/noisy.tsx".to_string(),
            format!("export const Noisy = () => {{\n{spam}  return null;\n}};\n"),
        ));
        let convs = derive_from("sem-macro-chatty", &files);
        let text = joined(&convs);
        assert!(!text.contains("use `useMemo`"), "one file dominated the count: {text}");
    }

    #[test]
    fn a_directory_with_no_receiverless_calls_derives_no_macro_rule() {
        // A struct with no calls at all has made no choice, and stating
        // agreement about that absence would be inventing a convention nobody
        // holds — the same defect raising's removal fixed for `collaborator`.
        let files = fixture::agreeing(
            "internal/model",
            "rs",
            6,
            "pub struct Item$N {\n    pub id: u64,\n}\n",
        );
        let convs = derive_from("sem-no-macro", &files);
        assert!(!convs.iter().any(|c| c.id.starts_with("shape.macros")), "got {}", joined(&convs));
    }

    #[test]
    fn a_spec_directory_derives_no_macro_rule_from_the_test_frameworks_own_vocabulary() {
        // Measured on a real Rails repository: `shape.macros` derived 97
        // rules, most of them `context`, `expect`, `it` and `describe` —
        // true of every spec file and not a convention the team chose.
        let files = fixture::agreeing(
            "spec/services",
            "rb",
            6,
            "describe Item$N do\n  it 'works' do\n    expect(1).to eq(1)\n  end\nend\n",
        );
        let convs = derive_from("sem-spec-macro", &files);
        assert!(!convs.iter().any(|c| c.id.starts_with("shape.macros")), "got {}", joined(&convs));
    }

    #[test]
    fn a_test_directory_named_test_derives_no_macro_rule_either() {
        let files = fixture::agreeing(
            "src/components/__tests__",
            "tsx",
            6,
            "describe('Item$N', () => {\n  it('renders', () => {\n    expect(1).toBe(1);\n  });\n});\n",
        );
        let convs = derive_from("sem-tests-dir-macro", &files);
        assert!(!convs.iter().any(|c| c.id.starts_with("shape.macros")), "got {}", joined(&convs));
    }

    #[test]
    fn a_model_directory_still_derives_the_association_it_shares() {
        // The rule a spec directory must not drown out: a real framework
        // macro that carries information nothing else in the vocabulary
        // states.
        let files = fixture::agreeing(
            "app/models",
            "rb",
            6,
            "class Item$N < ApplicationRecord\n  has_many :listings\nend\n",
        );
        let convs = derive_from("sem-model-macro", &files);
        assert!(joined(&convs).contains("use `has_many`"), "got {}", joined(&convs));
    }

    #[test]
    fn four_base_controllers_are_one_family() {
        let mut files: Vec<(String, String)> = Vec::new();
        for (i, base) in [
            "Api::V1::BaseController",
            "Api::V1::BaseController",
            "Api::V1::BaseController",
            "Api::V1::Admin::BaseController",
            "Api::V1::Admin::BaseController",
            "Api::V1::ChromeExtension::BaseController",
        ]
        .iter()
        .enumerate()
        {
            files.push((
                format!("app/controllers/c{i}_controller.rb"),
                format!("class C{i}Controller < {base}\n  def index; end\nend\n"),
            ));
        }
        let convs = derive_from("sem-family", &files);
        let text = joined(&convs);
        assert!(text.contains("inherit from a `*BaseController`"), "got {text}");
    }

    #[test]
    fn a_family_rule_never_refuses_a_write() {
        // The check is a suffix comparison against a name the sample never
        // contained, which is exactly the check that can be wrong about a
        // legitimate file.
        let settings = canon_core::Settings::default();
        let total = canon_core::Confidence::derive(6, 6).expect("total");
        assert_eq!(
            canon_core::enforcement_for(
                "shape.family.app.controllers.rb",
                &canon_core::Scope::DirExt("app/controllers".into(), "rb".into()),
                total,
                &settings
            ),
            canon_core::Enforcement::Advisory
        );
    }

    #[test]
    fn a_family_rule_yields_to_a_directory_where_one_base_already_won() {
        // The family fallback only ever speaks when `base_class` found no
        // winner; a directory that already agrees on one exact base must not
        // gain a second, redundant rule about the same thing.
        let files = fixture::agreeing(
            "app/services",
            "rb",
            6,
            "class Item$N < ApplicationService\n  def call; end\nend\n",
        );
        let convs = derive_from("sem-family-yields", &files);
        assert!(!convs.iter().any(|c| c.id.starts_with("shape.family")), "got {}", joined(&convs));
    }

    #[test]
    fn a_directory_with_no_shared_suffix_states_no_family() {
        // Six unrelated base classes with no common suffix at all is not a
        // family with a low-confidence winner, it is a directory with no
        // agreement, and the majority gate that already governs `base_class`
        // has to withhold a rule here for the same reason it withholds one
        // there — not invent one from whichever base happened to repeat once
        // more than the others.
        let mut files: Vec<(String, String)> = Vec::new();
        for (i, base) in ["Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot", "Golf", "Hotel"]
            .iter()
            .enumerate()
        {
            files.push((
                format!("app/controllers/c{i}_controller.rb"),
                format!("class C{i}Controller < {base}\n  def index; end\nend\n"),
            ));
        }
        let convs = derive_from("sem-family-no-agreement", &files);
        assert!(!convs.iter().any(|c| c.id.starts_with("shape.family")), "got {}", joined(&convs));
    }

    #[test]
    fn a_rust_directory_derives_no_family_rule_even_when_trait_names_share_a_suffix() {
        // Rust has no inheritance at all, so "Types here inherit from a
        // `*Handler`" is not a sentence this language's shapes can make
        // true, however many of its trait names happen to share a suffix.
        // `base_class` already withholds the exact form for the same reason
        // `contract` gives it the verb "implement" instead; `base_family` is
        // the same fact one step blurrier and has to yield the same way.
        let mut files: Vec<(String, String)> = Vec::new();
        for (i, module) in ["mod_a", "mod_a", "mod_a", "mod_b", "mod_b", "mod_c"].iter().enumerate()
        {
            files.push((
                format!("src/handlers/item{i}.rs"),
                format!(
                    "pub struct Item{i};\n\nimpl {module}::Handler for Item{i} {{\n    fn handle(&self) {{}}\n}}\n"
                ),
            ));
        }
        let convs = derive_from("sem-rs-no-family", &files);
        assert!(!convs.iter().any(|c| c.id.starts_with("shape.family")), "got {}", joined(&convs));
    }

    #[test]
    fn family_of_reads_the_last_segment_of_every_separator_canon_writes() {
        assert_eq!(family_of("Api::V1::BaseController"), "BaseController");
        assert_eq!(family_of("App\\Http\\Controllers\\BaseController"), "BaseController");
        assert_eq!(family_of("controllers.base.BaseController"), "BaseController");
        assert_eq!(family_of("BaseController"), "BaseController");
    }

    #[test]
    fn a_call_expression_base_keeps_the_shape_the_extractor_gave_it() {
        // The extractor deliberately keeps a call-expression base whole,
        // because each argument list builds a different anonymous type. This
        // split it again at the last separator it could find, so
        // `Struct.new(:street, :city)` came out as the family `city)`.
        assert_eq!(family_of("Struct.new(:street, :city)"), "Struct.new(:street, :city)");
        assert_eq!(family_of("Data.define(x: Integer)"), "Data.define(x: Integer)");
        assert_eq!(family_of("namedtuple('Point', ['x', 'y'])"), "namedtuple('Point', ['x', 'y'])");
    }

    #[test]
    fn a_sidekiq_worker_derives_the_module_it_includes() {
        // Measured on a real Rails repository: 485 of 490 files in
        // `app/workers` declare no superclass at all and compose their
        // behaviour with `include Sidekiq::Worker` instead, so `base_class`
        // above sees nothing about any of them.
        let files = fixture::agreeing(
            "app/workers",
            "rb",
            6,
            "class Item$N\n  include Sidekiq::Worker\n\n  def perform; end\nend\n",
        );
        let convs = derive_from("sem-mixin", &files);
        assert!(joined(&convs).contains("include `Sidekiq::Worker`"), "got {}", joined(&convs));
    }

    #[test]
    fn a_unanimous_mixin_is_not_buried_by_one_only_some_types_also_include() {
        // The same defect the contract rule's equivalent test pins, for the
        // sibling rule that shares its dominant-pick logic: every one of six
        // files includes `Sidekiq::Worker`; four of them also include
        // `Zeitwerk::Loader`, which sorts after it. A per-file pick keyed on
        // in-file repetition ties both names at one occurrence and resolves
        // the tie to whichever sorts last, so those four files would vote
        // `Zeitwerk::Loader` and the true 6/6 consensus on `Sidekiq::Worker`
        // would never be counted.
        let mut files: Vec<(String, String)> = (0..4)
            .map(|i| {
                (
                    format!("app/workers/multi{i}.rb"),
                    format!(
                        "class Multi{i}\n  include Sidekiq::Worker\n  include Zeitwerk::Loader\n\n  def perform; end\nend\n"
                    ),
                )
            })
            .collect();
        files.extend((0..2).map(|i| {
            (
                format!("app/workers/plain{i}.rb"),
                format!("class Plain{i}\n  include Sidekiq::Worker\n\n  def perform; end\nend\n"),
            )
        }));
        let convs = derive_from("sem-mixin-unanimous", &files);
        assert!(joined(&convs).contains("include `Sidekiq::Worker`"), "got {}", joined(&convs));
    }

    #[test]
    fn a_type_with_no_mixins_derives_no_mixin_rule() {
        let files =
            fixture::agreeing("app/models", "rb", 6, "class Item$N < ApplicationRecord\nend\n");
        let convs = derive_from("sem-no-mixin", &files);
        assert!(!convs.iter().any(|c| c.id.starts_with("shape.mixin")), "got {}", joined(&convs));
    }

    #[test]
    fn a_class_body_include_does_not_also_derive_a_macro_rule() {
        // `include` is a fact `shape.mixin` already carries, more precisely
        // than `shape.macros` ever could: `Types here include
        // `Sidekiq::Worker`` names the module, where `Files here use
        // `include`` would only name the keyword every Ruby author already
        // knows.
        let files = fixture::agreeing(
            "app/workers",
            "rb",
            6,
            "class Item$N\n  include Sidekiq::Worker\n\n  def perform; end\nend\n",
        );
        let convs = derive_from("sem-mixin-not-macro", &files);
        let text = joined(&convs);
        assert!(text.contains("include `Sidekiq::Worker`"), "got {text}");
        assert!(
            !convs
                .iter()
                .any(|c| c.id.starts_with("shape.macros") && c.statement.contains("`include`")),
            "got {text}"
        );
    }

    #[test]
    fn a_laravel_job_directory_derives_its_queue_contract() {
        // Measured on a real Laravel repository: 119 of 119 files in
        // `app/Jobs` declare `implements ShouldQueue`, and `interfaces` had
        // been extracted for the life of the extractor without deriving
        // anything from it.
        let files = fixture::agreeing(
            "app/Jobs",
            "php",
            6,
            "<?php\nnamespace App\\Jobs;\nclass Job$N implements ShouldQueue\n{\n    public function handle() {}\n}\n",
        );
        let convs = derive_from("sem-contract", &files);
        assert!(joined(&convs).contains("implement `ShouldQueue`"), "got {}", joined(&convs));
    }

    #[test]
    fn a_ruby_directory_derives_no_contract_rule() {
        // Ruby has no `implements` clause at all; whatever composition it
        // does is already `mixin`'s to state, never this rule's.
        let files = fixture::agreeing(
            "app/workers",
            "rb",
            6,
            "class Item$N\n  include Sidekiq::Worker\n\n  def perform; end\nend\n",
        );
        let convs = derive_from("sem-contract-rb", &files);
        assert!(
            !convs.iter().any(|c| c.id.starts_with("shape.contract")),
            "got {}",
            joined(&convs)
        );
    }

    #[test]
    fn a_python_directory_derives_no_contract_rule_even_if_interfaces_were_populated() {
        // Python's `interfaces` is empty by construction today; its leading
        // bases land in `mixins` instead. The gate reads the language rather
        // than trusting that emptiness, so an extractor change that started
        // recording something there could not silently produce a false
        // "implement" statement — proven here by handing the rule a
        // `FactSet` with `interfaces` populated directly, bypassing the
        // extractor entirely.
        let members: Vec<FactSet> = (0..6)
            .map(|i| {
                let facts = FileFacts {
                    types: vec![canon_extract::TypeFacts {
                        name: format!("Item{i}"),
                        interfaces: vec!["ShouldQueue".to_string()],
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                FactSet {
                    rel: format!("app/jobs/item{i}.py"),
                    dir: "app/jobs".to_string(),
                    ext: "py".to_string(),
                    weight: 1.0,
                    modified_unix: 0,
                    stem: format!("item{i}"),
                    facts,
                }
            })
            .collect();
        let refs: Vec<&FactSet> = members.iter().collect();
        let settings = Settings::default();
        assert!(contract("app/jobs", "py", &refs, &settings).is_none());
    }

    #[test]
    fn a_unanimous_interface_is_not_buried_by_one_only_some_types_also_declare() {
        // Every one of six files implements `Loggable`; four of them also
        // implement `ZExtra`, which sorts after it. A per-file pick keyed on
        // in-file repetition ties both names at one occurrence and resolves
        // the tie to whichever sorts last, so those four files would vote
        // `ZExtra` and the true 6/6 consensus on `Loggable` would never be
        // counted at all — `ZExtra`'s 4/6 falls under the confidence floor,
        // so the directory would derive nothing rather than name the
        // interface every file in it actually shares.
        let mut files: Vec<(String, String)> = (0..4)
            .map(|i| {
                (
                    format!("app/Jobs/Multi{i}.php"),
                    format!(
                        "<?php\nclass Multi{i} implements Loggable, ZExtra\n{{\n    public function handle() {{}}\n}}\n"
                    ),
                )
            })
            .collect();
        files.extend((0..2).map(|i| {
            (
                format!("app/Jobs/Plain{i}.php"),
                format!(
                    "<?php\nclass Plain{i} implements Loggable\n{{\n    public function handle() {{}}\n}}\n"
                ),
            )
        }));
        let convs = derive_from("sem-contract-unanimous", &files);
        assert!(joined(&convs).contains("implement `Loggable`"), "got {}", joined(&convs));
    }

    #[test]
    fn a_typescript_class_directory_derives_the_interface_it_implements() {
        let files = fixture::agreeing(
            "src/handlers",
            "ts",
            6,
            "export class Handler$N implements RequestHandler {\n  handle(): void {}\n}\n",
        );
        let convs = derive_from("sem-contract-ts", &files);
        assert!(joined(&convs).contains("implement `RequestHandler`"), "got {}", joined(&convs));
    }

    #[test]
    fn a_rust_directory_derives_the_trait_it_implements() {
        let files = fixture::agreeing(
            "src/models",
            "rs",
            6,
            "pub struct Item$N;\n\nimpl Loggable for Item$N {\n    fn log(&self) {}\n}\n",
        );
        let convs = derive_from("sem-contract-rs", &files);
        assert!(joined(&convs).contains("implement `Loggable`"), "got {}", joined(&convs));
    }

    #[test]
    fn a_rust_trait_impl_is_stated_once_rather_than_as_a_base_as_well() {
        // A Rust type's `superclass` is its first trait impl, so `base_class`
        // and `contract` read one fact and state it twice: "Types here
        // inherit from `Loggable`" beside "Types here implement `Loggable`",
        // two injected lines and two claims nothing can collapse. A trait
        // impl is not inheritance, and `contract` is the honest sentence.
        let files = fixture::agreeing(
            "src/models",
            "rs",
            6,
            "pub struct Item$N;\n\nimpl Loggable for Item$N {\n    fn log(&self) {}\n}\n",
        );
        let convs = derive_from("sem-rs-one-fact", &files);
        assert!(joined(&convs).contains("implement `Loggable`"), "got {}", joined(&convs));
        assert!(
            !convs.iter().any(|c| c.id.starts_with("shape.base")),
            "one fact stated twice: {}",
            joined(&convs)
        );
    }

    #[test]
    fn a_ruby_base_class_is_not_suppressed_by_a_contract_rule() {
        // The suppression is about Rust, where one fact fills both fields.
        // Every other language reads a base and an `implements` clause from
        // two different pieces of syntax, and both remain worth stating.
        let files = fixture::agreeing(
            "app/services",
            "rb",
            6,
            "class Item$N < ApplicationService\n  def call; end\nend\n",
        );
        let convs = derive_from("sem-rb-base-kept", &files);
        assert!(
            joined(&convs).contains("inherit from `ApplicationService`"),
            "got {}",
            joined(&convs)
        );
    }

    #[test]
    fn a_type_with_no_interfaces_derives_no_contract_rule() {
        let files =
            fixture::agreeing("src/models", "ts", 6, "export class Item$N {\n  go(): void {}\n}\n");
        let convs = derive_from("sem-no-contract", &files);
        assert!(
            !convs.iter().any(|c| c.id.starts_with("shape.contract")),
            "got {}",
            joined(&convs)
        );
    }

    #[test]
    fn an_annotation_only_a_minority_carries_is_not_a_convention() {
        // Six carrying it against six that do not, so the sample clears
        // `min_files` on both sides and the abstention is the agreement gate
        // refusing a half-and-half split rather than arithmetic about a
        // sample too small to count.
        let mut files: Vec<(String, String)> = (0..6)
            .map(|i| {
                (
                    format!("src/x/plain{i}.ts"),
                    format!("export class Plain{i} {{\n  go(): void {{}}\n}}\n"),
                )
            })
            .collect();
        files.extend((0..6).map(|i| {
            (
                format!("src/x/decorated{i}.ts"),
                format!("@Injectable()\nexport class Decorated{i} {{\n  go(): void {{}}\n}}\n"),
            )
        }));
        let convs = derive_from("sem-annot-minority", &files);
        assert!(
            !convs.iter().any(|c| c.id.starts_with("shape.annotation")),
            "got {}",
            joined(&convs)
        );
    }

    #[test]
    fn the_annotation_every_file_shares_beats_the_one_that_sorts_last() {
        // The shape the rule was written for. A `NestJS` controller carries
        // `@Controller` once and a different mix of `@Get`, `@Post` and
        // `@Delete` beside it, so every within-file count ties at one. A
        // per-file pick keyed on that repetition resolves every tie to
        // whichever name sorts last, so each file votes for a method
        // decorator only some of them carry and the one decorator all six
        // share is never counted at all.
        let bodies = [
            "  @Get()\n  findAll(): void {}\n",
            "  @Post()\n  create(): void {}\n",
            "  @Delete()\n  remove(): void {}\n",
            "  @Get()\n  findAll(): void {}\n\n  @Post()\n  create(): void {}\n",
            "  @Post()\n  create(): void {}\n\n  @Delete()\n  remove(): void {}\n",
            "  @Get()\n  findAll(): void {}\n\n  @Delete()\n  remove(): void {}\n",
        ];
        let files: Vec<(String, String)> = bodies
            .iter()
            .enumerate()
            .map(|(i, body)| {
                (
                    format!("src/orders/c{i}.controller.ts"),
                    format!("@Controller('orders')\nexport class C{i}Controller {{\n{body}}}\n"),
                )
            })
            .collect();
        let convs = derive_from("sem-nest-controller", &files);
        let rule = convs
            .iter()
            .find(|c| c.id.starts_with("shape.annotation"))
            .unwrap_or_else(|| panic!("no annotation rule in {}", joined(&convs)));
        assert!(rule.statement.contains("`@Controller`"), "got {}", rule.statement);
        assert_eq!(rule.agreeing, 6, "the shared decorator was not counted in every file");
    }

    #[test]
    fn the_macro_every_file_shares_beats_the_one_that_sorts_last() {
        // The same tie-break, for the sibling family. A `<script setup>`
        // block calls each compiler macro once, so every within-file count
        // ties at one and the pick fell to whichever name sorted last.
        let bodies = [
            "const open = ref(false)\n",
            "const items = reactive([])\n",
            "const total = computed(() => 1)\n",
            "const open = ref(false)\nconst items = reactive([])\n",
            "const items = reactive([])\nconst total = computed(() => 1)\n",
            "const open = ref(false)\nconst total = computed(() => 1)\n",
        ];
        let files: Vec<(String, String)> = bodies
            .iter()
            .enumerate()
            .map(|(i, body)| {
                (
                    format!("src/components/C{i}.vue"),
                    format!(
                        "<template><div/></template>\n<script setup lang=\"ts\">\nconst props = defineProps<{{ title: string }}>()\n{body}</script>\n"
                    ),
                )
            })
            .collect();
        let convs = derive_from("sem-vue-shared-macro", &files);
        let rule = convs
            .iter()
            .find(|c| c.id.starts_with("shape.macros"))
            .unwrap_or_else(|| panic!("no macro rule in {}", joined(&convs)));
        assert!(rule.statement.contains("`defineProps`"), "got {}", rule.statement);
        assert_eq!(rule.agreeing, 6, "the shared macro was not counted in every file");
    }

    #[test]
    fn a_directory_deeper_than_four_levels_derives_a_rule_of_its_own() {
        // A workspace holding several checkouts prefixes every path with the
        // checkout name, so `api/app/services/billing` is already four levels
        // down and everything below it had no group at all.
        let files = fixture::agreeing(
            "api/app/services/billing/invoices",
            "rb",
            6,
            "class Item$N < InvoiceService\n  def call; end\nend\n",
        );
        let convs = derive_from("sem-deep-group", &files);
        assert!(
            convs.iter().any(|c| crate::scope_dir_of(c) == "api/app/services/billing/invoices"),
            "nothing was derived at the fifth level: {:?}",
            convs.iter().map(|c| c.scope.render()).collect::<Vec<_>>()
        );
    }
}
