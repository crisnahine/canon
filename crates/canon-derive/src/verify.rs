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

/// What a violation claims, independent of which rule noticed it.
///
/// The kind of rule and what the claim is about — deliberately not what the
/// repository wants instead. Two rules of one family derived at two nesting
/// levels can disagree, and keying on the expected value let both through, so
/// one wrong base class produced two lines telling the author two different
/// things. The defect is the identity; the answer belongs to whichever scope
/// describes this file most closely.
type Claim = (&'static str, String);

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

    let mut out: Vec<(Claim, &Convention, Violation)> = Vec::new();
    if crate::tier0::counts_toward_naming(rel) {
        for convention in &applicable {
            if strictness == Strictness::OnlyWhatWasCounted && !naming_speaks_for(rel, convention) {
                continue;
            }
            if let Some((claim, v)) = check_naming(rel, convention) {
                out.push((claim, convention, v));
            }
        }
    }
    for convention in &applicable {
        if let Some((claim, v)) = check_test_suffix(rel, convention) {
            out.push((claim, convention, v));
        }
        if let Some((claim, v)) = check_qualifier(rel, convention) {
            out.push((claim, convention, v));
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
    let Some(facts) = facts else { return one_line_per_claim(out) };

    for convention in &applicable {
        if let Some((claim, v)) = check_import(&facts, convention) {
            out.push((claim, convention, v));
        }
        if let Some((claim, v)) = check_export_style(&facts, convention) {
            out.push((claim, convention, v));
        }
        if let Some((claim, v)) = check_namespace(rel, &facts, convention) {
            out.push((claim, convention, v));
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
        return one_line_per_claim(out);
    }

    // Resolved once, the same way deriving resolves it, so the two halves
    // cannot disagree about which type a convention is about.
    let subject = crate::subject::primary_type(&facts, crate::subject::stem_of(rel));

    for convention in &applicable {
        out.extend(
            check_shape(&facts, subject, convention, strictness)
                .into_iter()
                .map(|(claim, v)| (claim, *convention, v)),
        );
    }
    one_line_per_claim(out)
}

/// One line per claim, from the narrowest rule that makes it.
///
/// Rules are derived at every ancestor directory, so a file three levels deep
/// breaks the same rule three times over and got three sentences differing only
/// in their denominators. A reader takes those for three separate rules and
/// fixes the same thing three times, and the copies spend a budget a distinct
/// fact needed: at six lines, a file breaking three rules in a three-level
/// directory emitted eight violations carrying four facts, and the fourth —
/// this file has no test — was the one that fell off the end.
///
/// Raising the cap is not the fix. The repetition is the defect, and a longer
/// block keeps every part of it.
///
/// The narrowest scope wins because its counts describe the files actually
/// beside the one being written. First occurrence keeps its position, so the
/// order the checks run in is still the order the reader sees.
fn one_line_per_claim(found: Vec<(Claim, &Convention, Violation)>) -> Vec<Violation> {
    let mut kept: Vec<(Claim, usize, Violation)> = Vec::new();
    for (claim, convention, violation) in found {
        let specificity = convention.scope.specificity();
        match kept.iter().position(|(seen, _, _)| *seen == claim) {
            None => kept.push((claim, specificity, violation)),
            Some(at) => {
                if let Some(slot) = kept.get_mut(at)
                    && specificity > slot.1
                {
                    *slot = (claim, specificity, violation);
                }
            }
        }
    }
    kept.into_iter().map(|(_, _, violation)| violation).collect()
}

/// Whether the file declared nothing the derivation would have counted.
///
/// `FileFacts::is_empty` is the wrong question here, because it consults
/// `calls`, and only the full extraction records those: `verify_with` takes the
/// cheap structural pass unless an import rule happens to be in scope, so a
/// predicate reading `calls` answers differently depending on an unrelated
/// rule. These three fields are populated identically either way.
fn declares_nothing(facts: &FileFacts) -> bool {
    facts.namespace.is_none() && facts.types.is_empty() && facts.free_functions.is_empty()
}

fn extension_of(rel: &str) -> Option<&str> {
    rel.rsplit_once('/').map_or(rel, |(_, name)| name).rsplit_once('.').map(|(_, e)| e)
}

fn check_naming(rel: &str, convention: &Convention) -> Option<(Claim, Violation)> {
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
    // A name that distinguishes no style cannot break one. `404.tsx`, `2fa.ts`
    // and `請求書.ts` have no case to read and no separator to read it at, so
    // they are compatible with the three lowercase styles and with neither
    // cased one — which made a `camelCase` directory refuse all three. The
    // derivation already refuses to draw a conclusion from names like these;
    // the check has to refuse to draw one too.
    if !naming::is_discriminating(stem) {
        return None;
    }
    // And an acronym is written the same way whatever the surrounding style.
    // `docs/FAQ.md`, `docs/API.md`, `src/lib/API.ts` — a separator-free
    // all-uppercase name is how every project spells one, so it is a name the
    // style system does not reach rather than a name that broke it. Relaxing
    // `PascalCase` alone left it refused in every directory that is not
    // `PascalCase`.
    if naming::is_bare_acronym(stem) {
        return None;
    }
    Some((
        ("naming", stem.to_string()),
        Violation {
            convention_id: convention.id.clone(),
            message: format!(
                "file name `{stem}` is not {} ({}/{} files matching {} are)",
                expected.label(),
                convention.agreeing,
                convention.total,
                convention.scope.render()
            ),
        },
    ))
}

/// Whether a naming rule was counted over anything like this file.
///
/// A scope is a coarse claim. `Scope::Ext("md")` says "every `.md` in the
/// repository", but the sample behind it may have been six files in `docs/`,
/// and it then refused `.github/PULL_REQUEST_TEMPLATE.md` — a file the
/// repository's tooling requires and the rule never saw. The same coarseness
/// hides a qualifier: a directory of `Button.module.css` derives `PascalCase`
/// for `**/*.css` and refuses a plain `globals.css`, which is a different kind
/// of file that happens to share an extension.
///
/// The evidence is a capped sample rather than the whole set, so this can only
/// ever withhold a refusal it should have made, never make one it should not.
/// That is the correct direction to be wrong in, and it is checked only on the
/// refusal path — advice still says everything it knows.
fn naming_speaks_for(rel: &str, convention: &Convention) -> bool {
    if !convention.id.starts_with("naming.") || convention.evidence.is_empty() {
        return true;
    }
    // The qualifiers between the name and the extension. `Button.module.css`
    // is `module`; `globals.css` is nothing; `charge_card.html.erb` is `html`.
    let qualifier = |path: &str| -> String {
        let name = path.rsplit_once('/').map_or(path, |(_, n)| n);
        let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
        stem.split_once('.').map_or(String::new(), |(_, rest)| rest.to_string())
    };
    let mine = qualifier(rel);
    if !convention.evidence.iter().any(|e| qualifier(&e.rel) == mine) {
        return false;
    }

    sample_covers(rel, convention)
}

/// Whether the rule's sample came from anywhere near this file.
///
/// A rule may speak about a file only where it was counted. An empty list means
/// the sample was too wide to record, which is what a genuinely repository-wide
/// rule looks like; anything else has to cover the file's own directory or an
/// ancestor of it.
///
/// Applied to every scope, not only `Scope::Ext`. Gating it on `Ext` left a
/// `src/**/*.tsx` rule counted entirely in `src/components/` refusing
/// `src/pages/` and `src/hooks/`, because the scope alone was taken as proof
/// the sample covered them.
///
/// Shared by the two path-only families rather than living inside the naming
/// one. `qualifier_conventions` groups on the same ancestor keys, root
/// included, so a `format.` rule can carry `Scope::Ext` and would otherwise
/// speak about every file of that extension wherever it sits.
fn sample_covers(rel: &str, convention: &Convention) -> bool {
    if convention.sample_roots.is_empty() {
        return true;
    }
    let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
    let below_a_sample =
        |counted: &String| counted == dir || is_below(dir, counted) || is_below(counted, dir);
    match &convention.scope {
        // A scope that names a directory carries its own boundary, so a sample
        // taken anywhere inside it speaks for the whole of it: its root, and
        // every directory between that root and a sampled one. Asking only
        // whether the file sits *under* a counted directory left all of those
        // answering neither way, and a rule that refused a name in every
        // subdirectory it sampled allowed the same name one level up.
        // `Scope::matches` has already put the file inside the prefix;
        // repeating it here is what keeps the wider arm from leaking — a
        // sibling subtree that contributed nothing is still refused nothing.
        canon_core::Scope::Dir(prefix) | canon_core::Scope::DirExt(prefix, _) => {
            convention.sample_roots.iter().any(below_a_sample)
                && (dir == prefix || is_below(dir, prefix))
        }
        // A scope with no directory has the repository root as the ancestor of
        // every sample it took, so the ancestor reading would admit every file
        // in the repository. A `**/*.md` rule counted in `docs/` has to go on
        // saying nothing about a file at the root, which is the case this guard
        // was written for.
        //
        // The empty string is a sample root like any other, recording a file
        // counted at the repository root — and every directory in the tree sits
        // below that. Passed to `is_below` it therefore readmitted everything,
        // reinstating the same reading from the other end: one root-level file
        // in the sample let the rule refuse directories it had never counted.
        // Here it speaks for the root and nothing else.
        canon_core::Scope::Repo | canon_core::Scope::Ext(_) => convention
            .sample_roots
            .iter()
            .any(|counted| counted == dir || (!counted.is_empty() && is_below(dir, counted))),
    }
}

/// Whether `dir` sits inside `ancestor`, which may be the repository root.
fn is_below(dir: &str, ancestor: &str) -> bool {
    if ancestor.is_empty() {
        return !dir.is_empty();
    }
    dir.strip_prefix(ancestor).is_some_and(|rest| rest.starts_with('/'))
}

const IMPORT_PREFIX: &str = "Files here import from ";
const SUFFIX_PREFIX: &str = "Test files are named ";
const FORMAT_PREFIX: &str = "Files here are named ";

/// "Views here are named `*.html.erb`."
///
/// Path-only, like the rule it checks. `show.erb` is not a template Rails will
/// render for an HTML request, and a view tree had no other rule to tell it so.
///
/// Keyed on the id rather than on the statement alone, because the naming
/// family opens with the same words and means something else by them.
fn check_qualifier(rel: &str, convention: &Convention) -> Option<(Claim, Violation)> {
    if !convention.id.starts_with("format.") {
        return None;
    }
    // The same gate the derivation applies, or the rule judges files it was
    // never counted over: a `_row.html.erb` partial is named by Rails, a
    // `.keep.erb` has no name root, and neither was in the sample. Deriving
    // and checking disagreeing here is the defect behind every false positive
    // enforcement produced against fourteen real repositories.
    if !crate::tier0::counts_toward_naming(rel) {
        return None;
    }
    // And the sample gate its neighbour gets. `qualifier_conventions` groups on
    // the same ancestor keys naming does, including the repository root, so a
    // `format.` rule can carry `Scope::Ext` and then speak about every file of
    // that extension wherever it lives. Advisory, so the cost is a spurious
    // line rather than a refusal — and a spurious line is still the rule
    // judging a file it was never counted over.
    if !sample_covers(rel, convention) {
        return None;
    }
    let expected = backticked(&convention.statement, FORMAT_PREFIX)?;
    let expected = expected.strip_prefix("*.")?;
    let name = rel.rsplit_once('/').map_or(rel, |(_, n)| n);
    let actual = name.split_once('.').map_or("", |(_, rest)| rest);
    if actual == expected {
        return None;
    }
    Some((
        ("format", name.to_string()),
        Violation {
            convention_id: convention.id.clone(),
            message: format!(
                "`{name}` is not named `*.{expected}` ({}/{} files matching {} are)",
                convention.agreeing,
                convention.total,
                convention.scope.render()
            ),
        },
    ))
}

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
fn check_import(facts: &FileFacts, convention: &Convention) -> Option<(Claim, Violation)> {
    let expected = backticked(&convention.statement, IMPORT_PREFIX)?;
    if facts.imports.is_empty() || facts.imports.contains(&expected) {
        return None;
    }
    Some((
        // Keyed on the module, unlike every other family. The rest answer one
        // question about the file and the narrowest scope owns the answer; an
        // import rule states that a module is needed, and two scopes can each
        // need a different one. Collapsing them dropped a second missing
        // import the author had to act on separately.
        ("shape.import", expected.clone()),
        Violation {
            convention_id: convention.id.clone(),
            message: format!(
                "this file does not import `{expected}`; files here do ({}/{} matching {}): it imports {}",
                convention.agreeing,
                convention.total,
                convention.scope.render(),
                facts.imports.join(", ")
            ),
        },
    ))
}

const DEFAULT_EXPORT: &str = "Files here export a default";
const NAMED_EXPORTS: &str = "Files here use named exports";
const NAMESPACE_PREFIX: &str = "Files here declare namespace ";

/// "Files here export a default."
///
/// Checked against what the module actually exports, which is how the rule was
/// counted. A module that exports nothing is not reported: it has made no
/// choice, and a directory of type declarations beside a component tree is a
/// different kind of file rather than a broken one.
fn check_export_style(facts: &FileFacts, convention: &Convention) -> Option<(Claim, Violation)> {
    let wants_default = match convention.statement.as_str() {
        DEFAULT_EXPORT => true,
        NAMED_EXPORTS => false,
        _ => return None,
    };
    if !facts.default_export && facts.free_functions.is_empty() {
        return None;
    }
    if facts.default_export == wants_default {
        return None;
    }
    let evidence = format!(
        "{}/{} matching {}",
        convention.agreeing,
        convention.total,
        convention.scope.render()
    );
    let message = if wants_default {
        format!(
            "this file exports {} by name; files here export a default ({evidence})",
            facts.free_functions.join(", ")
        )
    } else {
        format!("this file exports a default; files here use named exports ({evidence})")
    };
    Some((
        ("shape.export", String::new()),
        Violation { convention_id: convention.id.clone(), message },
    ))
}

/// "Files here declare namespace `App\Services\Billing`."
///
/// PSR-4 agreement between namespace and directory, checked the way the rule
/// was counted. Declaring none is reported separately from declaring another,
/// because they are different mistakes with different fixes.
fn check_namespace(
    rel: &str,
    facts: &FileFacts,
    convention: &Convention,
) -> Option<(Claim, Violation)> {
    let expected = backticked(&convention.statement, NAMESPACE_PREFIX)?;
    // Only the directory the rule names; see `speaks_for_this_directory`.
    if !crate::speaks_for_this_directory(convention, rel) {
        return None;
    }
    // Deriving drops a file whose facts are empty before it can vote, so
    // checking must not judge it. A blank or markup-only file is a different
    // kind of file, not one that forgot its namespace.
    if declares_nothing(facts) {
        return None;
    }
    if facts.namespace.as_ref() == Some(&expected) {
        return None;
    }
    let evidence = format!(
        "{}/{} matching {}",
        convention.agreeing,
        convention.total,
        convention.scope.render()
    );
    let message = match &facts.namespace {
        Some(actual) => format!(
            "this file declares namespace `{actual}`; files here declare `{expected}` ({evidence})"
        ),
        None => {
            format!("this file declares no namespace; files here declare `{expected}` ({evidence})")
        }
    };
    Some((
        ("shape.namespace", String::new()),
        Violation { convention_id: convention.id.clone(), message },
    ))
}

/// "Test files are named `*_spec.rb`."
///
/// Path-only, and only about a file that is a test. Deriving it and never
/// checking it meant a `thing_test.rb` written into a repository that names
/// every test `*_spec.rb` was told the rule in the same block and then not
/// told it had broken it.
fn check_test_suffix(rel: &str, convention: &Convention) -> Option<(Claim, Violation)> {
    let expected = backticked(&convention.statement, SUFFIX_PREFIX)?;
    if !crate::tier0::is_test_path(rel) {
        return None;
    }
    let name = rel.rsplit_once('/').map_or(rel, |(_, n)| n);
    if crate::tier0::matches_test_glob(name, &expected) {
        return None;
    }
    Some((
        ("tests.suffix", name.to_string()),
        Violation {
            convention_id: convention.id.clone(),
            message: format!(
                "test file `{name}` is not named `{expected}` ({}/{} matching {} are)",
                convention.agreeing,
                convention.total,
                convention.scope.render()
            ),
        },
    ))
}

fn check_shape(
    facts: &FileFacts,
    subject: Option<&canon_extract::TypeFacts>,
    convention: &Convention,
    strictness: Strictness,
) -> Vec<(Claim, Violation)> {
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
        // A file that declares nothing at all never voted — `gather` drops it
        // before derivation — so it cannot have broken the rule it produced.
        if facts.types.is_empty()
            && !declares_nothing(facts)
            && facts.free_functions.len() != expected
        {
            out.push((
                ("shape.module-arity", String::new()),
                Violation {
                    convention_id: convention.id.clone(),
                    message: format!(
                        "this file exports {} function(s); files here export {expected} ({evidence}): {}",
                        facts.free_functions.len(),
                        facts.free_functions.join(", ")
                    ),
                },
            ));
        }
    }

    // The subject, not every declared type. A namespace module and a small
    // error class beside the real one are not what the convention was derived
    // from, and judging them reports correct files as broken.
    let Some(t) = subject else { return out };

    // Advising on any arity is useful; refusing on a larger one is not. The
    // rule is derived over types with exactly this many methods, and a type
    // that carries one more is routinely legitimate: a Rails migration needs
    // `up` and `down` for an irreversible change, a Go type implements
    // `fmt.Stringer`, a Ruby object defines `to_s`, a TypeScript class exposes
    // a getter. Each of those was a hard refusal, and the advice attached to it
    // told the author to delete a method the language or framework requires.
    let arity_may_refuse =
        strictness == Strictness::Advisory || t.public_arity() < expected_arity(convention);
    if let Some(expected) = trailing_count(&convention.statement, "expose exactly ")
        && t.public_arity() != expected
        && arity_may_refuse
    {
        out.push((
            ("shape.public-arity", t.name.clone()),
            Violation {
                convention_id: convention.id.clone(),
                message: format!(
                    "`{}` exposes {} public method(s); types here expose {expected} ({evidence}): {}",
                    t.name,
                    t.public_arity(),
                    t.public_methods.join(", ")
                ),
            },
        ));
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
        out.push((
            ("shape.entrypoint", t.name.clone()),
            Violation { convention_id: convention.id.clone(), message },
        ));
    }

    if let Some(expected) = backticked(&convention.statement, "Types here inherit from ") {
        {
            // A type may declare several contracts, or embed several types, and
            // which one landed in `superclass` is decided by source order for
            // an interface and by an unordered set for an embed. Accepting a
            // match from either `interfaces` or `mixins` stops a refusal from
            // depending on where the author put a block or which embedded
            // field the extractor chose to call the base.
            match &t.superclass {
                _ if t.interfaces.contains(&expected) || t.mixins.contains(&expected) => {}
                Some(actual) if actual == &expected => {}
                Some(actual) => out.push((
                    ("shape.base", t.name.clone()),
                    Violation {
                        convention_id: convention.id.clone(),
                        message: format!(
                            "`{}` inherits from `{actual}`; types here inherit from `{expected}` ({evidence})",
                            t.name
                        ),
                    },
                )),
                None => out.push((
                    ("shape.base", t.name.clone()),
                    Violation {
                        convention_id: convention.id.clone(),
                        message: format!(
                            "`{}` has no base type; types here inherit from `{expected}` ({evidence})",
                            t.name
                        ),
                    },
                )),
            }
        }
    }

    out
}

/// The arity a statement asks for, or zero when it asks for none.
fn expected_arity(convention: &Convention) -> usize {
    trailing_count(&convention.statement, "expose exactly ").unwrap_or(0)
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
    settings: &canon_core::Settings,
) -> Option<Violation> {
    if crate::tier0::is_test_path(rel) {
        return None;
    }
    let convention = conventions
        .iter()
        .filter(|c| c.id.starts_with("tests.colocation") && c.scope.matches(rel))
        .max_by_key(|c| c.scope.specificity())?;
    if crate::tier0::has_test_for(root, rel, settings) {
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
    let found: Vec<(Claim, &Convention, Violation)> = conventions
        .iter()
        .filter(|c| c.scope.matches(rel))
        // The same gate `verify_with` applies. Without it a `NotebookEdit`, or
        // an `Edit` whose `old_string` no longer matches the file, refused
        // writes an identical `Write` allowed — the "depends which tool the
        // model reached for" defect, one branch over from where it was fixed.
        .filter(|c| naming_speaks_for(rel, c))
        .filter_map(|c| check_naming(rel, c).map(|(claim, v)| (claim, c, v)))
        .collect();
    // And the same deduplication, or a name that breaks one rule derived at
    // three levels is reported as three separate problems here too.
    one_line_per_claim(found)
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
            sample_roots: vec![],
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
    fn one_defect_is_stated_once_however_many_scopes_derived_the_rule() {
        // Issue #17. Rules are derived at every ancestor directory, so a file
        // three levels down broke `shape.base` three times and `shape.public
        // -arity` twice and got six lines carrying three facts. The reader
        // takes the copies for separate rules, and they cost the budget a
        // distinct fact: at six lines, the fourth fact — this file has no test
        // — was the one that fell off the end.
        let mut convs = Vec::new();
        let mut at = |statement: &str, dir: &str, agreeing: usize| {
            let mut c = conv(&format!("shape.{}.{dir}.rb", convs.len()), statement);
            c.scope = Scope::DirExt(dir.into(), "rb".into());
            // Distinct counts, so the repeated lines are not string-equal and
            // deduplicating on the rendered sentence would not find them.
            c.agreeing = agreeing;
            c.total = agreeing + 2;
            convs.push(c);
        };
        for (dir, agreeing) in [
            ("app/services", 377),
            ("app/services/billing", 303),
            ("app/services/billing/invoices", 118),
        ] {
            at("Types here inherit from `ApplicationService`", dir, agreeing);
        }
        for (dir, agreeing) in [("app/services", 346), ("app/services/billing", 115)] {
            at("Types here expose exactly 1 public method", dir, agreeing);
        }
        at("That public method is named `run`", "app/services", 1243);

        let source =
            "class Widget < WrongBase\n  def one; end\n  def two; end\n  def three; end\nend\n";
        let found = verify_source("app/services/billing/invoices/widget.rb", source, &convs);

        assert_eq!(found.len(), 3, "one line per fact, got {found:#?}");
        let line = |needle: &str| {
            found
                .iter()
                .find(|v| v.message.contains(needle))
                .unwrap_or_else(|| panic!("no line about {needle} in {found:#?}"))
                .message
                .clone()
        };
        // The narrowest scope that derived the rule is the one kept: its counts
        // describe the files actually beside the one being written.
        assert!(
            line("inherits from")
                .contains("118/120 matching app/services/billing/invoices/**/*.rb"),
            "got {}",
            line("inherits from")
        );
        assert!(
            line("public method(s)").contains("115/117 matching app/services/billing/**/*.rb"),
            "got {}",
            line("public method(s)")
        );
        assert!(
            line("the entrypoint here").contains("1243/1245 matching app/services/**/*.rb"),
            "got {}",
            line("the entrypoint here")
        );
    }

    #[test]
    fn a_view_missing_the_format_segment_is_told_which_one_it_needs() {
        // Issue #16. Derived and never checked is the shape a rule has when it
        // costs budget and changes nothing. `show.erb` is not a template Rails
        // will render for an HTML request.
        let mut rule = conv("format.app.views.orders.erb", "Files here are named `*.html.erb`");
        rule.scope = Scope::DirExt("app/views/orders".into(), "erb".into());

        let bad =
            verify_source("app/views/orders/show.erb", "<h1>x</h1>", std::slice::from_ref(&rule));
        assert_eq!(bad.len(), 1, "got {bad:#?}");
        assert!(bad[0].message.contains("`*.html.erb`"), "got {}", bad[0].message);

        let good = verify_source(
            "app/views/orders/show.html.erb",
            "<h1>x</h1>",
            std::slice::from_ref(&rule),
        );
        assert!(good.is_empty(), "a conforming view was reported: {good:#?}");

        // A different format is a different kind of view, and the rule is
        // advisory precisely so this can be said without refusing it.
        let other =
            verify_source("app/views/orders/show.json.erb", "{}", std::slice::from_ref(&rule));
        assert_eq!(other.len(), 1, "got {other:#?}");
    }

    #[test]
    fn a_component_that_picks_the_other_export_style_is_told() {
        // Issue #16. Derived and never checked is the shape a rule has when it
        // costs budget and changes nothing.
        let mut default_rule =
            conv("shape.export.src.components.tsx", "Files here export a default");
        default_rule.scope = Scope::DirExt("src/components".into(), "tsx".into());

        let named = verify_source(
            "src/components/Card.tsx",
            "export const Card = () => null;",
            std::slice::from_ref(&default_rule),
        );
        assert_eq!(named.len(), 1, "got {named:#?}");
        assert!(named[0].message.contains("default"), "got {}", named[0].message);

        let conforming = verify_source(
            "src/components/Card.tsx",
            "const Card = () => null;\nexport default Card;",
            std::slice::from_ref(&default_rule),
        );
        assert!(conforming.is_empty(), "a conforming component was reported: {conforming:#?}");

        // And the other direction.
        let mut named_rule = conv("shape.export.src.widgets.tsx", "Files here use named exports");
        named_rule.scope = Scope::DirExt("src/widgets".into(), "tsx".into());
        let offender = verify_source(
            "src/widgets/Card.tsx",
            "const Card = () => null;\nexport default Card;",
            std::slice::from_ref(&named_rule),
        );
        assert_eq!(offender.len(), 1, "got {offender:#?}");

        // A module that exports nothing has made no choice, and saying it broke
        // one is the check being wrong about a file that is simply different.
        let silent = verify_source(
            "src/widgets/types.tsx",
            "type Card = { id: string };",
            std::slice::from_ref(&default_rule),
        );
        assert!(silent.is_empty(), "a module that exports nothing was judged: {silent:#?}");
    }

    #[test]
    fn a_php_file_in_the_wrong_namespace_is_told_which_one_its_neighbours_use() {
        let mut rule = conv(
            "shape.namespace.src.Services.Billing.php",
            "Files here declare namespace `App\\Services\\Billing`",
        );
        rule.scope = Scope::DirExt("src/Services/Billing".into(), "php".into());

        let wrong = verify_source(
            "src/Services/Billing/ChargeCard.php",
            "<?php\nnamespace App\\Services\\Payments;\nclass ChargeCard { public function handle() {} }\n",
            std::slice::from_ref(&rule),
        );
        assert_eq!(wrong.len(), 1, "got {wrong:#?}");
        assert!(wrong[0].message.contains("App\\Services\\Payments"), "got {}", wrong[0].message);

        let missing = verify_source(
            "src/Services/Billing/ChargeCard.php",
            "<?php\nclass ChargeCard { public function handle() {} }\n",
            std::slice::from_ref(&rule),
        );
        assert_eq!(missing.len(), 1, "got {missing:#?}");
        assert!(missing[0].message.contains("declares no namespace"), "got {}", missing[0].message);

        let right = verify_source(
            "src/Services/Billing/ChargeCard.php",
            "<?php\nnamespace App\\Services\\Billing;\nclass ChargeCard { public function handle() {} }\n",
            std::slice::from_ref(&rule),
        );
        assert!(right.is_empty(), "a conforming file was reported: {right:#?}");
    }

    #[test]
    fn the_format_rule_judges_only_the_files_it_was_counted_over() {
        // Deriving excludes a file and then checking judges it against the
        // resulting rule: the defect behind every false positive fourteen real
        // repositories produced. A Rails partial is named by the framework, a
        // dotfile has no name root at all, and neither was in the sample.
        let mut rule = conv("format.app.views.orders.erb", "Files here are named `*.html.erb`");
        rule.scope = Scope::DirExt("app/views/orders".into(), "erb".into());
        for excluded in [
            "app/views/orders/_row.json.erb", // a partial: role-marked
            "app/views/orders/.keep.erb",     // no name root
            "app/views/orders/show_spec.erb", // a test
        ] {
            assert!(
                verify_source(excluded, "<h1>x</h1>", std::slice::from_ref(&rule)).is_empty(),
                "{excluded} was judged by a rule derived without it"
            );
        }
        // An ordinary view is still told.
        assert_eq!(verify_source("app/views/orders/show.json.erb", "{}", &[rule]).len(), 1);
    }

    #[test]
    fn the_new_checks_survive_input_that_declares_nothing() {
        // Hostile shapes for the three families added for issue #16: a file
        // that is empty, one that is not the language it claims, and one that
        // declares a namespace and nothing else.
        let mut export_rule = conv("shape.export.src.tsx", "Files here export a default");
        export_rule.scope = Scope::DirExt("src".into(), "tsx".into());
        let mut ns_rule = conv("shape.namespace.src.php", "Files here declare namespace `App`");
        ns_rule.scope = Scope::DirExt("src".into(), "php".into());

        for (rel, source) in [
            ("src/Empty.tsx", ""),
            ("src/Broken.tsx", "\u{0}\u{1} not code at all"),
            ("src/Empty.php", "<?php"),
            ("src/Broken.php", "<?php class {{{"),
        ] {
            let _ = verify_source(rel, source, &[export_rule.clone(), ns_rule.clone()]);
        }

        // A namespace and nothing else is still a fact, and still checkable.
        let bare = verify_source("src/Bare.php", "<?php\nnamespace Other;\n", &[ns_rule]);
        assert_eq!(bare.len(), 1, "got {bare:#?}");
        assert!(bare[0].message.contains("`Other`"), "got {}", bare[0].message);
    }

    #[test]
    fn two_scopes_that_disagree_still_produce_one_line_for_one_defect() {
        // The claim is the defect, not the answer. Two rules of one family
        // derived at two levels can disagree — a billing subtree with its own
        // base class under a services tree with another — and keying the
        // deduplication on the expected value let both through, so one wrong
        // base class produced two lines telling the author two different
        // things. The narrowest scope is the one that describes this file.
        let mut wide =
            conv("shape.base.app.services.rb", "Types here inherit from `ApplicationService`");
        wide.scope = Scope::DirExt("app/services".into(), "rb".into());
        wide.agreeing = 90;
        wide.total = 100;
        let mut narrow =
            conv("shape.base.app.services.billing.rb", "Types here inherit from `BillingBase`");
        narrow.scope = Scope::DirExt("app/services/billing".into(), "rb".into());
        narrow.agreeing = 10;
        narrow.total = 10;

        let found = verify_source(
            "app/services/billing/refund.rb",
            "class Refund < Other\n  def call; end\nend\n",
            &[wide, narrow],
        );
        assert_eq!(found.len(), 1, "one defect, one line: {found:#?}");
        assert!(
            found[0].message.contains("`BillingBase`"),
            "the wider rule won: {}",
            found[0].message
        );
    }

    #[test]
    fn a_namespace_rule_speaks_only_for_the_directory_it_names() {
        // PSR-4 makes namespace and directory agree, so a subdirectory's
        // namespace differs from its parent's by definition. Scoped with the
        // `**` a directory rule normally carries, the parent's rule reaches
        // every descendant and is guaranteed wrong about all of them — for
        // the one family whose whole premise is that the two mirror.
        let mut rule = conv(
            "shape.namespace.src.Services.Billing.php",
            "Files here declare namespace `App\\Services\\Billing`",
        );
        rule.scope = Scope::DirExt("src/Services/Billing".into(), "php".into());

        let nested = verify_source(
            "src/Services/Billing/Invoices/Generate.php",
            "<?php\nnamespace App\\Services\\Billing\\Invoices;\nclass Generate { public function handle() {} }\n",
            std::slice::from_ref(&rule),
        );
        assert!(nested.is_empty(), "a correct PSR-4 subdirectory was reported: {nested:#?}");

        // Directly in the directory it names, it still holds.
        let beside = verify_source(
            "src/Services/Billing/Charge.php",
            "<?php\nnamespace App\\Services\\Payments;\nclass Charge { public function handle() {} }\n",
            &[rule],
        );
        assert_eq!(beside.len(), 1, "got {beside:#?}");
    }

    #[test]
    fn a_file_that_declares_nothing_is_not_told_it_forgot_a_namespace() {
        // Deriving drops a file whose facts are empty before it can vote, so
        // checking must not judge it against the resulting rule.
        let mut rule = conv("shape.namespace.src.php", "Files here declare namespace `App`");
        rule.scope = Scope::DirExt("src".into(), "php".into());
        assert!(
            verify_source("src/Blank.php", "<?php\n", &[rule]).is_empty(),
            "a file that declares nothing was judged"
        );
    }

    #[test]
    fn the_families_added_for_templates_and_modules_can_never_refuse_a_write() {
        // All three are advisory by construction: none of their id prefixes is
        // in the enforceable set, and `blocking_violations` recomputes the
        // grade from the id rather than trusting the stored decision. Pinned
        // here because renaming one under `naming.` would silently promote it.
        let settings = canon_core::Settings::default();
        for (id, statement, scope) in [
            (
                "format.app.views.erb",
                "Files here are named `*.html.erb`",
                Scope::DirExt("app/views".into(), "erb".into()),
            ),
            (
                "shape.export.src.tsx",
                "Files here export a default",
                Scope::DirExt("src".into(), "tsx".into()),
            ),
            (
                "shape.namespace.src.php",
                "Files here declare namespace `App`",
                Scope::DirExt("src".into(), "php".into()),
            ),
        ] {
            let rule = blocking(id, statement, scope);
            assert_eq!(
                rule.enforcement_now(&settings),
                Enforcement::Advisory,
                "{id} was graded enforceable at total agreement"
            );
        }
    }

    #[test]
    fn two_scopes_naming_two_different_imports_both_get_said() {
        // Imports are the one family where two scopes can both be right. A
        // service directory may require the shared client and a billing
        // subdirectory may additionally require the ledger; neither answer
        // replaces the other, so collapsing them drops a fact the author has
        // to act on separately.
        let mut wide = conv("shape.import.app.services.ts", "Files here import from `@repo/ui`");
        wide.scope = Scope::DirExt("app/services".into(), "ts".into());
        let mut narrow =
            conv("shape.import.app.services.billing.ts", "Files here import from `@repo/db`");
        narrow.scope = Scope::DirExt("app/services/billing".into(), "ts".into());

        let found = verify_source(
            "app/services/billing/refund.ts",
            "import { x } from '@repo/other';\nexport const a = () => x;\n",
            &[wide, narrow],
        );
        assert_eq!(found.len(), 2, "one import rule silenced the other: {found:#?}");
    }

    #[test]
    fn a_module_that_declares_nothing_is_not_judged_on_how_much_it_exports() {
        // Deriving drops a file whose facts are empty before it can vote. A
        // TSX file holding one type alias declares nothing this reads, so it
        // never voted and must not be told it exports too few functions.
        let mut rule =
            conv("shape.module-arity.src.widgets.tsx", "Files here export exactly 1 function");
        rule.scope = Scope::DirExt("src/widgets".into(), "tsx".into());
        assert!(
            verify_source("src/widgets/types.tsx", "type Card = { id: string };", &[rule])
                .is_empty(),
            "a file that declares nothing was judged on its export count"
        );
    }

    #[test]
    fn whether_a_file_is_judged_does_not_depend_on_which_other_rules_apply() {
        // `verify_with` reads a file two ways: the cheap structural pass, and
        // the full one when an import rule is in scope. Only the second
        // records calls, so a predicate that consults them answers differently
        // depending on a rule that has nothing to do with the question.
        let mut ns = conv("shape.namespace.src.php", "Files here declare namespace `App`");
        ns.scope = Scope::DirExt("src".into(), "php".into());
        let mut import = conv("shape.import.src.php", "Files here import from `App\\Kernel`");
        import.scope = Scope::DirExt("src".into(), "php".into());

        let procedural = "<?php\nadd_action('init', 'setup');\n";
        let alone = verify_source("src/plugin.php", procedural, std::slice::from_ref(&ns));
        let beside = verify_source("src/plugin.php", procedural, &[ns, import]);
        let namespace_lines = |vs: &[Violation]| {
            vs.iter().filter(|v| v.convention_id.starts_with("shape.namespace")).count()
        };
        assert_eq!(
            namespace_lines(&alone),
            namespace_lines(&beside),
            "an unrelated import rule changed the namespace answer:\n{alone:#?}\n{beside:#?}"
        );
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
        // Every top-level directory a test path can sit in, so a fixture that
        // is not about scope-versus-sample is not silenced by that guard.
        c.sample_roots =
            ["", "app", "src", "pages", "lib", "spec", "docs", "packages", "test", "tests"]
                .iter()
                .map(|s| (*s).to_string())
                .collect();
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
    fn a_file_the_framework_named_is_not_refused_for_the_name_it_was_given() {
        // Next.js, Nuxt and SvelteKit all name route files with characters no
        // style admits. Reading "compatible with nothing" as "breaks the rule"
        // refused every one of them, and the author cannot rename them.
        let settings = canon_core::Settings::default();
        let rule = blocking(
            "naming.repo.tsx",
            "Files here are named in kebab-case",
            Scope::Ext("tsx".into()),
        );
        for rel in [
            "pages/posts/[id].tsx",
            "pages/[...slug].tsx",
            "src/routes/+page.server.tsx",
            "app/views/_form.tsx",
        ] {
            assert!(
                blocking_violations(
                    rel,
                    Some("export const A = 1;".into()),
                    std::slice::from_ref(&rule),
                    &settings
                )
                .is_empty(),
                "{rel} is named by the framework, not by its author"
            );
        }
        // A name that picked the wrong style is still a violation.
        assert_eq!(
            blocking_violations(
                "pages/MyPage.tsx",
                Some("export const A = 1;".into()),
                &[rule],
                &settings
            )
            .len(),
            1
        );
    }

    #[test]
    fn a_name_that_distinguishes_no_style_cannot_break_one() {
        let settings = canon_core::Settings::default();
        let rule = blocking(
            "naming.repo.ts",
            "Files here are named in camelCase",
            Scope::Ext("ts".into()),
        );
        for rel in ["src/404.ts", "src/請求書.ts", "src/2fa.ts"] {
            assert!(
                blocking_violations(
                    rel,
                    Some("export const a = 1;".into()),
                    std::slice::from_ref(&rule),
                    &settings
                )
                .is_empty(),
                "{rel} has no case to read and no separator to read it at"
            );
        }
    }

    #[test]
    fn a_rule_does_not_refuse_a_directory_or_a_qualifier_its_sample_never_saw() {
        let settings = canon_core::Settings::default();

        // Counted in `docs/`, so it speaks for `docs/`.
        let mut docs = blocking(
            "naming.repo.md",
            "Files here are named in kebab-case",
            Scope::Ext("md".into()),
        );
        docs.evidence = ["getting-started", "api-reference", "style-guide"]
            .iter()
            .map(|n| canon_core::Evidence { rel: format!("docs/{n}.md"), line: 0 })
            .collect();
        docs.sample_roots = vec!["docs".to_string()];
        assert!(
            blocking_violations(
                ".github/PULL_REQUEST_TEMPLATE.md",
                Some("## What\n".into()),
                &[docs.clone()],
                &settings
            )
            .is_empty(),
            "a rule counted in docs/ refused a file the repository's tooling requires"
        );
        assert_eq!(
            blocking_violations("docs/BadName.md", Some("# x\n".into()), &[docs], &settings).len(),
            1,
            "and still holds inside the directory it was counted in"
        );

        // Counted over CSS modules, so it says nothing about a plain stylesheet.
        let mut modules = blocking(
            "naming.src.css",
            "Files here are named in PascalCase",
            Scope::DirExt("src".into(), "css".into()),
        );
        modules.evidence = ["Button", "Card", "Modal"]
            .iter()
            .map(|n| canon_core::Evidence { rel: format!("src/{n}.module.css"), line: 0 })
            .collect();
        modules.sample_roots = vec!["src".to_string()];
        assert!(
            blocking_violations("src/globals.css", Some(".a{}".into()), &[modules], &settings)
                .is_empty()
        );
    }

    #[test]
    fn a_naming_rule_speaks_for_its_own_scope_root_not_only_for_what_is_below_it() {
        // Issue #18. The guard asked only whether the file's directory sat
        // *under* a sampled one. Where every sampled file lives in a
        // subdirectory, the scope root is an ancestor of all of them and
        // answers neither question, so one badly-named file was refused in
        // every subdirectory the rule sampled and allowed at the top of the
        // scope — same rule, same confidence, same name.
        let settings = canon_core::Settings::default();
        let mut rule = blocking(
            "naming.src.components.area.group.tsx",
            "Files here are named in PascalCase",
            Scope::DirExt("src/components/area/group".into(), "tsx".into()),
        );
        rule.evidence = ["sub-a/Alpha", "sub-b/Beta", "sub-b/nested/Gamma"]
            .iter()
            .map(|n| canon_core::Evidence {
                rel: format!("src/components/area/group/{n}.tsx"),
                line: 0,
            })
            .collect();
        rule.sample_roots = [
            "src/components/area/group/sub-a",
            "src/components/area/group/sub-b",
            "src/components/area/group/sub-b/nested",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();

        let source = Some("export const A = 1;".to_string());
        for rel in [
            "src/components/area/group/sub-a/zz-probe.tsx",
            "src/components/area/group/sub-b/nested/zz-probe.tsx",
            "src/components/area/group/zz-probe.tsx",
        ] {
            assert_eq!(
                blocking_violations(rel, source.clone(), std::slice::from_ref(&rule), &settings)
                    .len(),
                1,
                "{rel} escaped a rule whose own scope covers it"
            );
        }

        // And the reason the fix has to read the scope: a rule with no
        // directory prefix has the repository root as the ancestor of every
        // sample it took, so accepting the ancestor direction there would make
        // the guard admit everything. A `**/*.md` rule counted in `docs/` must
        // still say nothing about a file at the root.
        let mut anywhere = blocking(
            "naming.repo.md",
            "Files here are named in kebab-case",
            Scope::Ext("md".into()),
        );
        anywhere.evidence = ["getting-started", "api-reference", "style-guide"]
            .iter()
            .map(|n| canon_core::Evidence { rel: format!("docs/{n}.md"), line: 0 })
            .collect();
        anywhere.sample_roots = vec!["docs".to_string()];
        assert!(
            blocking_violations(
                "RootThing.md",
                Some("# x\n".into()),
                std::slice::from_ref(&anywhere),
                &settings
            )
            .is_empty(),
            "a rule counted in docs/ refused a file at the repository root"
        );
        assert_eq!(
            blocking_violations("docs/RootThing.md", Some("# x\n".into()), &[anywhere], &settings)
                .len(),
            1,
            "and still holds inside the directory it was counted in"
        );
    }

    #[test]
    fn one_file_sampled_at_the_repository_root_does_not_license_every_directory() {
        // `sample_roots` records the repository root as the empty string, and
        // "is this directory below the empty string" is true of every
        // directory there is. One counted root file therefore reinstated the
        // reading the guard exists to prevent, on the refusal path.
        let settings = canon_core::Settings::default();
        let mut rule = blocking(
            "naming.repo.md",
            "Files here are named in kebab-case",
            Scope::Ext("md".into()),
        );
        rule.evidence = ["getting-started", "api-reference", "style-guide"]
            .iter()
            .map(|n| canon_core::Evidence { rel: format!("docs/{n}.md"), line: 0 })
            .collect();
        rule.sample_roots = vec![String::new(), "docs".to_string()];
        assert!(
            blocking_violations(
                ".github/PULL_REQUEST_TEMPLATE.md",
                Some("## What\n".into()),
                std::slice::from_ref(&rule),
                &settings
            )
            .is_empty(),
            "one sampled root file let the rule refuse a directory it never counted"
        );
        // The directories it did count are still held.
        assert_eq!(
            blocking_violations(
                "docs/BadName.md",
                Some("# x\n".into()),
                std::slice::from_ref(&rule),
                &settings
            )
            .len(),
            1
        );
        assert_eq!(
            blocking_violations("BadName.md", Some("# x\n".into()), &[rule], &settings).len(),
            1,
            "and the root itself, which is what the empty string records"
        );
    }

    #[test]
    fn the_first_test_written_into_a_directory_is_not_refused_by_its_code_rules() {
        // The sample cannot contain it yet, so the rule is still at total
        // agreement and refused it. A colocated `test_void_invoice.py` was told
        // it must inherit `BaseService` and expose one public method.
        //
        // A base read from a Python positional list is never Blocking, so the
        // arity rule is the only one left that can prove the exemption fires:
        // it refuses a shortfall against the expected count, never a surplus,
        // so the fixture below has zero public methods against an expected
        // one. The same content is written to a test path and a code path, so
        // path is the only variable. If the test-path exemption stopped
        // working, this same class would be refused there too.
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
        let code_file = "class VoidInvoice:\n    pass\n";
        for rel in [
            "app/services/test_void_invoice.py",
            "app/services/__tests__/test_void_invoice.py",
            "app/services/void_invoice_test.py",
        ] {
            assert!(
                blocking_violations(rel, Some(code_file.to_string()), &rules, &settings).is_empty(),
                "{rel} was refused for not being shaped like the code it tests"
            );
        }

        // The code beside it is still held to the rules.
        assert!(
            !blocking_violations(
                "app/services/void_invoice.py",
                Some(code_file.to_string()),
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
