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

/// Whether `name` could have been written in `style`.
///
/// Compatibility, not classification. One name can be compatible with several
/// styles and that is the normal case for short names.
#[must_use]
pub(crate) fn is_compatible(name: &str, style: Style) -> bool {
    if name.is_empty() {
        return false;
    }
    let alnum_ok = |sep: char| {
        name.chars().all(|c| c.is_ascii_alphanumeric() || c == sep)
            && !name.starts_with(sep)
            && !name.ends_with(sep)
            && !name.contains([sep, sep].iter().collect::<String>().as_str())
    };
    let first = name.chars().next().unwrap_or('_');
    match style {
        Style::Snake => alnum_ok('_') && !name.chars().any(|c| c.is_ascii_uppercase()),
        Style::Kebab => alnum_ok('-') && !name.chars().any(|c| c.is_ascii_uppercase()),
        Style::Camel => {
            first.is_ascii_lowercase() && name.chars().all(|c| c.is_ascii_alphanumeric())
        }
        Style::Pascal => {
            first.is_ascii_uppercase()
                && name.chars().all(|c| c.is_ascii_alphanumeric())
                && name.chars().any(|c| c.is_ascii_lowercase())
        }
        Style::ScreamingSnake => alnum_ok('_') && !name.chars().any(|c| c.is_ascii_lowercase()),
    }
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
    first.is_ascii_uppercase() || chars.any(|c| c.is_ascii_uppercase())
}

/// The styles every name in `names` is compatible with, but only when the
/// sample contains at least one name that distinguishes between styles.
///
/// Returns an empty vector when the sample is all single lowercase words,
/// because there is genuinely nothing to say.
#[must_use]
pub(crate) fn shared_styles(names: &[String]) -> Vec<Style> {
    if !names.iter().any(|n| is_discriminating(n)) {
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
        assert!(shared_styles(&owned(&["create", "update", "delete"])).is_empty());
    }

    #[test]
    fn one_discriminating_name_is_enough_to_pin_the_style() {
        let got = shared_styles(&owned(&["create", "update", "cancel_enrolment"]));
        assert_eq!(got, vec![Style::Snake]);
    }

    #[test]
    fn a_mixed_sample_agrees_on_nothing() {
        assert!(shared_styles(&owned(&["create_a", "createB"])).is_empty());
    }

    #[test]
    fn pascal_and_camel_are_distinguished_by_the_first_letter_only() {
        assert!(is_compatible("CreateEnrolment", Style::Pascal));
        assert!(!is_compatible("CreateEnrolment", Style::Camel));
        assert!(is_compatible("createEnrolment", Style::Camel));
        assert!(!is_compatible("createEnrolment", Style::Pascal));
    }

    #[test]
    fn an_all_caps_acronym_is_not_pascal_case() {
        // `HTTP` has no lowercase, so it is screaming snake, not Pascal.
        assert!(!is_compatible("HTTP", Style::Pascal));
        assert!(is_compatible("HTTP", Style::ScreamingSnake));
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
