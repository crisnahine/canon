//! Naming style, and the reason a single-word name proves nothing.
//!
//! `create` is valid `snake_case`, valid `kebab-case` and valid `camelCase` at
//! the same time. A directory of single-word file names is compatible with
//! every style and therefore evidence for none, so classification returns the
//! set a name is compatible with, and a convention is only derived when at
//! least one multi-word name in the sample actually distinguishes between
//! them.
//!
//! Skipping that check produces the most annoying possible failure: a
//! confident, unfalsifiable rule that was never observed.

use serde::{Deserialize, Serialize};

/// A naming style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Style {
    /// `create_enrolment`
    Snake,
    /// `create-enrolment`
    Kebab,
    /// `createEnrolment`
    Camel,
    /// `CreateEnrolment`
    Pascal,
    /// `CREATE_ENROLMENT`
    ScreamingSnake,
}

impl Style {
    /// Every style, for iteration.
    pub const ALL: &'static [Self] =
        &[Self::Snake, Self::Kebab, Self::Camel, Self::Pascal, Self::ScreamingSnake];

    /// The name as written in documentation and in an injected block.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Snake => "snake_case",
            Self::Kebab => "kebab-case",
            Self::Camel => "camelCase",
            Self::Pascal => "PascalCase",
            Self::ScreamingSnake => "SCREAMING_SNAKE_CASE",
        }
    }
}

/// The part of a file name a style rule is about.
///
/// Everything before the first dot, not the last. `charge_card.html.erb`,
/// `globals.d.ts`, `Button.module.css` and `payments.service.ts` are all named
/// after the thing before the qualifiers, and the qualifiers are not part of
/// anybody's naming convention.
///
/// Reading up to the last dot instead made every one of those compatible with
/// no style at all, because a `.` is not a word character. One `globals.d.ts`
/// in a directory of six `snake_case` files took that directory from four
/// conventions to none, and every Rails view is a `*.html.erb`.
#[must_use]
pub(crate) fn name_root(stem: &str) -> &str {
    stem.split_once('.').map_or(stem, |(root, _)| root)
}

/// Whether `name` could have been written in `style`.
///
/// Compatibility, not classification. One name can be compatible with several
/// styles and that is the normal case for short names.
///
/// Unicode-aware rather than ASCII-only. `café_service` is `snake_case` by any
/// reading, and treating it as compatible with nothing meant a blocking naming
/// rule refused it in a repository whose other file names happened to be
/// ASCII. Scripts with no case distinction classify as lowercase, which is the
/// right answer: they cannot witness a style and they cannot break one.
#[must_use]
pub(crate) fn is_compatible(name: &str, style: Style) -> bool {
    if name.is_empty() {
        return false;
    }
    let word_ok = |sep: char| {
        name.chars().all(|c| c.is_alphanumeric() || c == sep)
            && !name.starts_with(sep)
            && !name.ends_with(sep)
            && !name.contains([sep, sep].iter().collect::<String>().as_str())
    };
    let alnum = || name.chars().all(char::is_alphanumeric);
    let first = name.chars().next().unwrap_or('_');
    match style {
        Style::Snake => word_ok('_') && !name.chars().any(char::is_uppercase),
        Style::Kebab => word_ok('-') && !name.chars().any(char::is_uppercase),
        Style::Camel => first.is_lowercase() && alnum(),
        // A separator-free all-caps name is how a `PascalCase` project writes
        // an acronym — `SEO.tsx`, `FAQ.tsx`, `API.ts` — and requiring a
        // lowercase letter refused every one of them in a directory of
        // `PascalCase` components. It is compatible with `SCREAMING_SNAKE_CASE`
        // too; compatibility was never exclusive, and a sample that contains
        // only such names still pins no style and derives nothing.
        Style::Pascal => first.is_uppercase() && alnum(),
        Style::ScreamingSnake => word_ok('_') && !name.chars().any(char::is_lowercase),
    }
}

/// Whether any style at all could have produced this name.
///
/// A name that no style accepts is not a badly styled name, it is a name from
/// outside the style system: a framework token the author could not have
/// chosen differently. `[id].tsx` and `[...slug].tsx` are Next.js and Nuxt
/// dynamic routes, `+page.server.ts` and `+layout.ts` are `SvelteKit` routes,
/// `_form.html.erb` is a Rails partial, `_variables.scss` is a Sass partial.
/// None contains only word characters, so `is_compatible` is false for every
/// style, and a check that reads "compatible with nothing" as "breaks the
/// rule" refuses a file the developer cannot rename.
///
/// This is the general form of a guard that first shipped for a leading
/// underscore alone. Fixing the instance rather than the class left every
/// file-based router in the same trap.
#[must_use]
pub(crate) fn outside_the_style_system(name: &str) -> bool {
    !Style::ALL.iter().any(|s| is_compatible(name, *s))
}

/// Whether `name` is an acronym written the only way an acronym is written.
///
/// `FAQ`, `API`, `SEO`, `HTTP`. No separator to read a style at, and no
/// lowercase letter to read one from, so every project spells it identically
/// whatever style it otherwise holds. It is compatible with `PascalCase` and
/// `SCREAMING_SNAKE_CASE` and with nothing else, which means a `kebab-case` or
/// `camelCase` directory reads it as a violation of a choice its author never
/// made.
#[must_use]
pub(crate) fn is_bare_acronym(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(char::is_alphanumeric)
        && name.chars().any(char::is_uppercase)
        && !name.chars().any(char::is_lowercase)
}

/// Whether `name` has enough structure to tell two styles apart.
///
/// A name is discriminating when it contains a separator or an internal case
/// change. `create` is not; `create_enrolment`, `createEnrolment` and
/// `CreateEnrolment` all are.
#[must_use]
pub(crate) fn is_discriminating(name: &str) -> bool {
    if name.contains('_') || name.contains('-') {
        return true;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else { return false };
    first.is_uppercase() || chars.any(char::is_uppercase)
}

/// The styles every name in `names` is compatible with, but only when the
/// sample is large enough and varied enough to have witnessed one.
///
/// Two gates, and both were needed against real repositories.
///
/// At least one name has to *distinguish* between styles, or the sample is all
/// single lowercase words and is evidence for three styles at once.
///
/// And the sample has to contain `min_distinct` different names. Ten files all
/// called `Cargo.toml` are one observation, not ten, and the rule they produced
/// — "files here are named in `PascalCase`", at total agreement, enforced —
/// refused a perfectly ordinary `deny.toml`. Counting files rather than names
/// is what let one repeated name look unanimous.
///
/// Returns an empty vector when either gate fails, because there is genuinely
/// nothing to say.
#[must_use]
pub(crate) fn shared_styles(names: &[String], min_distinct: usize) -> Vec<Style> {
    if !names.iter().any(|n| is_discriminating(n)) {
        return Vec::new();
    }
    let distinct: std::collections::HashSet<&str> = names.iter().map(String::as_str).collect();
    if distinct.len() < min_distinct {
        return Vec::new();
    }
    Style::ALL.iter().copied().filter(|s| names.iter().all(|n| is_compatible(n, *s))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_multi_word_name_is_compatible_with_exactly_one_style() {
        let cases = [
            ("create_enrolment", Style::Snake),
            ("create-enrolment", Style::Kebab),
            ("createEnrolment", Style::Camel),
            ("CreateEnrolment", Style::Pascal),
            ("CREATE_ENROLMENT", Style::ScreamingSnake),
        ];
        for (name, expected) in cases {
            let got: Vec<Style> =
                Style::ALL.iter().copied().filter(|s| is_compatible(name, *s)).collect();
            assert_eq!(got, vec![expected], "{name} matched {got:?}");
        }
    }

    #[test]
    fn a_single_lowercase_word_is_compatible_with_three_styles_at_once() {
        // The whole reason `shared_styles` needs a discriminating witness.
        let got: Vec<Style> =
            Style::ALL.iter().copied().filter(|s| is_compatible("create", *s)).collect();
        assert_eq!(got, vec![Style::Snake, Style::Kebab, Style::Camel]);
    }

    #[test]
    fn a_sample_of_single_words_yields_no_style_convention() {
        assert!(shared_styles(&owned(&["create", "update", "delete"]), 3).is_empty());
    }

    #[test]
    fn one_discriminating_name_is_enough_to_pin_the_style() {
        let got = shared_styles(&owned(&["create", "update", "cancel_enrolment"]), 3);
        assert_eq!(got, vec![Style::Snake]);
    }

    #[test]
    fn a_mixed_sample_agrees_on_nothing() {
        assert!(shared_styles(&owned(&["create_a", "createB"]), 2).is_empty());
    }

    #[test]
    fn one_name_repeated_is_one_observation_not_many() {
        // Ten `Cargo.toml` files derived an enforced PascalCase rule that
        // refused `deny.toml`. Distinct names, not file count, is the sample.
        let repeated = owned(&["Cargo", "Cargo", "Cargo", "Cargo", "Cargo"]);
        assert!(shared_styles(&repeated, 5).is_empty());
        assert_eq!(shared_styles(&repeated, 1), vec![Style::Pascal]);
    }

    #[test]
    fn a_name_root_stops_at_the_first_dot() {
        assert_eq!(name_root("charge_card.html"), "charge_card");
        assert_eq!(name_root("globals.d"), "globals");
        assert_eq!(name_root("payments.service"), "payments");
        assert_eq!(name_root("charge_card"), "charge_card");
        assert_eq!(name_root(""), "");
    }

    #[test]
    fn an_accented_name_is_snake_case_like_any_other() {
        // Blocking on naming refused `café_service.py` in a repository whose
        // other names happened to be ASCII.
        assert!(is_compatible("café_service", Style::Snake));
        assert!(is_compatible("héllo_wörld", Style::Snake));
        assert!(!is_compatible("Café_Service", Style::Snake));
        assert_eq!(
            shared_styles(&owned(&["café_service", "refund_payment"]), 2),
            vec![Style::Snake]
        );
    }

    #[test]
    fn a_script_without_case_cannot_witness_a_style_or_break_one() {
        assert!(is_compatible("請求書", Style::Snake));
        assert!(!is_discriminating("請求書"));
    }

    #[test]
    fn pascal_and_camel_are_distinguished_by_the_first_letter_only() {
        assert!(is_compatible("CreateEnrolment", Style::Pascal));
        assert!(!is_compatible("CreateEnrolment", Style::Camel));
        assert!(is_compatible("createEnrolment", Style::Camel));
        assert!(!is_compatible("createEnrolment", Style::Pascal));
    }

    #[test]
    fn a_separator_free_acronym_is_compatible_with_both_readings() {
        // `SEO.tsx` and `FAQ.tsx` are how a PascalCase project writes an
        // acronym, and demanding a lowercase letter refused every one of them.
        // It reads as `SCREAMING_SNAKE_CASE` too, and compatibility was never
        // exclusive — a sample of nothing but acronyms pins neither style and
        // so derives nothing, which is the right answer.
        assert!(is_compatible("HTTP", Style::Pascal));
        assert!(is_compatible("HTTP", Style::ScreamingSnake));
        assert_eq!(shared_styles(&owned(&["HTTP", "SEO", "FAQ"]), 3).len(), 2, "pins neither");
        // A separator settles it: only screaming snake admits one.
        assert!(!is_compatible("HTTP_CLIENT", Style::Pascal));
        assert_eq!(shared_styles(&owned(&["HTTP_CLIENT", "SEO"]), 2), vec![Style::ScreamingSnake]);
        // And a real Pascal sample still admits the acronym beside it.
        assert_eq!(shared_styles(&owned(&["UserCard", "SEO"]), 2), vec![Style::Pascal]);
    }

    #[test]
    fn a_framework_named_file_is_outside_the_style_system() {
        // Every file-based router names files the author cannot rename.
        for token in ["[id]", "[...slug]", "+page", "+layout", "_form", "$types"] {
            assert!(outside_the_style_system(token), "{token} should be unclassifiable");
        }
        for real in ["user_card", "UserCard", "userCard", "user-card", "SEO"] {
            assert!(!outside_the_style_system(real), "{real} is a style choice");
        }
    }

    #[test]
    fn malformed_separators_are_compatible_with_nothing() {
        for bad in ["_leading", "trailing_", "double__sep", "-lead", ""] {
            let got: Vec<Style> =
                Style::ALL.iter().copied().filter(|s| is_compatible(bad, *s)).collect();
            assert!(got.is_empty(), "{bad} matched {got:?}");
        }
    }

    #[test]
    fn a_name_with_digits_still_classifies() {
        assert!(is_compatible("create_v2", Style::Snake));
        assert!(is_compatible("createV2", Style::Camel));
    }

    #[test]
    fn discrimination_requires_a_separator_or_a_case_change() {
        assert!(!is_discriminating("create"));
        assert!(is_discriminating("create_a"));
        assert!(is_discriminating("createA"));
        assert!(is_discriminating("Create"));
        assert!(!is_discriminating(""));
    }
}
