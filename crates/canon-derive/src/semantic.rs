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

/// The type a file is *about*.
///
/// Ruby files routinely declare a small `class SomethingError < StandardError`
/// beside the real class, and TypeScript files declare prop interfaces beside
/// the component. Counting every declared type lets those auxiliaries outvote
/// the subject: pointed at a real Rails repository, treating them equally
/// produced "types here inherit from `StandardError`" across the whole
/// services tree, which is true of the error classes and false of every
/// service in it.
///
/// Resolution order: the type whose name matches the file name, then the one
/// with the largest surface, then the first declared.
fn primary_type<'f>(facts: &'f FileFacts, stem: &str) -> Option<&'f canon_extract::TypeFacts> {
    facts.types.iter().find(|t| to_snake(&t.name) == stem).or_else(|| {
        facts.types.iter().max_by_key(|t| t.public_methods.len() + t.private_methods.len())
    })
}

/// `CreateEnrolment` and `Enrolments::Create` both reduce toward a file stem.
fn to_snake(name: &str) -> String {
    let last = name.rsplit("::").next().unwrap_or(name);
    let mut out = String::with_capacity(last.len() + 4);
    for (i, c) in last.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

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
        primary_type(&self.facts, &self.stem)
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
        out.extend(base_class(&dir, &ext, &members, settings));
        out.extend(module_arity(&dir, &ext, &members, settings));
        out.extend(collaborator(&dir, &ext, &members, settings));
    }
    out
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
    let confidence = Confidence::derive_counted(weight, total, observations.len())?;
    if confidence.value() < settings.confidence_floor {
        return None;
    }
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
        enforcement: Enforcement::Advisory,
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
        enforcement: Enforcement::Advisory,
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
        enforcement: Enforcement::Advisory,
    })
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
        enforcement: Enforcement::Advisory,
    })
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
}
