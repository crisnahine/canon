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
    verify_with(rel, source, conventions, Strictness::Advisory)
}

/// How closely a check has to match the sample the rule was derived from.
///
/// The two callers want different things and used to get the same thing.
///
/// Advice is generous on purpose: a type with `up` and `down` is told the
/// entrypoint here is named `change`, even though only single-method files
/// were counted, because withholding that costs two round trips to fix one
/// file.
///
/// A refusal cannot be generous. A rule may only refuse when every file in
/// scope agrees, and "in scope" has to mean the files the rule was actually
/// counted over. Applied to the others it refused correct code: `RuboCop`'s
/// `lib/rubocop/server/core.rb`, which has two public methods where the rule
/// was derived from seven files that have one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Strictness {
    Advisory,
    OnlyWhatWasCounted,
}

fn verify_with(
    rel: &str,
    source: &str,
    conventions: &[Convention],
    strictness: Strictness,
) -> Vec<Violation> {
    let applicable: Vec<&Convention> =
        conventions.iter().filter(|c| c.scope.matches(rel)).collect();
    if applicable.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    if crate::tier0::counts_toward_naming(rel) {
        for convention in &applicable {
            if let Some(v) = check_naming(rel, convention) {
                out.push(v);
            }
        }
    }
    for convention in &applicable {
        if let Some(v) = check_test_suffix(rel, convention) {
            out.push(v);
        }
    }

    // Structure is enough for every check that reads declarations, and skipping
    // the query pass is most of what makes a write cheap. An import rule is the
    // exception: it was derived from the query's import list where the language
    // has one, so checking it against the structural list would compare two
    // different readings of the same file. Enforcement never needs this — no
    // import rule is ever Blocking — so the hot path keeps the cheap pass and
    // only `PostToolUse`, after the write has already landed, pays for it.
    let wants_imports =
        applicable.iter().any(|c| c.id.starts_with("shape.import") && has_import_statement(c));
    let facts = extension_of(rel).and_then(canon_extract::lang::from_extension).and_then(|l| {
        if wants_imports {
            canon_extract::extract(l, source, rel).ok()
        } else {
            canon_extract::extract_structure(l, source, rel).ok()
        }
    });
    let Some(facts) = facts else { return out };

    for convention in &applicable {
        if let Some(v) = check_import(&facts, convention) {
            out.push(v);
        }
    }

    // A test is a different kind of file from the code beside it, and a shape
    // rule derived from a directory is a rule about that code. The first test
    // written into such a directory has no counterexample in the sample yet,
    // so the rule is still at total agreement and refused it: a colocated
    // `test_void_invoice.py` was told it must inherit `BaseService` and expose
    // one public method. Advice on it is harmless and stays; a refusal is the
    // check being wrong about a legitimate file, which is the one thing
    // enforcement is not allowed to be.
    if strictness == Strictness::OnlyWhatWasCounted && crate::tier0::is_test_path(rel) {
        return out;
    }

    // Resolved once, the same way deriving resolves it, so the two halves
    // cannot disagree about which type a convention is about.
    let subject = crate::subject::primary_type(&facts, crate::subject::stem_of(rel));

    for convention in &applicable {
        out.extend(check_shape(&facts, subject, convention, strictness));
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
    // The same root the rule was derived from. Reading up to the last dot here
    // and up to the first dot there would report `Button.module.css` as
    // breaking a rule it was never counted against.
    let stem = naming::name_root(crate::subject::stem_of(rel));
    if stem.is_empty() || naming::is_compatible(stem, expected) {
        return None;
    }
    Some(Violation {
        convention_id: convention.id.clone(),
        message: format!(
            "file name `{stem}` is not {} ({}/{} files matching {} are)",
            expected.label(),
            convention.agreeing,
            convention.total,
            convention.scope.render()
        ),
    })
}

const IMPORT_PREFIX: &str = "Files here import from ";
const SUFFIX_PREFIX: &str = "Test files are named ";

fn has_import_statement(convention: &Convention) -> bool {
    backticked(&convention.statement, IMPORT_PREFIX).is_some()
}

/// "Files here import from `rails_helper`."
///
/// The highest-value family canon derives and, until now, the only one with no
/// check at all. A wrong import is the way generated code drifts hardest,
/// because it compiles and type-checks whenever a plausible alternative exists;
/// a spec that requires `spec_helper` in a directory where 1,027 of 1,027 files
/// require `rails_helper` was stated at and then never checked.
///
/// Matched against the file's imports as written, which is how the rule was
/// counted. A file that imports nothing at all is not reported: a module with
/// no dependencies is a different kind of file, not a broken one.
fn check_import(facts: &FileFacts, convention: &Convention) -> Option<Violation> {
    let expected = backticked(&convention.statement, IMPORT_PREFIX)?;
    if facts.imports.is_empty() || facts.imports.contains(&expected) {
        return None;
    }
    Some(Violation {
        convention_id: convention.id.clone(),
        message: format!(
            "this file does not import `{expected}`; files here do ({}/{} matching {}): it imports {}",
            convention.agreeing,
            convention.total,
            convention.scope.render(),
            facts.imports.join(", ")
        ),
    })
}

/// "Test files are named `*_spec.rb`."
///
/// Path-only, and only about a file that is a test. Deriving it and never
/// checking it meant a `thing_test.rb` written into a repository that names
/// every test `*_spec.rb` was told the rule in the same block and then not
/// told it had broken it.
fn check_test_suffix(rel: &str, convention: &Convention) -> Option<Violation> {
    let expected = backticked(&convention.statement, SUFFIX_PREFIX)?;
    if !crate::tier0::is_test_path(rel) {
        return None;
    }
    let name = rel.rsplit_once('/').map_or(rel, |(_, n)| n);
    if crate::tier0::matches_test_glob(name, &expected) {
        return None;
    }
    Some(Violation {
        convention_id: convention.id.clone(),
        message: format!(
            "test file `{name}` is not named `{expected}` ({}/{} matching {} are)",
            convention.agreeing,
            convention.total,
            convention.scope.render()
        ),
    })
}

fn check_shape(
    facts: &FileFacts,
    subject: Option<&canon_extract::TypeFacts>,
    convention: &Convention,
    strictness: Strictness,
) -> Vec<Violation> {
    // The scope travels with the counts. A bare "47/52" beside a sentence about
    // "this directory" invites the reader to check the directory and find a
    // different number, because the rule may have been counted repository-wide.
    let evidence = format!(
        "{}/{} matching {}",
        convention.agreeing,
        convention.total,
        convention.scope.render()
    );
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

    // Whatever the arity, when advising. Gating that on a single public method
    // withheld the rule from exactly the files that broke it hardest: a type
    // with `up` and `down` was told its count was wrong and never told the
    // expected name, which is two round trips to fix one file.
    //
    // A refusal is gated, because the rule was derived over the files with one
    // public method and says nothing about the rest.
    let entrypoint_applies = strictness == Strictness::Advisory || t.public_arity() == 1;
    if let Some(expected) = backticked(&convention.statement, "That public method is named ")
        && entrypoint_applies
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

/// "Every file here has a test of the same name."
///
/// Separate from [`verify_source`] because it is the one check that cannot be
/// answered from the file alone: it has to look for a sibling that does not
/// exist yet. The caller supplies the repository root, the same way
/// [`crate::duplicates_against_siblings`] already does.
///
/// Reported only when the rule is strong and the file is not itself a test.
/// Advisory always: a file may legitimately be the one thing in a directory
/// that needs no test, which is why the rule was never enforceable.
#[must_use]
pub fn missing_test(
    root: &std::path::Path,
    rel: &str,
    conventions: &[Convention],
) -> Option<Violation> {
    if crate::tier0::is_test_path(rel) {
        return None;
    }
    let convention = conventions
        .iter()
        .filter(|c| c.id.starts_with("tests.colocation") && c.scope.matches(rel))
        .max_by_key(|c| c.scope.specificity())?;
    if crate::tier0::has_test_for(root, rel) {
        return None;
    }
    Some(Violation {
        convention_id: convention.id.clone(),
        message: format!(
            "no test found for this file; {}/{} files matching {} have one",
            convention.agreeing,
            convention.total,
            convention.scope.render()
        ),
    })
}

/// The violations that justify refusing a write.
///
/// Only rules the repository agrees on totally and whose check cannot be wrong
/// about a legitimate file. Everything else is reported and not enforced.
///
/// Enforcement is recomputed from `settings` rather than read off the
/// snapshot. A refusal tells the author to turn it off in `.canon.toml`, and
/// reading the stored decision meant doing so had no effect until the next
/// session rebuilt the snapshot — the escape hatch was inert at exactly the
/// moment it was needed.
/// `source` is the file as it will exist after the write, when the caller can
/// know it. `None` means it cannot — a notebook cell, or an edit to a file that
/// is not on disk — and only the path-only rules are checked, because a naming
/// rule reads the path and never the content. Withholding those too would make
/// enforcement depend on which tool was reached for rather than on what lands.
#[must_use]
pub fn blocking_violations(
    rel: &str,
    source: Option<String>,
    conventions: &[Convention],
    settings: &canon_core::Settings,
) -> Vec<Violation> {
    if !settings.enforce {
        return Vec::new();
    }
    let enforceable: Vec<Convention> = conventions
        .iter()
        .filter(|c| c.enforcement_now(settings) == canon_core::Enforcement::Blocking)
        .cloned()
        .collect();
    if enforceable.is_empty() {
        return Vec::new();
    }
    let Some(source) = source else {
        return path_violations(rel, &enforceable);
    };
    verify_with(rel, &source, &enforceable, Strictness::OnlyWhatWasCounted)
}

/// The subset of checks that need no content at all.
fn path_violations(rel: &str, conventions: &[Convention]) -> Vec<Violation> {
    if !crate::tier0::counts_toward_naming(rel) {
        return Vec::new();
    }
    conventions
        .iter()
        .filter(|c| c.scope.matches(rel))
        .filter_map(|c| check_naming(rel, c))
        .collect()
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
        // The counts travel with the scope they were counted over, so a reader
        // checking the number knows which files to count.
        assert!(
            violations[0].message.contains("47/52 matching app/services/**/*.rb"),
            "got {}",
            violations[0].message
        );
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

    fn blocking(id: &str, statement: &str, scope: Scope) -> Convention {
        let mut c = conv(id, statement);
        c.scope = scope;
        c.confidence = Confidence::derive(7, 7).expect("valid");
        c.agreeing = 7;
        c.total = 7;
        c
    }

    #[test]
    fn a_file_the_rule_was_never_counted_over_is_not_refused_for_breaking_it() {
        // Every false positive fourteen real repositories produced, and all
        // one defect: deriving excludes a file from the sample, then checking
        // judges it against the resulting rule.
        let settings = canon_core::Settings::default();
        let rule = blocking(
            "naming.repo.rb",
            "Files here are named in snake_case",
            Scope::Ext("rb".into()),
        );
        for excluded in [
            "spec/views/auth/_status.html.haml_spec.rb",     // a test
            "app/javascript/utils/__tests__/base64-test.rb", // a test, by directory
        ] {
            assert!(
                blocking_violations(
                    excluded,
                    Some("class A; end\n".to_string()),
                    std::slice::from_ref(&rule),
                    &settings
                )
                .is_empty(),
                "{excluded} was refused by a rule derived without it"
            );
        }

        let py = blocking(
            "naming.repo.py",
            "Files here are named in snake_case",
            Scope::Ext("py".into()),
        );
        assert!(
            blocking_violations(
                "src/flask/json/__init__.py",
                Some("x = 1\n".to_string()),
                &[py],
                &settings
            )
            .is_empty(),
            "a dunder name is excluded when deriving and must be when checking"
        );

        let rst = blocking(
            "naming.repo.rst",
            "Files here are named in kebab-case",
            Scope::Ext("rst".into()),
        );
        assert!(
            blocking_violations("AUTHORS.rst", Some("x\n".to_string()), &[rst], &settings)
                .is_empty(),
            "a conventional name is excluded when deriving and must be when checking"
        );

        // A file the rule does speak for is still refused.
        assert_eq!(
            blocking_violations(
                "app/services/NotSnake.rb",
                Some("class A; end\n".to_string()),
                &[rule],
                &settings
            )
            .len(),
            1
        );
    }

    #[test]
    fn the_first_test_written_into_a_directory_is_not_refused_by_its_code_rules() {
        // The sample cannot contain it yet, so the rule is still at total
        // agreement and refused it. A colocated `test_void_invoice.py` was told
        // it must inherit `BaseService` and expose one public method.
        let settings = canon_core::Settings::default();
        let rules = vec![
            blocking(
                "shape.base.app.py",
                "Types here inherit from `BaseService`",
                Scope::DirExt("app".into(), "py".into()),
            ),
            blocking(
                "shape.public-arity.app.py",
                "Types here expose exactly 1 public method",
                Scope::DirExt("app".into(), "py".into()),
            ),
        ];
        let test_file = "class TestVoidInvoice:\n    def test_voids(self): pass\n    def test_rejects(self): pass\n";
        for rel in [
            "app/services/test_void_invoice.py",
            "app/services/__tests__/test_void_invoice.py",
            "app/services/void_invoice_test.py",
        ] {
            assert!(
                blocking_violations(rel, Some(test_file.to_string()), &rules, &settings).is_empty(),
                "{rel} was refused for not being shaped like the code it tests"
            );
        }

        // The code beside it is still held to the rules.
        assert!(
            !blocking_violations(
                "app/services/void_invoice.py",
                Some(test_file.to_string()),
                &rules,
                &settings
            )
            .is_empty()
        );
    }

    #[test]
    fn the_entrypoint_rule_advises_at_any_arity_and_refuses_at_the_one_it_counted() {
        // Derived over files with a single public method. Advising a type with
        // two is deliberate and saves a round trip; refusing it applies the
        // rule to files it was never counted over, and it refused RuboCop's
        // own `lib/rubocop/server/core.rb`.
        let settings = canon_core::Settings::default();
        let rule = blocking(
            "shape.entrypoint.lib.rb",
            "That public method is named `run`",
            Scope::DirExt("lib".into(), "rb".into()),
        );
        let two = "class Core\n  def token; end\n  def start; end\nend\n";

        let advice = verify_source("lib/core.rb", two, std::slice::from_ref(&rule));
        assert!(advice.iter().any(|v| v.message.contains("`run`")), "advice was withheld");

        assert!(
            blocking_violations(
                "lib/core.rb",
                Some(two.to_string()),
                std::slice::from_ref(&rule),
                &settings
            )
            .is_empty(),
            "a two-method type was refused by a rule counted over one-method files"
        );

        let one = "class Core\n  def token; end\nend\n";
        assert_eq!(
            blocking_violations("lib/core.rb", Some(one.to_string()), &[rule], &settings).len(),
            1
        );
    }

    #[test]
    fn enforcement_and_suppression_are_read_per_write_not_per_snapshot() {
        // The stored decision made `.canon.toml` inert until the next session,
        // which is the one moment nobody reaches for it.
        let rule = blocking(
            "naming.repo.rb",
            "Files here are named in snake_case",
            Scope::Ext("rb".into()),
        );
        let rel = "app/NotSnake.rb";
        let source = "class A; end\n";

        let on = canon_core::Settings::default();
        assert_eq!(
            blocking_violations(rel, Some(source.to_string()), std::slice::from_ref(&rule), &on)
                .len(),
            1
        );

        let off = canon_core::Settings { enforce: false, ..canon_core::Settings::default() };
        assert!(
            blocking_violations(rel, Some(source.to_string()), std::slice::from_ref(&rule), &off)
                .is_empty()
        );

        let suppressed = canon_core::Settings {
            suppress: vec!["naming.repo.rb".to_string()],
            ..canon_core::Settings::default()
        };
        assert!(
            blocking_violations(
                rel,
                Some(source.to_string()),
                std::slice::from_ref(&rule),
                &suppressed
            )
            .is_empty()
        );

        let mut rollup = rule;
        rollup.id = "naming.repo.rb.rollup".to_string();
        assert!(
            blocking_violations(rel, Some(source.to_string()), &[rollup], &on).is_empty(),
            "a rule assembled from other rules generalises to directories that never voted"
        );
    }

    #[test]
    fn statement_parsing_survives_text_it_does_not_recognise() {
        assert_eq!(trailing_count("expose exactly many methods", "expose exactly "), None);
        assert_eq!(backticked("named without backticks", "named "), None);
        assert_eq!(trailing_count("no prefix here", "expose exactly "), None);
    }
}
