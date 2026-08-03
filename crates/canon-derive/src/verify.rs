//! Checking what was actually written against what the repository does.
//!
//! Runs after the write, on the file as it now exists. That ordering matters:
//! injection changes what gets written most of the time, and this catches the
//! rest, at the moment the model can still fix it cheaply rather than at review
//! time when the context is gone.
//!
//! Every violation names the count behind it. "Repo agrees on 1, this has 3
//! (47/52)" is actionable; "violates convention" is an argument.

use canon_core::Convention;
use canon_extract::FileFacts;

use crate::naming;

/// One disagreement between a written file and a derived convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The convention that disagreed.
    pub convention_id: String,
    /// What the repository does, what this file does, and the evidence.
    pub message: String,
}

/// Compare `source` against every convention that applies to `rel`.
///
/// Returns an empty vector when the file agrees, when nothing applies, or when
/// the language has no extractor. All three mean the same thing to the caller:
/// nothing to say.
#[must_use]
pub fn verify_source(rel: &str, source: &str, conventions: &[Convention]) -> Vec<Violation> {
    let applicable: Vec<&Convention> =
        conventions.iter().filter(|c| c.scope.matches(rel)).collect();
    if applicable.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    for convention in &applicable {
        if let Some(v) = check_naming(rel, convention) {
            out.push(v);
        }
    }

    let facts = extension_of(rel)
        .and_then(canon_extract::lang::from_extension)
        .and_then(|l| canon_extract::extract(l, source, rel).ok());
    let Some(facts) = facts else { return out };

    // Resolved once, the same way deriving resolves it, so the two halves
    // cannot disagree about which type a convention is about.
    let subject = crate::subject::primary_type(&facts, crate::subject::stem_of(rel));

    for convention in &applicable {
        out.extend(check_shape(&facts, subject, convention));
    }
    out
}

fn extension_of(rel: &str) -> Option<&str> {
    rel.rsplit_once('/').map_or(rel, |(_, name)| name).rsplit_once('.').map(|(_, e)| e)
}

fn check_naming(rel: &str, convention: &Convention) -> Option<Violation> {
    let expected = convention.id.starts_with("naming.").then(|| {
        naming::Style::ALL.iter().copied().find(|s| convention.statement.contains(s.label()))
    })??;
    let stem = crate::subject::stem_of(rel);
    if naming::is_compatible(stem, expected) {
        return None;
    }
    Some(Violation {
        convention_id: convention.id.clone(),
        message: format!(
            "file name `{stem}` is not {} ({}/{} files here are)",
            expected.label(),
            convention.agreeing,
            convention.total
        ),
    })
}

fn check_shape(
    facts: &FileFacts,
    subject: Option<&canon_extract::TypeFacts>,
    convention: &Convention,
) -> Vec<Violation> {
    let evidence = format!("{}/{}", convention.agreeing, convention.total);
    let mut out = Vec::new();

    if let Some(expected) = trailing_count(&convention.statement, "export exactly ") {
        // Only meaningful for a module with no types, which is how the rule
        // was derived. A file that introduces a class is a different shape,
        // not a violation of this one.
        if facts.types.is_empty() && facts.free_functions.len() != expected {
            out.push(Violation {
                convention_id: convention.id.clone(),
                message: format!(
                    "this file exports {} function(s); files here export {expected} ({evidence}): {}",
                    facts.free_functions.len(),
                    facts.free_functions.join(", ")
                ),
            });
        }
    }

    // The subject, not every declared type. A namespace module and a small
    // error class beside the real one are not what the convention was derived
    // from, and judging them reports correct files as broken.
    let Some(t) = subject else { return out };

    if let Some(expected) = trailing_count(&convention.statement, "expose exactly ")
        && t.public_arity() != expected
    {
        out.push(Violation {
            convention_id: convention.id.clone(),
            message: format!(
                "`{}` exposes {} public method(s); types here expose {expected} ({evidence}): {}",
                t.name,
                t.public_arity(),
                t.public_methods.join(", ")
            ),
        });
    }

    // Whatever the arity. Gating this on a single public method withheld the
    // rule from exactly the files that broke it hardest: a type with `up` and
    // `down` was told its count was wrong and never told the expected name,
    // which is two round trips to fix one file.
    if let Some(expected) = backticked(&convention.statement, "That public method is named ")
        && !t.public_methods.is_empty()
        && !t.public_methods.contains(&expected)
    {
        let message = if t.public_arity() == 1 {
            format!(
                "`{}` exposes `{}`; the entrypoint here is named `{expected}` ({evidence})",
                t.name,
                t.public_methods.first().map_or("", String::as_str)
            )
        } else {
            format!(
                "`{}` exposes {} but not `{expected}`; the entrypoint here is named `{expected}` ({evidence})",
                t.name,
                t.public_methods.join(", ")
            )
        };
        out.push(Violation { convention_id: convention.id.clone(), message });
    }

    if let Some(expected) = backticked(&convention.statement, "Types here inherit from ") {
        {
            match &t.superclass {
                Some(actual) if actual == &expected => {}
                Some(actual) => out.push(Violation {
                    convention_id: convention.id.clone(),
                    message: format!(
                        "`{}` inherits from `{actual}`; types here inherit from `{expected}` ({evidence})",
                        t.name
                    ),
                }),
                None => out.push(Violation {
                    convention_id: convention.id.clone(),
                    message: format!(
                        "`{}` has no base type; types here inherit from `{expected}` ({evidence})",
                        t.name
                    ),
                }),
            }
        }
    }

    out
}

/// The integer immediately after `prefix`, e.g. `1` in "expose exactly 1 ...".
fn trailing_count(statement: &str, prefix: &str) -> Option<usize> {
    let rest = statement.split_once(prefix)?.1;
    rest.split_whitespace().next()?.parse().ok()
}

/// The backticked identifier immediately after `prefix`.
fn backticked(statement: &str, prefix: &str) -> Option<String> {
    let rest = statement.split_once(prefix)?.1;
    let inner = rest.strip_prefix('`')?;
    inner.split_once('`').map(|(name, _)| name.to_string())
}

/// The violations that justify refusing a write.
///
/// Only rules the repository agrees on totally and whose check cannot be wrong
/// about a legitimate file. Everything else is reported and not enforced.
#[must_use]
pub fn blocking_violations(rel: &str, source: &str, conventions: &[Convention]) -> Vec<Violation> {
    let enforceable: Vec<Convention> = conventions
        .iter()
        .filter(|c| c.enforcement == canon_core::Enforcement::Blocking)
        .cloned()
        .collect();
    if enforceable.is_empty() {
        return Vec::new();
    }
    verify_source(rel, source, &enforceable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use canon_core::{Confidence, Enforcement, Scope};

    fn conv(id: &str, statement: &str) -> Convention {
        Convention {
            id: id.into(),
            statement: statement.into(),
            scope: Scope::DirExt("app/services".into(), "rb".into()),
            confidence: Confidence::derive(47, 52).expect("valid"),
            agreeing: 47,
            total: 52,
            exemplar: None,
            evidence: vec![],
            enforcement: Enforcement::Advisory,
        }
    }

    #[test]
    fn a_conforming_file_produces_no_violations() {
        let convs = vec![
            conv("shape.public-arity.app.services.rb", "Types here expose exactly 1 public method"),
            conv("shape.entrypoint.app.services.rb", "That public method is named `call`"),
            conv("shape.base.app.services.rb", "Types here inherit from `ApplicationService`"),
        ];
        let source =
            "class Create < ApplicationService\n  def call; end\n  private\n  def h; end\nend\n";
        assert!(verify_source("app/services/create.rb", source, &convs).is_empty());
    }

    #[test]
    fn too_many_public_methods_is_reported_with_the_evidence() {
        let convs = vec![conv(
            "shape.public-arity.app.services.rb",
            "Types here expose exactly 1 public method",
        )];
        let source = "class Create\n  def call; end\n  def extra; end\nend\n";
        let violations = verify_source("app/services/create.rb", source, &convs);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("exposes 2 public method"));
        assert!(violations[0].message.contains("(47/52)"), "got {}", violations[0].message);
    }

    #[test]
    fn a_wrong_entrypoint_name_is_reported() {
        let convs =
            vec![conv("shape.entrypoint.app.services.rb", "That public method is named `call`")];
        let source = "class Create\n  def perform; end\nend\n";
        let violations = verify_source("app/services/create.rb", source, &convs);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("`perform`"));
    }

    #[test]
    fn a_missing_base_class_is_reported_differently_from_a_wrong_one() {
        let convs = vec![conv(
            "shape.base.app.services.rb",
            "Types here inherit from `ApplicationService`",
        )];
        let missing = verify_source("app/services/a.rb", "class A\nend\n", &convs);
        assert!(missing[0].message.contains("has no base type"));

        let wrong = verify_source("app/services/a.rb", "class A < Other\nend\n", &convs);
        assert!(wrong[0].message.contains("inherits from `Other`"));
    }

    #[test]
    fn a_namespace_module_is_not_reported_as_a_violation() {
        // Issue #1. This file agrees with every convention in its scope. The
        // old check judged `Billing` as well and reported two violations, and
        // once those rules reached total agreement it refused the write.
        let convs = vec![
            conv("shape.public-arity.app.services.rb", "Types here expose exactly 1 public method"),
            conv("shape.base.app.services.rb", "Types here inherit from `ApplicationService`"),
        ];
        let source = "module Billing\n  class ApplyVariance < ApplicationService\n    def call; end\n  end\nend\n";
        let violations = verify_source("app/services/apply_variance.rb", source, &convs);
        assert!(violations.is_empty(), "correct file reported as broken: {violations:#?}");
    }

    #[test]
    fn an_error_class_beside_the_subject_is_not_judged() {
        let convs = vec![conv(
            "shape.public-arity.app.services.rb",
            "Types here expose exactly 1 public method",
        )];
        let source = "class ChargeCard < Base\n  def call; end\n\n  class DeclinedError < StandardError; end\nend\n";
        let violations = verify_source("app/services/charge_card.rb", source, &convs);
        assert!(violations.is_empty(), "got {violations:#?}");
    }

    #[test]
    fn the_entrypoint_name_is_reported_even_when_the_arity_is_wrong() {
        // Issue #2. The old gate withheld the entrypoint rule from exactly the
        // files that broke it hardest, costing two round trips to fix one file.
        let convs = vec![
            conv("shape.public-arity.db.rb", "Types here expose exactly 1 public method"),
            conv("shape.entrypoint.db.rb", "That public method is named `change`"),
        ];
        let source = "class AddThing\n  def up; end\n  def down; end\nend\n";
        let violations = verify_source("app/services/add_thing.rb", source, &convs);

        let text = violations.iter().map(|v| v.message.as_str()).collect::<Vec<_>>().join(" | ");
        assert!(text.contains("exposes 2 public method"), "got {text}");
        assert!(text.contains("`change`"), "the entrypoint rule was withheld: {text}");
    }

    #[test]
    fn a_type_that_has_the_expected_entrypoint_among_others_is_not_reported() {
        // Reporting a missing name is the point; reporting a present one is
        // noise, and would fire on every type with a second public method.
        let convs =
            vec![conv("shape.entrypoint.app.services.rb", "That public method is named `call`")];
        let source = "class A\n  def call; end\n  def extra; end\nend\n";
        let violations = verify_source("app/services/a.rb", source, &convs);
        assert!(violations.is_empty(), "got {violations:#?}");
    }

    #[test]
    fn a_file_outside_every_scope_is_not_checked() {
        let convs = vec![conv(
            "shape.public-arity.app.services.rb",
            "Types here expose exactly 1 public method",
        )];
        let source = "class Anything\n  def a; end\n  def b; end\nend\n";
        assert!(verify_source("lib/other.rb", source, &convs).is_empty());
    }

    #[test]
    fn a_naming_violation_is_caught_without_a_parser() {
        let mut c = conv("naming.src.tsx", "Files here are named in PascalCase");
        c.scope = Scope::DirExt("src".into(), "tsx".into());
        let violations = verify_source("src/user_card.tsx", "export const A = () => 1;", &[c]);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("is not PascalCase"));
    }

    #[test]
    fn an_unparseable_file_still_reports_its_naming_violations() {
        let mut c = conv("naming.src.tsx", "Files here are named in PascalCase");
        c.scope = Scope::DirExt("src".into(), "tsx".into());
        let violations = verify_source("src/user_card.tsx", "\u{0}\u{1} not code at all", &[c]);
        assert_eq!(violations.len(), 1);
    }

    #[test]
    fn a_language_with_no_extractor_yields_no_shape_violations() {
        let mut c = conv("shape.public-arity.app.vue", "Types here expose exactly 1 public method");
        c.scope = Scope::DirExt("app".into(), "vue".into());
        assert!(verify_source("app/a.vue", "<template></template>", &[c]).is_empty());
    }

    #[test]
    fn a_module_arity_rule_ignores_files_that_declare_a_class() {
        let mut c = conv("shape.module-arity.src.ts", "Files here export exactly 1 function");
        c.scope = Scope::DirExt("src".into(), "ts".into());
        let with_class = verify_source("src/a.ts", "export class A { call() {} }", &[c.clone()]);
        assert!(with_class.is_empty(), "a class file is a different shape, not a violation");

        let two =
            verify_source("src/a.ts", "export const a = () => 1;\nexport const b = () => 2;", &[c]);
        assert_eq!(two.len(), 1);
        assert!(two[0].message.contains("exports 2 function"));
    }

    #[test]
    fn statement_parsing_survives_text_it_does_not_recognise() {
        assert_eq!(trailing_count("expose exactly many methods", "expose exactly "), None);
        assert_eq!(backticked("named without backticks", "named "), None);
        assert_eq!(trailing_count("no prefix here", "expose exactly "), None);
    }
}
