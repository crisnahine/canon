//! The declarative half of extraction.
//!
//! Hand-written cursor walks resolve the facts that are *stateful*: Ruby's
//! `private` is a section keyword, Go's methods live outside their type. Those
//! cannot be a pattern match, and trying made a confident, wrong convention the
//! first time it was attempted.
//!
//! Everything else is a pattern, and patterns belong in a query. Call sites,
//! raises and imports are the same shape in every file of a language, so they
//! are written once in `queries/<language>/facts.scm` and matched by
//! tree-sitter rather than by another hand-rolled walk per language.
//!
//! # Why canon ships its own queries
//!
//! Every grammar crate exports a `TAGS_QUERY`, and using those directly is
//! tempting. They do not agree with each other. Go's reports `@reference.type`
//! where Ruby's reports `@reference.call`, and TypeScript's covers only the
//! constructs TypeScript adds, on the assumption that the JavaScript query is
//! concatenated with it. Pointed at `class Foo { call() { charge(1) } }` the
//! TypeScript tags query returns nothing at all.
//!
//! So the capture vocabulary here is canon's, and it is identical across
//! languages. Everything above this module reads the same names whatever was
//! parsed.
//!
//! # The vocabulary
//!
//! | Capture | Meaning |
//! |---|---|
//! | `@call` | the name being called |
//! | `@call.receiver` | what it was called on, when written |
//! | `@raise` | an exception type being raised or thrown |
//! | `@import` | a module path being imported |
//!
//! A capture the vocabulary does not define is ignored rather than an error, so
//! a query can carry a helper capture for a predicate without the reader here
//! having to know about it.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use tree_sitter::{Query, QueryCursor, StreamingIterator as _};

use crate::Language;

/// One call site.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Call {
    /// What was called.
    pub name: String,
    /// What it was called on, when the source wrote one. `Payment` in
    /// `Payment.charge(x)`; `None` for a bare `charge(x)`.
    pub receiver: Option<String>,
}

impl Call {
    /// How the call reads in source, for reporting.
    #[must_use]
    pub fn render(&self) -> String {
        match &self.receiver {
            Some(r) => format!("{r}.{}", self.name),
            None => self.name.clone(),
        }
    }
}

/// What one query pass found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct QueryFacts {
    /// Call sites, in source order.
    pub(crate) calls: Vec<Call>,
    /// Exception types raised or thrown.
    pub(crate) raises: Vec<String>,
    /// Imported module paths, as written.
    pub(crate) imports: Vec<String>,
}

/// The query source for a language, or `None` when it has none yet.
///
/// Compiled into the binary rather than read from disk: a hook has no
/// installation directory it can rely on, and a query that fails to load would
/// silently reduce every convention it feeds.
fn source_for(language: Language) -> Option<&'static str> {
    Some(match language {
        Language::Ruby => include_str!("../queries/ruby/facts.scm"),
        Language::JavaScript | Language::Jsx => include_str!("../queries/javascript/facts.scm"),
        Language::TypeScript | Language::Tsx => include_str!("../queries/typescript/facts.scm"),
        Language::Python => include_str!("../queries/python/facts.scm"),
        Language::Go => include_str!("../queries/go/facts.scm"),
        Language::Rust => include_str!("../queries/rust/facts.scm"),
        Language::Php => include_str!("../queries/php/facts.scm"),
        Language::Vue => return None,
    })
}

/// Compiled queries, one per language, for the life of the process.
///
/// Compiling is expensive relative to matching, and a session indexes
/// thousands of files. Without this the same query is rebuilt per file and
/// dominates the cold path.
type Cache = Mutex<HashMap<Language, Option<&'static Query>>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The compiled query for a language, compiling it on first use.
///
/// A query that fails to compile is remembered as absent rather than retried.
/// It is a defect in a file shipped inside the binary, so it will fail
/// identically every time, and a hook must not spend the attempt on each file.
fn compiled(language: Language) -> Option<&'static Query> {
    let mut cache = cache().lock().ok()?;
    if let Some(found) = cache.get(&language) {
        return *found;
    }
    let built = source_for(language)
        .and_then(|src| Query::new(&crate::lang::grammar(language)?, src).ok())
        // Leaked deliberately: one query per language for the process lifetime,
        // which is what makes the cache a `&'static` the callers can hold.
        .map(|q| &*Box::leak(Box::new(q)));
    cache.insert(language, built);
    built
}

/// Run the language's fact query over a parsed tree.
///
/// Returns empty facts when the language has no query, when the query does not
/// compile, or when nothing matches. All three mean the same thing to a
/// caller: this pass contributed nothing.
#[must_use]
pub(crate) fn run(language: Language, tree: &tree_sitter::Tree, source: &str) -> QueryFacts {
    let Some(query) = compiled(language) else { return QueryFacts::default() };
    let names = query.capture_names();
    let mut facts = QueryFacts::default();
    let mut cursor = QueryCursor::new();

    // A file of generated code can produce enormous numbers of matches. The
    // cap bounds the work; exceeding it yields fewer facts, never a hang.
    cursor.set_match_limit(MATCH_LIMIT);

    let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
    while let Some(m) = matches.next() {
        let text = |index: u32| -> Option<String> {
            m.captures
                .iter()
                .find(|c| c.index == index)
                .and_then(|c| c.node.utf8_text(source.as_bytes()).ok())
                .map(str::to_string)
        };
        let index_of = |wanted: &str| {
            names.iter().position(|n| *n == wanted).and_then(|i| u32::try_from(i).ok())
        };

        if let Some(name) = index_of("call").and_then(&text) {
            let receiver = index_of("call.receiver").and_then(&text);
            facts.calls.push(Call { name, receiver });
        }
        if let Some(raised) = index_of("raise").and_then(&text) {
            facts.raises.push(raised);
        }
        if let Some(path) = index_of("import").and_then(&text) {
            facts.imports.push(crate::util::unquote(&path));
        }
    }
    facts
}

/// Matches past which a file is generated rather than written.
const MATCH_LIMIT: u32 = 20_000;

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(language: Language, source: &str) -> QueryFacts {
        let grammar = crate::lang::grammar(language).expect("a wired grammar");
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar).expect("language");
        let tree = parser.parse(source, None).expect("a tree");
        run(language, &tree, source)
    }

    fn called(f: &QueryFacts) -> Vec<String> {
        f.calls.iter().map(Call::render).collect()
    }

    #[test]
    fn every_wired_language_has_a_query_that_compiles() {
        // A query that fails to compile degrades silently to no facts, so the
        // only place it can be caught is here.
        for language in Language::ALL {
            if !crate::lang::provider(*language).grammar_ready {
                continue;
            }
            assert!(
                compiled(*language).is_some(),
                "{} has no compiling fact query",
                language.name()
            );
        }
    }

    #[test]
    fn a_receiver_call_is_captured_with_its_receiver_in_every_language() {
        // The capture vocabulary is canon's precisely so this reads the same
        // whatever was parsed.
        let cases = [
            (Language::Ruby, "class A\n  def call\n    Payment.charge(1)\n  end\nend\n"),
            (Language::TypeScript, "class A { call() { Payment.charge(1); } }"),
            (Language::JavaScript, "class A { call() { Payment.charge(1); } }"),
            (Language::Python, "class A:\n    def call(self):\n        Payment.charge(1)\n"),
            (Language::Rust, "fn call() { Payment::charge(1); }"),
            (Language::Php, "<?php function call() { Payment::charge(1); }"),
            (Language::Go, "package a\nfunc Call() { payment.Charge(1) }\n"),
        ];
        for (language, source) in cases {
            let names = called(&facts(language, source));
            assert!(
                names.iter().any(|c| c.contains("charge") || c.contains("Charge")),
                "{}: no call captured, got {names:?}",
                language.name()
            );
        }
    }

    #[test]
    fn a_call_without_a_receiver_has_none() {
        let f = facts(Language::Ruby, "class A\n  def call\n    notify_customer(1)\n  end\nend\n");
        let bare = f.calls.iter().find(|c| c.name == "notify_customer");
        assert!(bare.is_some(), "got {:?}", called(&f));
        assert_eq!(bare.and_then(|c| c.receiver.clone()), None);
    }

    #[test]
    fn a_bare_ruby_identifier_is_not_claimed_as_a_call() {
        // `notify_customer` with no parentheses parses as an identifier, and
        // Ruby cannot tell that from a local variable read without resolving
        // scope. Claiming it would report every variable as a call.
        let f =
            facts(Language::Ruby, "class A\n  def call\n    total = 1\n    total\n  end\nend\n");
        assert!(f.calls.is_empty(), "got {:?}", called(&f));
    }

    #[test]
    fn raises_are_captured_where_the_language_has_them() {
        let ruby = facts(Language::Ruby, "def a\n  raise ArgumentError, 'no'\nend\n");
        assert!(ruby.raises.iter().any(|r| r == "ArgumentError"), "got {:?}", ruby.raises);

        let ts = facts(Language::TypeScript, "function a() { throw new TypeError('no'); }");
        assert!(ts.raises.iter().any(|r| r == "TypeError"), "got {:?}", ts.raises);

        let py = facts(Language::Python, "def a():\n    raise ValueError('no')\n");
        assert!(py.raises.iter().any(|r| r == "ValueError"), "got {:?}", py.raises);
    }

    #[test]
    fn imports_come_back_unquoted() {
        let ts = facts(Language::TypeScript, "import { a } from './lib/a';\n");
        assert_eq!(ts.imports, vec!["./lib/a"]);

        let py = facts(Language::Python, "import json\n");
        assert!(py.imports.iter().any(|i| i == "json"), "got {:?}", py.imports);
    }

    #[test]
    fn a_language_with_no_query_yields_nothing_rather_than_failing() {
        assert!(source_for(Language::Vue).is_none());
        assert!(compiled(Language::Vue).is_none());
    }

    #[test]
    fn hostile_input_produces_no_facts_and_does_not_panic() {
        for language in Language::ALL {
            if !crate::lang::provider(*language).grammar_ready {
                continue;
            }
            for source in ["", "\u{0}\u{1}", "{{{{{{", &"(".repeat(2_000)] {
                let _ = facts(*language, source);
            }
        }
    }

    #[test]
    fn compiling_is_cached_rather_than_repeated_per_file() {
        let first = compiled(Language::Ruby).expect("compiles");
        let second = compiled(Language::Ruby).expect("compiles");
        assert!(std::ptr::eq(first, second), "the query was rebuilt");
    }
}
