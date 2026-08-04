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
    // Everything beyond the scope itself lives in one predicate, which
    // `canon explain` asks too. How tests are named is only about the file
    // being written when that file is a test, and a namespace rule speaks for
    // one directory rather than the subtree its scope reaches.
    let mut matching: Vec<&Convention> = conventions
        .iter()
        .filter(|c| c.scope.matches(rel))
        .filter(|c| crate::offered_for_path(c, rel))
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
            sample_roots: vec![],
            enforcement: Enforcement::Advisory,
        }
    }

    #[test]
    fn nothing_the_shared_predicate_withholds_survives_selection() {
        // `canon explain` answers with `offered_for_path` and this answers with
        // `for_path`. The two agreed by both remembering the same list of
        // filters, which is an invariant on the honour system: the next filter
        // added here and not there makes the audit page list a rule the
        // injected block withheld. Asserting the containment is what makes the
        // drift a test failure rather than a silent divergence.
        let rules = [
            conv(
                "tests.suffix.rb",
                "Test files are named `*_spec.rb`",
                Scope::Ext("rb".into()),
                9,
                10,
            ),
            conv(
                "shape.namespace.src.Services.php",
                "Files here declare namespace `App\\Services`",
                Scope::DirExt("src/Services".into(), "php".into()),
                9,
                10,
            ),
            conv(
                "naming.src.rb",
                "Files here are named in snake_case",
                Scope::DirExt("src".into(), "rb".into()),
                9,
                10,
            ),
        ];
        for rel in [
            "src/Services/charge_card.rb",
            "src/Services/Billing/Charge.php",
            "src/Services/Charge.php",
            "spec/services/charge_card_spec.rb",
        ] {
            for chosen in for_path(&rules, rel, 4000) {
                assert!(
                    crate::offered_for_path(chosen, rel),
                    "`{}` reached {rel} through selection but the shared predicate withholds it",
                    chosen.id
                );
            }
        }
    }

    #[test]
    fn a_namespace_rule_is_not_offered_to_a_subdirectory_it_does_not_name() {
        // PSR-4 makes a subdirectory's namespace differ from its parent's, so
        // offering both tells the model two different namespaces for one file.
        // The check half already refuses to judge on the parent's answer; the
        // injected half has to agree, or the advice contradicts the report.
        let parent = conv(
            "shape.namespace.src.Services.Billing.php",
            "Files here declare namespace `App\\Services\\Billing`",
            Scope::DirExt("src/Services/Billing".into(), "php".into()),
            6,
            6,
        );
        let own = conv(
            "shape.namespace.src.Services.Billing.Invoices.php",
            "Files here declare namespace `App\\Services\\Billing\\Invoices`",
            Scope::DirExt("src/Services/Billing/Invoices".into(), "php".into()),
            6,
            6,
        );
        let rules = [parent, own];
        let chosen = for_path(&rules, "src/Services/Billing/Invoices/Void.php", 4000);
        let namespaces: Vec<&str> = chosen
            .iter()
            .filter(|c| c.id.starts_with("shape.namespace"))
            .map(|c| c.statement.as_str())
            .collect();
        assert_eq!(namespaces.len(), 1, "two namespaces offered for one file: {namespaces:?}");
        assert!(
            namespaces[0].ends_with("`App\\Services\\Billing\\Invoices`"),
            "got {namespaces:?}"
        );
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
