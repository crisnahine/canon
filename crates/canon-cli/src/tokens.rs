//! One tokeniser, eleven languages.
//!
//! The sentence selector and the Slack query builder ask the same question:
//! does this sentence name something that exists in the code. The shapes
//! differ by language, and the difference is the whole reason this is one
//! function rather than eleven. `seller_pipeline_2025_hubspot_deal_id` is
//! Ruby and Python `snake_case`, `Workers::Google::BigQuery::SyncTableWorker`
//! is Ruby namespacing, `requiresBuyerVetting` is TypeScript `camelCase`,
//! `AIBC_PROPERTIES` is a Ruby constant, `listing.earnout_ticket` is dotted,
//! `App\Services\Listing` is PHP.
//!
//! One permissive tokeniser accepts all of them, so Ruby, JavaScript, JSX,
//! TypeScript, TSX, Python, Go, Rust, PHP, Vue SFC and ERB get identical
//! behaviour, and a twelfth language would get it with no work here at all.

// Nothing under `main` reaches this file until Task 10 calls `context::block` from
// `session_start`. `jira::distinctive_term` in Task 7 is the first caller written,
// and it is itself unreachable until then; the tests here do not count, because
// `cargo clippy --workspace --all-targets` builds the bin without `cfg(test)` as
// well. Under `RUSTFLAGS: -D warnings` (`.github/workflows/ci.yml:10`) that is a
// build error for identifiers, is_token_char, is_identifier, MIN_LENGTH and
// MIN_LETTERS: the two constants are read only by the Step 3 body, so this is also
// what keeps Step 2 free to fail on its assertion. Task 10 Step 3 deletes the allow
// in the commit that wires the chain up.
#![allow(dead_code)]

/// Shortest run that counts.
///
/// Eight characters, from the design. Shorter runs are `call`, `params`,
/// `id`, `status`: words that appear in every ticket, where a query built
/// from one returns the whole workspace and tells nobody anything.
const MIN_LENGTH: usize = 8;

/// Fewest letters a run needs before it is a name rather than a number.
///
/// `2026-08-05` is eight characters with a separator in the middle and is a
/// date. Requiring letters is what keeps a date out of a Slack query.
const MIN_LETTERS: usize = 3;

/// Every identifier-shaped run in `text`, in order, without repeats.
pub(crate) fn identifiers(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for raw in text.split(|c: char| !is_token_char(c)) {
        // A trailing full stop ends a sentence far more often than it joins
        // two parts of a name, and a leading one is never part of a name.
        let token = raw.trim_matches(|c| c == '.' || c == ':' || c == '-' || c == '\\');
        if is_identifier(token) && !found.iter().any(|f| f == token) {
            found.push(token.to_string());
        }
    }
    found
}

/// Characters a name may be spelled with, across all eleven languages.
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '.' | '-' | '\\')
}

/// Whether a run is a name rather than a long word or a number.
///
/// The shape signal is what does the work: an underscore, a separator
/// between two alphanumerics, or an internal lowercase-to-uppercase
/// transition. A word from a dictionary carries none of them.
fn is_identifier(token: &str) -> bool {
    if token.chars().count() < MIN_LENGTH {
        return false;
    }
    if token.chars().filter(char::is_ascii_alphabetic).count() < MIN_LETTERS {
        return false;
    }
    if token.contains('_') {
        return true;
    }
    let chars: Vec<char> = token.chars().collect();
    chars.windows(3).any(|w| match w {
        [before, separator, after] => {
            matches!(separator, ':' | '.' | '-' | '\\')
                && before.is_ascii_alphanumeric()
                && after.is_ascii_alphanumeric()
        }
        _ => false,
    }) || chars.windows(2).any(|w| match w {
        [before, after] => before.is_ascii_lowercase() && after.is_ascii_uppercase(),
        _ => false,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn one_identifier_from_every_wired_language_survives() {
        // Eleven languages, eleven shapes, one tokeniser. If a language is
        // ever added whose identifiers do not survive this, the tier stops
        // being language-agnostic and this test is where it shows.
        for (language, token) in [
            ("Ruby", "Workers::Google::BigQuery::SyncTableWorker"),
            ("JavaScript", "buildListingPayload"),
            ("JSX", "ListingBadgeRow"),
            ("TypeScript", "requiresBuyerVetting"),
            ("TSX", "useListingSummary"),
            ("Python", "seller_pipeline_2025_hubspot_deal_id"),
            ("Go", "ListingSummaryClient"),
            ("Rust", "canon_core::Convention"),
            ("PHP", "App\\Services\\Listing"),
            ("Vue SFC", "listing-badge-row"),
            ("ERB", "listing.earnout_ticket"),
        ] {
            let found = identifiers(&format!("the {token} bit is what changed"));
            assert!(
                found.iter().any(|f| f == token),
                "{language}: {token} was lost, got {found:?}"
            );
        }
    }

    #[test]
    fn the_six_real_shapes_from_the_corpus_survive() {
        let text = "Set `buyer_vetting_interested_listing` when \
                    seller_pipeline_2025_hubspot_deal_id is present. \
                    Workers::Google::BigQuery::SyncTableWorker reads AIBC_PROPERTIES \
                    and requiresBuyerVetting, then listing.earnout_ticket.";
        let found = identifiers(text);
        for want in [
            "buyer_vetting_interested_listing",
            "seller_pipeline_2025_hubspot_deal_id",
            "Workers::Google::BigQuery::SyncTableWorker",
            "AIBC_PROPERTIES",
            "requiresBuyerVetting",
            "listing.earnout_ticket",
        ] {
            assert!(found.iter().any(|f| f == want), "{want} was lost, got {found:?}");
        }
    }

    #[test]
    fn a_long_english_word_is_not_an_identifier() {
        // The filter that makes the signal worth anything. A query built from
        // `regardless` returns every message in the workspace.
        for word in ["regardless", "automated", "valuation", "assignment", "instructions"] {
            assert!(identifiers(word).is_empty(), "{word} was read as an identifier");
        }
    }

    #[test]
    fn a_date_and_a_short_name_are_not_identifiers() {
        assert!(identifiers("2026-08-05").is_empty(), "a date is not a name");
        assert!(identifiers("1000-2000").is_empty());
        assert!(identifiers("call").is_empty(), "shorter than the floor");
        assert!(identifiers("EF-7424").is_empty(), "a key is not an identifier");
    }

    #[test]
    fn a_repeat_is_reported_once_and_order_is_kept() {
        let found = identifiers(
            "requiresBuyerVetting and later requiresBuyerVetting again, \
                                 plus listing.earnout_ticket",
        );
        assert_eq!(found, vec!["requiresBuyerVetting", "listing.earnout_ticket"]);
    }

    #[test]
    fn punctuation_around_a_token_is_not_part_of_it() {
        assert_eq!(identifiers("(`requiresBuyerVetting`)."), vec!["requiresBuyerVetting"]);
        assert_eq!(identifiers("see listing.earnout_ticket."), vec!["listing.earnout_ticket"]);
    }
}
