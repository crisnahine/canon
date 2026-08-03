//! Choosing what to say about one file, inside a fixed budget.
//!
//! The budget is the whole problem. Every convention that matches is true, and
//! saying all of them turns the injected block into a wall of text that
//! competes with the user's own instruction. Four lines that describe the file
//! about to be written beat twenty that describe the repository.

use canon_core::Convention;

/// The conventions that apply to `rel`, best first, fitting `budget` bytes.
///
/// Ranked by specificity before confidence: a rule derived from the twelve
/// files beside this one describes it better than a rule derived from four
/// thousand across the repository, even when the broad rule is more certain.
#[must_use]
pub fn for_path<'a>(
    conventions: &'a [Convention],
    rel: &str,
    budget: usize,
) -> Vec<&'a Convention> {
    // How tests are named is only about the file being written when that file
    // is a test. Otherwise it is a true sentence about somewhere else, spending
    // budget and diluting the rules that do describe this path.
    let writing_a_test = crate::tier0::is_test_path(rel);
    let mut matching: Vec<&Convention> = conventions
        .iter()
        .filter(|c| c.scope.matches(rel))
        .filter(|c| writing_a_test || !c.id.starts_with("tests.suffix"))
        .collect();
    matching.sort_by(|a, b| b.rank().cmp(&a.rank()).then(a.id.cmp(&b.id)));

    let mut chosen: Vec<&Convention> = Vec::new();
    let mut used = 0usize;
    for candidate in matching {
        // The same rule is derived at several ancestor levels. Keep the most
        // specific, which sorting has already put first.
        if chosen.iter().any(|c| c.statement == candidate.statement) {
            continue;
        }
        let cost = candidate.render_line().len() + 1;
        if used + cost > budget {
            continue;
        }
        used += cost;
        chosen.push(candidate);
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use canon_core::{Confidence, Enforcement, Scope};

    fn conv(id: &str, statement: &str, scope: Scope, agreeing: usize, total: usize) -> Convention {
        Convention {
            id: id.into(),
            statement: statement.into(),
            scope,
            confidence: Confidence::derive(agreeing, total).expect("valid"),
            agreeing,
            total,
            exemplar: None,
            evidence: vec![],
            enforcement: Enforcement::Advisory,
        }
    }

    #[test]
    fn only_conventions_whose_scope_matches_are_returned() {
        let convs = vec![
            conv("a", "Ruby rule", Scope::DirExt("app".into(), "rb".into()), 10, 10),
            conv("b", "Frontend rule", Scope::DirExt("src".into(), "tsx".into()), 10, 10),
        ];
        let got = for_path(&convs, "app/services/create.rb", 1_500);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "a");
    }

    #[test]
    fn the_most_specific_rule_comes_first() {
        let convs = vec![
            conv("broad", "Broad", Scope::Ext("rb".into()), 100, 100),
            conv("deep", "Deep", Scope::DirExt("app/services".into(), "rb".into()), 8, 10),
        ];
        let got = for_path(&convs, "app/services/create.rb", 1_500);
        assert_eq!(got[0].id, "deep", "specificity must outrank confidence");
    }

    #[test]
    fn the_same_statement_derived_at_two_levels_appears_once() {
        let convs = vec![
            conv("shallow", "Same rule", Scope::DirExt("app".into(), "rb".into()), 20, 20),
            conv("deep", "Same rule", Scope::DirExt("app/services".into(), "rb".into()), 10, 10),
        ];
        let got = for_path(&convs, "app/services/create.rb", 1_500);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "deep", "the more specific derivation survives");
    }

    #[test]
    fn the_budget_is_respected() {
        let convs: Vec<Convention> = (0..50)
            .map(|i| {
                conv(
                    &format!("c{i}"),
                    &format!("Rule number {i} with a reasonably long statement"),
                    Scope::Ext("rb".into()),
                    10,
                    10,
                )
            })
            .collect();
        let got = for_path(&convs, "a.rb", 200);
        let rendered: usize = got.iter().map(|c| c.render_line().len() + 1).sum();
        assert!(rendered <= 200, "spent {rendered}");
        assert!(!got.is_empty(), "a small budget must still say something");
    }

    #[test]
    fn a_zero_budget_yields_nothing_rather_than_one_oversized_line() {
        let convs = vec![conv("a", "Rule", Scope::Repo, 10, 10)];
        assert!(for_path(&convs, "a.rb", 0).is_empty());
    }

    #[test]
    fn selection_is_deterministic_for_equally_ranked_rules() {
        let convs = vec![
            conv("z", "Z rule", Scope::Ext("rb".into()), 10, 10),
            conv("a", "A rule", Scope::Ext("rb".into()), 10, 10),
        ];
        let first: Vec<&str> =
            for_path(&convs, "x.rb", 1_500).iter().map(|c| c.id.as_str()).collect();
        let second: Vec<&str> =
            for_path(&convs, "x.rb", 1_500).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(first, second);
        assert_eq!(first, vec!["a", "z"], "ties break on id, not hash order");
    }

    #[test]
    fn an_empty_convention_set_yields_nothing() {
        assert!(for_path(&[], "a.rb", 1_500).is_empty());
    }
}
