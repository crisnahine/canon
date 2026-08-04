//! The type a file is *about*.
//!
//! One definition, used by both halves. Deriving asks which type a file
//! contributes to a convention; verifying asks which type a convention should
//! be checked against. When those two disagreed, a file that followed every
//! rule exactly was reported for breaking two of them:
//!
//! ```ruby
//! module Billing
//!   class ApplyVariance < ApplicationService
//!     def call; end
//!   end
//! end
//! ```
//!
//! Deriving resolved `ApplyVariance`. Verifying looped every declared type, so
//! it also judged the namespace `Billing`, which has no base type and no public
//! methods, and reported both as violations. Once those conventions reached
//! total agreement the same file was refused outright.
//!
//! A tool that flags correct code teaches you to stop reading its output, so
//! the two questions now have one answer.

use canon_extract::{FileFacts, TypeFacts};

/// The type a file is about, if it declares one.
///
/// Ruby files routinely declare a small `class SomethingError < StandardError`
/// beside the real class, and TypeScript files declare prop interfaces beside
/// the component. A namespace module wraps the subject rather than being it.
/// Counting all of them lets the auxiliaries outvote the subject.
///
/// Resolution order: the type whose name matches the file name, then the one
/// with the largest surface.
#[must_use]
pub(crate) fn primary_type<'f>(facts: &'f FileFacts, stem: &str) -> Option<&'f TypeFacts> {
    // Both sides normalised. Comparing `to_snake(name)` against the stem as
    // written meant the match could never fire for a `PascalCase` or
    // `camelCase` file name — which is every JavaScript, TypeScript and PHP
    // repository — so every one of those files fell through to the surface
    // comparison and a companion class could take the subject.
    let wanted = to_snake(stem);
    if let Some(named) = facts.types.iter().find(|t| to_snake(&t.name) == wanted) {
        return Some(named);
    }
    // Public surface first, and the earliest declaration on a tie. A companion
    // error type gets a `private_method` from its `Display` impl, which tied it
    // with a one-public-method subject, and `max_by_key` returns the *last*
    // maximum — so which class the file was judged as depended on which the
    // author wrote second.
    facts
        .types
        .iter()
        .enumerate()
        .max_by_key(|(index, t)| {
            (
                t.public_methods.len(),
                t.public_methods.len() + t.private_methods.len(),
                usize::MAX - index,
            )
        })
        .map(|(_, t)| t)
}

/// `CreateEnrolment` and `Enrolments::Create` both reduce toward a file stem.
#[must_use]
pub(crate) fn to_snake(name: &str) -> String {
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

/// The file name without directory or extension, for matching against a type.
#[must_use]
pub(crate) fn stem_of(rel: &str) -> &str {
    let name = rel.rsplit_once('/').map_or(rel, |(_, n)| n);
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ty(name: &str, public: &[&str], base: Option<&str>) -> TypeFacts {
        TypeFacts {
            name: name.into(),
            line: 1,
            public_methods: public.iter().map(|s| (*s).to_string()).collect(),
            private_methods: vec![],
            superclass: base.map(String::from),
            bases: vec![],
            interfaces: vec![],
            mixins: vec![],
        }
    }

    fn facts(types: Vec<TypeFacts>) -> FileFacts {
        FileFacts { types, ..FileFacts::default() }
    }

    #[test]
    fn a_namespace_module_is_not_the_subject() {
        // The bug this module exists for. `Billing` wraps the subject; it has
        // no base type and no public methods, and judging it reported a
        // correct file as breaking two conventions.
        let f = facts(vec![
            ty("Billing", &[], None),
            ty("ApplyVariance", &["call"], Some("ApplicationService")),
        ]);
        let subject = primary_type(&f, "apply_variance").expect("a subject");
        assert_eq!(subject.name, "ApplyVariance");
    }

    #[test]
    fn the_type_named_after_the_file_wins_over_a_larger_one() {
        let f = facts(vec![
            ty("Helper", &["a", "b", "c", "d"], None),
            ty("ApplyVariance", &["call"], Some("Base")),
        ]);
        assert_eq!(
            primary_type(&f, "apply_variance").map(|t| t.name.as_str()),
            Some("ApplyVariance")
        );
    }

    #[test]
    fn without_a_name_match_the_largest_surface_wins() {
        let f = facts(vec![ty("SmallError", &[], None), ty("Real", &["a", "b"], None)]);
        assert_eq!(primary_type(&f, "unrelated").map(|t| t.name.as_str()), Some("Real"));
    }

    #[test]
    fn a_file_declaring_no_type_has_no_subject() {
        assert!(primary_type(&facts(vec![]), "anything").is_none());
    }

    #[test]
    fn a_compound_name_reduces_to_its_last_segment() {
        assert_eq!(to_snake("Billing::ApplyVariance"), "apply_variance");
        assert_eq!(to_snake("CreateEnrolment"), "create_enrolment");
        assert_eq!(to_snake("HTTP"), "h_t_t_p");
    }

    #[test]
    fn a_stem_drops_directory_and_extension() {
        assert_eq!(stem_of("app/services/apply_variance.rb"), "apply_variance");
        assert_eq!(stem_of("apply_variance.rb"), "apply_variance");
        assert_eq!(stem_of("Rakefile"), "Rakefile");
    }
}
