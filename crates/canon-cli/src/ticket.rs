//! The ticket key the branch already carries.
//!
//! Every documented way to bind a session to a ticket takes the key from the
//! human: `/fix-issue 1234`, `$ARGUMENTS`, a prompt naming the issue. That is
//! a mechanism nobody remembers to use. Measured on 684 pull requests created
//! on or after 2026-01-01 in the corpus this was designed against, 683 carry
//! the key in the source branch name, so the string is already there.
//!
//! That is a claim about branches that produced a pull request, not about
//! sessions. A session started before the branch was cut, a detached HEAD and
//! a branch named without a key all produce no key, and no key produces no
//! digest, which is the right failure.
//!
//! # Why this is not a regular expression
//!
//! canon links four dependency trees and a regex engine is not one of them.
//! The pattern a ticket key needs is a handful of character classes in a row,
//! so it is matched by hand here, in the same spirit as `args.rs` being
//! eighty lines rather than a parser dependency.

// `commands::check` reaches `KeyPattern::compile` and nothing past it. Everything
// else here waits on Task 10, where `session_start` calls `context::block` and the
// chain from `main` down to `key_for` exists for the first time. Until then each
// unreached item is `dead_code`, which `RUSTFLAGS: -D warnings`
// (`.github/workflows/ci.yml:10`) turns into a build error: key_for, first_match,
// match_at, Class::accepts and the terms field. The allow is scoped to this file
// and Task 10 Step 3 deletes it in the commit that wires the chain up. It also
// covers `git::branch_name` and `git::branch_subjects`, which `key_for` is the
// only caller of: an allowed item is a live root, so what it calls is live too.
#![allow(dead_code)]

use std::path::Path;

use crate::git;

/// One character class in a supported pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// `[A-Z]`
    Upper,
    /// `[A-Z0-9]`
    UpperDigit,
    /// `[0-9]` or `\d`
    Digit,
    /// Any other character, matched as itself.
    Literal(char),
}

impl Class {
    fn accepts(self, c: char) -> bool {
        match self {
            Self::Upper => c.is_ascii_uppercase(),
            Self::UpperDigit => c.is_ascii_uppercase() || c.is_ascii_digit(),
            Self::Digit => c.is_ascii_digit(),
            Self::Literal(want) => c == want,
        }
    }
}

/// A class, and whether it repeats.
#[derive(Debug, Clone, Copy)]
struct Term {
    class: Class,
    repeats: bool,
}

/// A compiled `key_pattern`.
pub(crate) struct KeyPattern {
    terms: Vec<Term>,
}

impl KeyPattern {
    /// Compile the supported subset, or refuse.
    ///
    /// Supported: `[A-Z]`, `[A-Z0-9]`, `[0-9]`, `\d`, and any other character
    /// as a literal, each optionally followed by `+`. Everything else is
    /// refused rather than read as a literal, because a pattern that silently
    /// means something other than what it looks like produces no key and no
    /// explanation.
    pub(crate) fn compile(pattern: &str) -> Option<Self> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut terms: Vec<Term> = Vec::new();
        let mut i = 0;
        while let Some(&c) = chars.get(i) {
            let class = match c {
                '[' => {
                    let close = i + chars.iter().skip(i).position(|&c| c == ']')?;
                    let body: String = chars.get(i + 1..close)?.iter().collect();
                    i = close;
                    match body.as_str() {
                        "A-Z" => Class::Upper,
                        "A-Z0-9" => Class::UpperDigit,
                        "0-9" => Class::Digit,
                        _ => return None,
                    }
                }
                '\\' => {
                    let next = *chars.get(i + 1)?;
                    i += 1;
                    match next {
                        'd' => Class::Digit,
                        other => Class::Literal(other),
                    }
                }
                '*' | '?' | '(' | ')' | '{' | '}' | '|' | '^' | '$' | '.' => return None,
                other => Class::Literal(other),
            };
            let repeats = chars.get(i + 1) == Some(&'+');
            if repeats {
                i += 1;
            }
            terms.push(Term { class, repeats });
            i += 1;
        }
        (!terms.is_empty()).then_some(Self { terms })
    }

    /// The first run in `text` the pattern matches.
    pub(crate) fn first_match(&self, text: &str) -> Option<String> {
        let chars: Vec<char> = text.chars().collect();
        for start in 0..chars.len() {
            if let Some(end) = self.match_at(&chars, start) {
                return Some(chars.get(start..end)?.iter().collect());
            }
        }
        None
    }

    /// Greedy, and it does not backtrack.
    ///
    /// Enough for a key, where each term's class differs from the next one's
    /// at the boundary: `[A-Z0-9]+` stops at the `-` that follows it. A
    /// pattern whose neighbouring classes overlap, `[A-Z0-9]+\d+`, cannot be
    /// matched by this and is not a shape anyone writes for a ticket key.
    fn match_at(&self, chars: &[char], start: usize) -> Option<usize> {
        let mut at = start;
        for term in &self.terms {
            if !chars.get(at).copied().is_some_and(|c| term.class.accepts(c)) {
                return None;
            }
            at += 1;
            if term.repeats {
                while chars.get(at).copied().is_some_and(|c| term.class.accepts(c)) {
                    at += 1;
                }
            }
        }
        Some(at)
    }
}

/// The key this branch carries, in the order the design gives.
///
/// 1. The branch name. 683 of 684 measured pull requests carried it there.
/// 2. The subjects of the commits on this branch that are not on its merge
///    base. Local, no network, and the only fallback that exists before a
///    pull request does.
/// 3. Nothing, and the gather stays silent.
pub(crate) fn key_for(root: &Path, pattern: &KeyPattern) -> Option<String> {
    if let Some(branch) = git::branch_name(root)
        && let Some(key) = pattern.first_match(&branch)
    {
        return Some(key);
    }
    git::branch_subjects(root)?.iter().find_map(|s| pattern.first_match(s))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn default_pattern() -> KeyPattern {
        KeyPattern::compile(&canon_core::ContextSettings::default().key_pattern)
            .expect("the shipped default must compile")
    }

    #[test]
    fn the_default_pattern_finds_the_key_in_a_real_branch_name() {
        // Branch names from the measured corpus. They are not sensitive, so
        // they ship as a fixture rather than as a description of one.
        let p = default_pattern();
        assert_eq!(
            p.first_match("EF-7424-unauthenticated-book-call-lets").as_deref(),
            Some("EF-7424")
        );
        assert_eq!(p.first_match("EF-6190-assign-listings").as_deref(), Some("EF-6190"));
        assert_eq!(p.first_match("feature/EF-7059").as_deref(), Some("EF-7059"));
        assert_eq!(p.first_match("EF-7349").as_deref(), Some("EF-7349"));
    }

    #[test]
    fn a_branch_with_no_key_yields_nothing_rather_than_a_guess() {
        // 1 of the 684: PR 4099, branch `crass-bundle-audit-fix`. Its key is
        // in the pull request title, and at session start there is no pull
        // request yet, which is why the fallback reads commit subjects.
        let p = default_pattern();
        assert_eq!(p.first_match("crass-bundle-audit-fix"), None);
        assert_eq!(p.first_match("production"), None);
        assert_eq!(p.first_match("main"), None);
        // The one title in the corpus with a space inside the key. A title is
        // not read at session start, and the default pattern does not match
        // it; both facts are deliberate.
        assert_eq!(p.first_match("EF- 7118 Prevent admin listing saves"), None);
    }

    #[test]
    fn the_key_is_the_whole_key_and_stops_where_the_digits_do() {
        let p = default_pattern();
        assert_eq!(p.first_match("ABCD1-234-and-more").as_deref(), Some("ABCD1-234"));
        assert_eq!(p.first_match("xxEF-7424").as_deref(), Some("EF-7424"));
    }

    #[test]
    fn a_pattern_outside_the_supported_subset_refuses_to_compile() {
        // Silence beats a wrong key. Someone who writes `.*` meant a regular
        // expression and would otherwise get a literal dot and a mismatch
        // they could not see.
        for unsupported in ["[a-z]+-\\d+", "(EF|OPS)-\\d+", "EF-\\d{4}", "EF-.*", ""] {
            assert!(KeyPattern::compile(unsupported).is_none(), "{unsupported} compiled");
        }
    }

    #[test]
    fn a_literal_project_prefix_is_a_supported_pattern() {
        let p = KeyPattern::compile("EF-\\d+").expect("compiles");
        assert_eq!(p.first_match("feature/EF-7059-x").as_deref(), Some("EF-7059"));
        assert_eq!(p.first_match("OPS-7059"), None);
    }

    #[test]
    fn a_directory_that_is_not_a_repository_produces_no_key() {
        let dir = std::env::temp_dir().join("canon-ticket-nonrepo");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(key_for(&dir, &default_pattern()), None);
    }

    #[test]
    fn the_corpus_of_real_branch_names_still_measures_683_of_684() {
        // The single number the whole tier is sized against. Branch names are
        // not sensitive, so the corpus ships as a fixture and this is its
        // regression test.
        //
        // A missing fixture fails here rather than returning. Returning made
        // this the one test in the file that stayed green under a matcher
        // pinned to offset 0, which its three siblings caught, so it was
        // guarding the number in name only.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/branch-names.txt");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "{} is missing ({e}). Task 4 Step 4 writes it from the pull-request dump; until it exists the 683-of-684 number this tier is sized against is unmeasured, and this test is the only thing that would say so.",
                path.display()
            )
        });
        let names: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        assert_eq!(names.len(), 684, "the fixture is the measured set, whole");
        let p = default_pattern();
        let missed: Vec<&&str> =
            names.iter().filter(|name| p.first_match(name).is_none()).collect();
        assert_eq!(missed, vec![&"crass-bundle-audit-fix"], "the miss is the one known miss");
    }
}
