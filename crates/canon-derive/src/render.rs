//! Turning selected conventions into the text the model reads.
//!
//! The shape of this block is load-bearing. It has to read as a statement of
//! fact about this repository, not as an instruction, because the user's own
//! prompt is the instruction and a block that competes with it will sometimes
//! win. "Types here expose one public method (47/52)" invites the model to
//! follow the house style; "You must expose one public method" invites it to
//! override what the user actually asked for.
//!
//! The counts do that work. They tell the model this is evidence rather than
//! policy, which is exactly what it is.

use canon_core::Convention;

/// Render the block for one target path, or `None` when there is nothing
/// worth saying.
///
/// Silence is a real outcome and the common one in an unfamiliar corner of a
/// repository. A block that says "no conventions found" spends the budget
/// teaching the model that canon has nothing useful to offer.
#[must_use]
pub fn render_block(rel: &str, selected: &[&Convention]) -> Option<String> {
    if selected.is_empty() {
        return None;
    }

    // The header names the scope and nothing else. It previously carried a
    // file count taken from the widest selected rule, which on a real
    // repository read "derived from 2399 files" above rules measured over
    // 1550. Each line already states its own evidence; a second, larger number
    // in the header can only mislead.
    let scope = selected.first().map_or_else(|| rel.to_string(), |c| c.scope.render());

    let mut out = String::new();
    out.push_str(&format!("Conventions for {scope}, derived from this repository:\n\n"));
    for convention in selected {
        out.push_str(&convention.render_line());
        out.push('\n');
    }

    // One example, from the most specific rule that has one. More than one
    // reads as a reading list rather than a pointer.
    if let Some(exemplar) = selected.iter().find_map(|c| c.exemplar.as_deref()) {
        if exemplar != rel {
            out.push_str(&format!("\nCanonical example, most recently modified: {exemplar}\n"));
        }
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use canon_core::{Confidence, Enforcement, Scope};

    fn conv(statement: &str, exemplar: Option<&str>) -> Convention {
        Convention {
            id: "shape.public-arity.app.services.rb".into(),
            statement: statement.into(),
            scope: Scope::DirExt("app/services".into(), "rb".into()),
            confidence: Confidence::derive(47, 52).expect("valid"),
            agreeing: 47,
            total: 52,
            exemplar: exemplar.map(String::from),
            evidence: vec![],
            enforcement: Enforcement::Advisory,
        }
    }

    #[test]
    fn nothing_to_say_renders_as_none_not_as_an_empty_block() {
        assert!(render_block("app/services/a.rb", &[]).is_none());
    }

    #[test]
    fn the_block_names_the_scope_the_counts_and_the_example() {
        let a = conv("Types here expose exactly 1 public method", Some("app/services/update.rb"));
        let block = render_block("app/services/create.rb", &[&a]).expect("a block");
        assert!(
            block.contains("Conventions for app/services/**/*.rb, derived from this repository:")
        );
        assert!(block.contains("- Types here expose exactly 1 public method. (47/52, 0.90)"));
        assert!(
            block.contains("Canonical example, most recently modified: app/services/update.rb")
        );
    }

    #[test]
    fn the_file_being_written_is_never_offered_as_its_own_example() {
        let a = conv("Types here expose exactly 1 public method", Some("app/services/create.rb"));
        let block = render_block("app/services/create.rb", &[&a]).expect("a block");
        assert!(!block.contains("Canonical example"), "got {block}");
    }

    #[test]
    fn a_rule_with_no_example_still_renders() {
        let a = conv("Files here are named in snake_case", None);
        let block = render_block("app/services/create.rb", &[&a]).expect("a block");
        assert!(block.contains("snake_case"));
        assert!(!block.contains("Canonical example"));
    }

    #[test]
    fn every_rule_appears_on_its_own_line() {
        let a = conv("Rule one", None);
        let b = conv("Rule two", None);
        let block = render_block("app/a.rb", &[&a, &b]).expect("a block");
        let bullets = block.lines().filter(|l| l.starts_with("- ")).count();
        assert_eq!(bullets, 2);
    }

    #[test]
    fn the_block_states_evidence_rather_than_issuing_an_order() {
        // If this ever reads as policy it will start overriding the user.
        let a = conv("Types here expose exactly 1 public method", None);
        let block = render_block("app/a.rb", &[&a]).expect("a block");
        for imperative in ["You must", "Always ", "Never ", "Do not", "Ensure "] {
            assert!(!block.contains(imperative), "block turned into an instruction: {block}");
        }
        assert!(block.contains("(47/52"), "the counts are what make it evidence");
    }
}
