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
}

impl TypeFacts {
    /// Size of the public surface.
    #[must_use]
    pub fn public_arity(&self) -> usize {
        self.public_methods.len()
    }
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
}

impl FileFacts {
    /// Whether the file contributed anything worth deriving from.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
            && self.free_functions.is_empty()
            && self.imports.is_empty()
            && self.calls.is_empty()
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
    let provider = lang::provider(language);
    if !provider.grammar_ready {
        return Err(ExtractError::Unsupported { language: language.name() });
    }
    let grammar =
        lang::grammar(language).ok_or(ExtractError::Unsupported { language: language.name() })?;
    let tree = util::parse(&grammar, language.name(), source, path)?;

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
        Language::Vue => return Err(ExtractError::Unsupported { language: language.name() }),
    };

    let found = query::run(language, &tree, source);
    facts.calls = found.calls;
    facts.raises = found.raises;
    // The query is the better import extractor where it has one: it reads the
    // field rather than the first string it finds. Where it has none, the
    // structural pass already filled these in.
    if !found.imports.is_empty() {
        facts.imports = found.imports;
    }
    Ok(facts)
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
    fn an_unwired_language_errors_rather_than_reporting_no_types() {
        let err = extract(Language::Vue, "<template></template>", "a.vue").unwrap_err();
        assert!(matches!(err, ExtractError::Unsupported { .. }));
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
        };
        assert_eq!(t.public_arity(), 1);
    }
}
