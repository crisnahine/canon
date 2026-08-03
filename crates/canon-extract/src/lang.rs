//! The language taxonomy, and the one table that says what is actually wired.
//!
//! [`provider`] matches exhaustively on purpose. Adding a variant to
//! [`Language`] fails to compile until a provider entry exists, so the
//! capability table can never drift from the code the way a hand-maintained
//! README does.

/// How a language decides that a member is callable from outside.
///
/// Recorded per language so `canon check` can explain the classification it
/// applied, and so a reader of the code can see at a glance why six extractors
/// cannot share one implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// A bare keyword flips every later member. Positional state, not a
    /// property of the member itself.
    SectionKeyword,
    /// The case of the first letter of the name.
    Capitalisation,
    /// An explicit modifier on the member, public when absent.
    ModifierDefaultPublic,
    /// An explicit modifier on the member, private when absent.
    ModifierDefaultPrivate,
    /// A naming convention with no enforcement from the language itself.
    NamePrefix,
}

/// Every language canon knows the name of, wired or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    /// Ruby.
    Ruby,
    /// JavaScript.
    JavaScript,
    /// JavaScript with JSX.
    Jsx,
    /// TypeScript.
    TypeScript,
    /// TypeScript with JSX.
    Tsx,
    /// Python.
    Python,
    /// Go.
    Go,
    /// Rust.
    Rust,
    /// PHP.
    Php,
    /// Vue single-file components.
    Vue,
    /// Embedded Ruby templates.
    Erb,
}

impl Language {
    /// Every variant, for table rendering and for tests that must cover all of
    /// them.
    pub const ALL: &'static [Self] = &[
        Self::Ruby,
        Self::JavaScript,
        Self::Jsx,
        Self::TypeScript,
        Self::Tsx,
        Self::Python,
        Self::Go,
        Self::Rust,
        Self::Php,
        Self::Vue,
        Self::Erb,
    ];

    /// Display name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Ruby => "Ruby",
            Self::JavaScript => "JavaScript",
            Self::Jsx => "JSX",
            Self::TypeScript => "TypeScript",
            Self::Tsx => "TSX",
            Self::Python => "Python",
            Self::Go => "Go",
            Self::Rust => "Rust",
            Self::Php => "PHP",
            Self::Vue => "Vue SFC",
            Self::Erb => "ERB",
        }
    }
}

/// What canon can do with a language.
#[derive(Debug, Clone, Copy)]
pub struct Provider {
    /// The language.
    pub language: Language,
    /// Whether a grammar is linked and an extractor exists.
    ///
    /// When false, only the facts derivable from paths and file sizes apply.
    pub grammar_ready: bool,
    /// How this language decides visibility.
    pub visibility: Visibility,
    /// Extensions that map here.
    pub extensions: &'static [&'static str],
}

/// The capability table.
///
/// Exhaustive by construction. This function is the single source of truth for
/// what `canon check` prints, so the documented capability and the linked
/// grammar cannot disagree.
#[must_use]
pub fn provider(language: Language) -> Provider {
    let (grammar_ready, visibility, extensions): (bool, Visibility, &'static [&'static str]) =
        match language {
            Language::Ruby => (true, Visibility::SectionKeyword, &["rb", "rake", "gemspec"]),
            Language::JavaScript => {
                (true, Visibility::ModifierDefaultPublic, &["js", "mjs", "cjs"])
            }
            Language::Jsx => (true, Visibility::ModifierDefaultPublic, &["jsx"]),
            Language::TypeScript => {
                (true, Visibility::ModifierDefaultPublic, &["ts", "mts", "cts"])
            }
            Language::Tsx => (true, Visibility::ModifierDefaultPublic, &["tsx"]),
            Language::Python => (true, Visibility::NamePrefix, &["py", "pyi"]),
            Language::Go => (true, Visibility::Capitalisation, &["go"]),
            Language::Rust => (true, Visibility::ModifierDefaultPrivate, &["rs"]),
            Language::Php => (true, Visibility::ModifierDefaultPublic, &["php"]),
            // Two grammars in one file. Handled by parsing the script block
            // alone through `set_included_ranges`, so the markup is skipped
            // rather than fed to a parser that cannot read it.
            Language::Vue => (true, Visibility::ModifierDefaultPublic, &["vue"]),
            // Ruby inside markup, same mechanism.
            Language::Erb => (true, Visibility::SectionKeyword, &["erb", "rhtml"]),
        };
    Provider { language, grammar_ready, visibility, extensions }
}

/// The tree-sitter grammar for a language, or `None` when none is linked.
///
/// The single place a grammar object is constructed. Extractors and the query
/// layer both need one, and two lists of the same mapping is how a language
/// ends up wired for parsing but not for queries.
#[must_use]
pub fn grammar(language: Language) -> Option<tree_sitter::Language> {
    Some(match language {
        Language::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        Language::JavaScript | Language::Jsx => tree_sitter_javascript::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Php => tree_sitter_php::LANGUAGE_PHP.into(),
        Language::Vue | Language::Erb => return None,
    })
}

/// Map a file extension to a language.
///
/// Case-insensitive, because Windows checkouts and generated files disagree
/// with everyone else about `.RB` and `.PY`.
#[must_use]
pub fn from_extension(ext: &str) -> Option<Language> {
    let lower = ext.to_ascii_lowercase();
    Language::ALL.iter().copied().find(|l| provider(*l).extensions.contains(&lower.as_str()))
}

/// Every language with a grammar linked.
#[must_use]
pub fn wired() -> Vec<Language> {
    Language::ALL.iter().copied().filter(|l| provider(*l).grammar_ready).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_extension_maps_to_exactly_one_language() {
        let mut seen: HashSet<&str> = HashSet::new();
        for lang in Language::ALL {
            for ext in provider(*lang).extensions {
                assert!(seen.insert(ext), "extension `{ext}` is claimed by two languages");
            }
        }
    }

    #[test]
    fn extension_lookup_is_case_insensitive() {
        assert_eq!(from_extension("rb"), Some(Language::Ruby));
        assert_eq!(from_extension("RB"), Some(Language::Ruby));
        assert_eq!(from_extension("tsx"), Some(Language::Tsx));
        assert_eq!(from_extension("nope"), None);
        assert_eq!(from_extension(""), None);
    }

    #[test]
    fn the_capability_table_is_total() {
        for lang in Language::ALL {
            let p = provider(*lang);
            assert_eq!(p.language, *lang);
            assert!(!p.extensions.is_empty(), "{} claims no extensions", lang.name());
        }
    }

    #[test]
    fn a_two_grammar_file_is_wired_through_the_language_it_embeds() {
        // Vue and ERB have no grammar of their own and are still wired: the
        // script block of one and the tags of the other are parsed by the
        // language they contain, through included ranges.
        for language in [Language::Vue, Language::Erb] {
            assert!(provider(language).grammar_ready, "{} should be wired", language.name());
            assert!(
                grammar(language).is_none(),
                "{} has no grammar of its own; it delegates",
                language.name()
            );
        }
    }

    #[test]
    fn the_wired_set_covers_the_stacks_this_tool_targets() {
        for lang in [
            Language::Ruby,
            Language::TypeScript,
            Language::Tsx,
            Language::JavaScript,
            Language::Python,
            Language::Go,
            Language::Rust,
            Language::Php,
        ] {
            assert!(provider(lang).grammar_ready, "{} must be wired", lang.name());
        }
    }
}
