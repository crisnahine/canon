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

/// Ancestor depth past which grouping stops paying for itself.
const MAX_GROUP_DEPTH: usize = 4;
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
    let mut groups: HashMap<(String, String), Vec<&FactSet>> = HashMap::new();
    for s in sets {
        let mut acc = String::new();
        let mut keys = vec![String::new()];
        for segment in s.dir.split('/').filter(|x| !x.is_empty()).take(MAX_GROUP_DEPTH) {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(segment);
            keys.push(acc.clone());
        }
        for dir in keys {
            groups.entry((dir, s.ext.clone())).or_default().push(s);
        }
    }

    let mut out = Vec::new();
    for ((dir, ext), members) in groups {
        // Shape is a property of a place in the tree, never of a whole
        // repository. Derived repository-wide on a Rails codebase the
        // migrations outnumber everything else, producing "the public method
        // here is named `change`" for every Ruby file in the project.
        if dir.is_empty() {
            continue;
        }
        out.extend(public_arity(&dir, &ext, &members, settings));
        out.extend(entrypoint_name(&dir, &ext, &members, settings));
        let base = base_class(&dir, &ext, &members, settings);
        if base.is_none() {
            out.extend(base_family(&dir, &ext, &members, settings));
        }
        out.extend(base);
        out.extend(module_arity(&dir, &ext, &members, settings));
        out.extend(collaborator(&dir, &ext, &members, settings));
        out.extend(macros(&dir, &ext, &members, settings));
        out.extend(import_source(&dir, &ext, &members, settings));
        out.extend(export_style(&dir, &ext, &members, settings));
        out.extend(annotation(&dir, &ext, &members, settings));
    }
    out.extend(namespace_per_directory(sets, settings));
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
fn majority<T: std::hash::Hash + Eq + Clone>(
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
    let (winner, weight) = tally
        .into_iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
    // The configured floor is not applied here. Applied during the vote it
    // removed the wide rule before `collapse_redundant` could use it, and every
    // narrow rule the wide one would have absorbed survived instead; see
    // `derive_from`, which filters the finished set.
    let confidence = Confidence::derive_counted(weight, total, observations.len())?;
    let agreeing = observations.iter().filter(|(v, _, _)| *v == winner).count();
    Some((winner, confidence, agreeing))
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
    Some(Convention {
        id: format!("shape.public-arity.{}.{ext}", id_fragment(dir)),
        statement: format!(
            "Types here expose exactly {arity} public method{}",
            if arity == 1 { "" } else { "s" }
        ),
        scope: scope_for(dir, ext),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &arity),
        evidence: evidence(&observations, &arity),
        sample_roots: Vec::new(),
        enforcement: canon_core::enforcement_for("shape.public-arity", confidence, settings),
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
    Some(Convention {
        id: format!("shape.entrypoint.{}.{ext}", id_fragment(dir)),
        statement: format!("That public method is named `{name}`"),
        scope: scope_for(dir, ext),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &name),
        evidence: evidence(&observations, &name),
        sample_roots: Vec::new(),
        enforcement: canon_core::enforcement_for("shape.entrypoint", confidence, settings),
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
    Some(Convention {
        id: format!("shape.base.{}.{ext}", id_fragment(dir)),
        statement: format!("Types here inherit from `{base}`"),
        scope: scope_for(dir, ext),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &winner),
        evidence: evidence(&observations, &winner),
        sample_roots: Vec::new(),
        enforcement: canon_core::enforcement_for("shape.base", confidence, settings),
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
fn base_family(
    dir: &str,
    ext: &str,
    members: &[&FactSet],
    settings: &Settings,
) -> Option<Convention> {
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
pub(crate) fn family_of(base: &str) -> String {
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
fn macros(dir: &str, ext: &str, members: &[&FactSet], settings: &Settings) -> Option<Convention> {
    let observations: Vec<(Option<String>, f32, &FactSet)> = members
        .iter()
        .map(|s| {
            let mut names: Vec<&String> = s
                .facts
                .calls
                .iter()
                .filter(|c| c.receiver.is_none() && is_macro(&c.name))
                .map(|c| &c.name)
                .collect();
            names.sort_unstable();
            names.dedup();
            // The macro this file leans on most, once per file, counted the
            // same way the names above were selected: only among the
            // receiverless calls a receiver would otherwise have excluded.
            let dominant = names
                .into_iter()
                .max_by_key(|n| {
                    s.facts.calls.iter().filter(|c| c.receiver.is_none() && &c.name == *n).count()
                })
                .cloned();
            (dominant, s.weight, *s)
        })
        .collect();

    let (winner, confidence, agreeing) = majority(&observations, settings)?;
    let name = winner.clone()?;
    Some(Convention {
        id: format!("shape.macros.{}.{ext}", id_fragment(dir)),
        statement: format!("Files here use `{name}`"),
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

/// Whether a receiverless call is a framework macro rather than plumbing.
///
/// Import keywords are already an import fact: `require "json"` parses as a
/// call with an argument list in the same query pass that also records it as
/// an import, and counting it again would find every Ruby directory agreeing
/// that it uses `require`. Everything else is left in — a helper a directory
/// calls unanimously is as much a convention as a framework macro, and the
/// agreement bar decides which one survives.
fn is_macro(name: &str) -> bool {
    !matches!(
        name,
        "require" | "require_relative" | "load" | "import" | "include_once" | "require_once"
    )
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
fn annotation(
    dir: &str,
    ext: &str,
    members: &[&FactSet],
    settings: &Settings,
) -> Option<Convention> {
    let observations: Vec<(Option<String>, f32, &FactSet)> = members
        .iter()
        .map(|s| {
            let mut names: Vec<&String> = s.facts.annotations.iter().map(|a| &a.name).collect();
            names.sort_unstable();
            names.dedup();
            let dominant = names
                .into_iter()
                .max_by_key(|n| s.facts.annotations.iter().filter(|a| &&a.name == n).count())
                .cloned();
            (dominant, s.weight, *s)
        })
        .collect();

    let (winner, confidence, agreeing) = majority(&observations, settings)?;
    let name = winner.clone()?;
    Some(Convention {
        id: format!("shape.annotation.{}.{ext}", id_fragment(dir)),
        statement: format!("Files here carry `@{name}`"),
        scope: scope_for(dir, ext),
        confidence,
        agreeing,
        total: observations.len(),
        exemplar: exemplar(&observations, &winner),
        evidence: evidence(&observations, &winner),
        sample_roots: Vec::new(),
        // Advisory. A directory of decorated classes can still legitimately
        // gain the one plain helper class that carries nothing.
        enforcement: Enforcement::Advisory,
    })
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
            canon_core::enforcement_for("shape.family.app.controllers.rb", total, &settings),
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
    fn an_annotation_only_a_minority_carries_is_not_a_convention() {
        let mut files =
            fixture::agreeing("src/x", "ts", 6, "export class A$N {\n  go(): void {}\n}\n");
        files.push((
            "src/x/odd.ts".to_string(),
            "@Injectable()\nexport class Odd {\n  go(): void {}\n}\n".to_string(),
        ));
        let convs = derive_from("sem-annot-minority", &files);
        assert!(
            !convs.iter().any(|c| c.id.starts_with("shape.annotation")),
            "got {}",
            joined(&convs)
        );
    }
}
