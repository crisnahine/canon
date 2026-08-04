//! Structural facts, extracted from source with tree-sitter.
//!
//! Everything above this crate works on [`FileFacts`] and never touches a
//! grammar. That boundary is the whole point: adding a language is one module
//! here plus one arm in [`lang::provider`], and it cannot reach into the
//! derivation rules. If a new language required changing how conventions are
//! derived, the abstraction would be wrong.
//!
//! # Why visibility lives here and not in the derivation layer
//!
//! "Types expose exactly one public method" is the convention teams actually
//! hold, and *public* means something different in each language:
//!
//! | Language | Rule |
//! |---|---|
//! | Ruby | a bare `private` is a section keyword that flips everything after it |
//! | Go | the first letter's case |
//! | TypeScript | an `accessibility_modifier`, or a `#` prefix |
//! | Python | a leading underscore, by convention only |
//! | Rust | the `pub` keyword |
//! | PHP | a modifier, defaulting to public when absent |
//!
//! None of those reduce to each other. Pushing them up into the derivation
//! layer would put six special cases in the one place that must stay
//! language-agnostic, so each extractor resolves visibility itself and reports
//! two flat lists.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::too_many_lines
    )
)]

mod error;
pub mod lang;
mod query;
mod util;

mod ecma;
mod embedded;
mod golang;
mod php;
mod python;
mod ruby;
mod rustlang;

pub use error::{ExtractError, Result};
pub use lang::{Language, Provider, Visibility};
pub use query::Call;

use serde::{Deserialize, Serialize};

/// A type declaration and its resolved public surface.
///
/// "Type" covers whatever the language calls the thing that owns methods: a
/// Ruby class or module, a Go struct, a Rust struct or enum, a TS class, a PHP
/// class or interface.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeFacts {
    /// Declared name.
    pub name: String,
    /// One-based line of the declaration.
    pub line: usize,
    /// Methods callable from outside, after the language's visibility rules.
    pub public_methods: Vec<String>,
    /// Methods that are not.
    pub private_methods: Vec<String>,
    /// Base class, embedded type, or implemented trait, when the language has
    /// such a thing and the declaration names one.
    pub superclass: Option<String>,
    /// Every type this declaration names as a parent, in source order.
    ///
    /// `superclass` is one of these, chosen for the statement a convention
    /// makes. Which one is language-specific and it is not always the first:
    /// Python's frameworks order a base list mixins-first, so the last entry
    /// is the type the class actually is, and Django states that ordering as a
    /// requirement rather than a preference. Go has no order at all, only a
    /// set of embedded fields.
    ///
    /// Empty for a language with a single base, where `superclass` says
    /// everything there is to say.
    pub bases: Vec<String>,
    /// Every contract the type declares, when the language lets it declare more
    /// than one: Rust's trait impls, PHP's `implements`, TypeScript's
    /// `implements`. `superclass` is one of these, chosen for the statement; a
    /// check that only compared against that one refused a file for the order
    /// its `impl` blocks were written in.
    pub interfaces: Vec<String>,
    /// The entries of `bases` that are not `superclass`, for a language where
    /// that split is source order rather than a keyword.
    ///
    /// Composition a type opts into, not the type it is, which is why it lives
    /// apart from both `superclass` and `interfaces`: a Django view's leading
    /// `LoginRequiredMixin` is not a contract the view declares the way
    /// `implements` is, and folding the two together would tell a later "types
    /// here implement X" rule that a mixin is an implemented interface.
    pub mixins: Vec<String>,
}

impl TypeFacts {
    /// Size of the public surface.
    #[must_use]
    pub fn public_arity(&self) -> usize {
        self.public_methods.len()
    }
}

/// One decorator, attribute or annotation, by the name it was written with.
///
/// The construct four of the six wired languages use to say what a file is.
/// A `NestJS` service is a plain class with `@Injectable()` on it, a Symfony
/// controller is a plain class with `#[Route]` on its methods, and a Rust DTO
/// is a plain struct with a derive list. None of those touch the base type,
/// the public method count, or the export style, which is why a directory of
/// them derived almost nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    /// The path as written: `pytest.fixture`, `ORM\Entity`, `tokio::main`.
    /// A Rust derive is one entry per trait, spelled `derive(Serialize)`,
    /// because the list is the convention and the word `derive` is not.
    pub name: String,
}

/// Everything one file contributes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFacts {
    /// Types declared in this file.
    pub types: Vec<TypeFacts>,
    /// Functions declared outside any type.
    pub free_functions: Vec<String>,
    /// Imported module paths, as written.
    pub imports: Vec<String>,
    /// Call sites, in source order.
    ///
    /// What a file reaches for is as much a convention as what it declares:
    /// "services here never call `ActiveRecord` directly" is a layering rule no
    /// linter checks and every team holds.
    pub calls: Vec<Call>,
    /// Exception types raised or thrown.
    pub raises: Vec<String>,
    /// Whether the module exports a default, for the languages that have one.
    ///
    /// A module's surface has a shape as well as a size, and a component tree
    /// picks one and holds it. Getting it wrong is the drift that type-checks:
    /// the file compiles, and every import of it has to be written the other
    /// way round.
    pub default_export: bool,
    /// The namespace this file declares, for the languages that have one.
    ///
    /// Its own field rather than a synthetic type, because a namespace is a
    /// property of the file and not of anything the file declares. PHP is the
    /// language it was added for: PSR-4 makes namespace and directory agree,
    /// which is a convention a team really holds and no other fact here can
    /// express.
    pub namespace: Option<String>,
    /// Decorators, attributes and annotations found anywhere in the file.
    ///
    /// Flat rather than attached to the declaration they annotate: the
    /// convention a directory holds is which annotations appear in a file of
    /// this kind, and resolving each one to its target costs a second pass for
    /// a distinction no rule currently makes.
    pub annotations: Vec<Annotation>,
}

impl FileFacts {
    /// Whether the file contributed anything worth deriving from.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
            && self.free_functions.is_empty()
            && self.imports.is_empty()
            && self.calls.is_empty()
            // Both are whole-file facts rather than declarations, so a file
            // whose only contribution is one of them has still contributed.
            // `export default Item;` declares nothing this reads as a free
            // function, and dropping it as empty silently removed exactly the
            // files a default-export rule is derived from.
            && !self.default_export
            && self.namespace.is_none()
            && self.annotations.is_empty()
    }
}

/// Parse `source` and return its structural facts.
///
/// Never panics on malformed input: tree-sitter recovers from syntax errors by
/// design, and a partial tree yields partial facts. That is the correct
/// behaviour here, because a repository always contains a file mid-edit, and
/// one broken file must not cost the repository its conventions.
///
/// # Errors
///
/// [`ExtractError::Unsupported`] when the language has no wired grammar.
/// Returning an error rather than empty facts is deliberate: empty facts are
/// indistinguishable from "this file genuinely declares nothing", and a
/// convention derived from files that were never parsed would be derived from
/// no evidence at all.
pub fn extract(language: Language, source: &str, path: &str) -> Result<FileFacts> {
    extract_inner(language, source, path, true)
}

/// Parse `source` for its declarations only, skipping the query pass.
///
/// Same structural facts — types, their public surface, free functions — and
/// no calls or raises. For the caller that has a written file in hand and a
/// set of shape rules to check it against, which is every write.
///
/// The difference is not small. Compiling a tree-sitter query happens once per
/// process, and a hook *is* one process per invocation, so the whole cost lands
/// on every write: measured on a real Rails repository, `inject` for a two-line
/// Ruby file spent 27 ms, of which 25 ms was compiling a query whose results
/// were then discarded. Structure alone is 2.5 ms.
///
/// # Errors
///
/// As [`extract`].
pub fn extract_structure(language: Language, source: &str, path: &str) -> Result<FileFacts> {
    extract_inner(language, source, path, false)
}

fn extract_inner(
    language: Language,
    source: &str,
    path: &str,
    with_query: bool,
) -> Result<FileFacts> {
    let provider = lang::provider(language);
    if !provider.grammar_ready {
        return Err(ExtractError::Unsupported { language: language.name() });
    }
    // A file that is two languages at once is parsed as the one canon has
    // conventions about, restricted to the byte ranges that language occupies.
    // The tree keeps the original offsets, so a line number still points at
    // the right line of the file.
    let (language, tree) = if let Some(found) = embedded::embedded_in(language, source) {
        let grammar = lang::grammar(found.language)
            .ok_or(ExtractError::Unsupported { language: found.language.name() })?;
        let tree =
            util::parse_ranges(&grammar, found.language.name(), source, path, &found.ranges)?;
        (found.language, tree)
    } else if let Some(grammar) = lang::grammar(language) {
        (language, util::parse(&grammar, language.name(), source, path)?)
    } else {
        // A language with no grammar of its own and nothing embedded: an ERB
        // template that is only markup, or a Vue component that is only a
        // `<template>`. Both are valid files that declare nothing, which is
        // not the same as a file canon failed to read.
        return Ok(FileFacts::default());
    };

    // One parse, two passes over the same tree. The structural pass resolves
    // what is stateful and language-specific; the query pass matches what is a
    // pattern. Parsing twice would double the cold path for no gain.
    let mut facts = match language {
        Language::Ruby => ruby::extract(&tree, source),
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => {
            ecma::extract(&tree, source)
        }
        Language::Python => python::extract(&tree, source),
        Language::Go => golang::extract(&tree, source),
        Language::Rust => rustlang::extract(&tree, source),
        Language::Php => php::extract(&tree, source),
        // Resolved above to the language they embed, or returned already when
        // the file contained none of it.
        Language::Vue | Language::Erb => FileFacts::default(),
    };

    // One name is one method. A type cannot expose two methods under the same
    // name in any build, so a repeat means either a conditional alternative or
    // an overload signature, and counting both inflates the arity:
    //
    //   #[cfg(unix)]    mod imp { impl super::Handle { pub fn open(&self) {} } }
    //   #[cfg(windows)] mod imp { impl super::Handle { pub fn open(&self) {} } }
    //
    //   foo(a: string): void;
    //   foo(a: number): void;
    //
    // The first was refused by an arity rule it satisfies; the second is one
    // method however many signatures declare it. Neither is visible to a
    // parser, which sees every branch at once.
    for t in &mut facts.types {
        dedupe(&mut t.public_methods);
        dedupe(&mut t.private_methods);
        dedupe(&mut t.interfaces);
    }

    if with_query {
        let found = query::run(language, &tree, source);
        facts.calls = found.calls;
        facts.raises = found.raises;
        facts.annotations = found.annotations;
        // The query is the better import extractor where it has one: it reads
        // the field rather than the first string it finds. Where it has none,
        // the structural pass already filled these in.
        if !found.imports.is_empty() {
            facts.imports = found.imports;
        }
    }
    Ok(facts)
}

/// Remove repeats, keeping the first of each and the original order.
fn dedupe(names: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    names.retain(|n| seen.insert(n.clone()));
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Parse and extract, the way [`extract`] does, for an extractor's own
    /// tests. They exercise the structural pass, which no longer parses.
    pub(crate) fn facts_of(language: Language, source: &str) -> FileFacts {
        extract(language, source, "test-input").unwrap_or_default()
    }

    #[test]
    fn a_language_with_no_grammar_errors_rather_than_reporting_no_types() {
        // Empty facts and "canon could not read this" must never look the
        // same: a convention derived from files that were never parsed would
        // be derived from no evidence at all. Every language is wired today,
        // so this asserts the mechanism rather than a particular language, and
        // it is what will catch the next one added without a grammar.
        for language in Language::ALL {
            if lang::provider(*language).grammar_ready {
                continue;
            }
            let err = extract(*language, "anything", "a.txt").unwrap_err();
            assert!(matches!(err, ExtractError::Unsupported { .. }), "{}", language.name());
        }
    }

    #[test]
    fn a_template_carrying_no_code_declares_nothing_rather_than_failing() {
        // Different from the case above. A Vue component that is only markup,
        // or an ERB template with no tags, was read successfully and genuinely
        // declares nothing.
        for (language, source) in
            [(Language::Vue, "<template><div/></template>"), (Language::Erb, "<p>hi</p>")]
        {
            let facts = extract(language, source, "a.tmpl").expect("read, not failed");
            assert!(facts.is_empty(), "{}", language.name());
        }
    }

    #[test]
    fn an_embedded_file_is_parsed_as_the_language_it_carries() {
        let erb =
            extract(Language::Erb, "<div>\n  <% Payment.charge(1) %>\n</div>\n", "show.html.erb")
                .expect("parses");
        assert!(
            erb.calls.iter().any(|c| c.name == "charge"),
            "the Ruby between the tags was not read: {:?}",
            erb.calls
        );

        let vue = extract(
            Language::Vue,
            "<template><div/></template>\n<script lang=\"ts\">\nexport class Card { render() {} }\n</script>\n",
            "Card.vue",
        )
        .expect("parses");
        assert_eq!(vue.types.first().map(|t| t.name.as_str()), Some("Card"));
    }

    #[test]
    fn every_wired_language_survives_hostile_input() {
        // The fail-open contract starts here. Anything that panics in an
        // extractor kills the hook process before it can emit valid JSON.
        let hostile = [
            "",
            "\u{0}\u{1}\u{2}",
            "{{{{{{{{",
            "\u{feff}class",
            "class A { { { {",
            &"a".repeat(10_000),
        ];
        for lang in Language::ALL {
            if !lang::provider(*lang).grammar_ready {
                continue;
            }
            // A delegating language yields empty facts when the host file
            // contains none of the embedded language, which is correct rather
            // than a failure to parse.
            for src in &hostile {
                let got = extract(*lang, src, "hostile.txt");
                assert!(
                    got.is_ok(),
                    "{} panicked or errored on {:?}",
                    lang.name(),
                    &src[..8.min(src.len())]
                );
            }
        }
    }

    #[test]
    fn public_arity_counts_only_the_public_list() {
        let t = TypeFacts {
            name: "Create".into(),
            line: 1,
            public_methods: vec!["call".into()],
            private_methods: vec!["a".into(), "b".into()],
            superclass: None,
            bases: Vec::new(),
            interfaces: Vec::new(),
            mixins: Vec::new(),
        };
        assert_eq!(t.public_arity(), 1);
    }
}
