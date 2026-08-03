//! Tree-walking helpers shared by every extractor.

use tree_sitter::Node;

/// Children as an owned `Vec`.
///
/// `Node::children` borrows the cursor it is given, so an iterator derived from
/// it cannot outlive the call. `Node` is `Copy` with a lifetime tied to the
/// tree rather than the cursor, so collecting first is both correct and cheap.
pub(crate) fn children_of(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

/// First direct child of the given kind.
pub(crate) fn child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    children_of(node).into_iter().find(|c| c.kind() == kind)
}

/// First direct child matching any of the given kinds.
pub(crate) fn child_of_any<'t>(node: Node<'t>, kinds: &[&str]) -> Option<Node<'t>> {
    children_of(node).into_iter().find(|c| kinds.contains(&c.kind()))
}

/// Source text of a node.
///
/// Uses a checked slice: byte ranges from a tree parsed against a *different*
/// string would otherwise panic, and the fail-open contract does not survive a
/// panic.
pub(crate) fn text(node: Node<'_>, src: &str) -> String {
    src.get(node.byte_range()).unwrap_or_default().to_string()
}

/// Source text of a named field.
pub(crate) fn field_text(node: Node<'_>, field: &str, src: &str) -> Option<String> {
    node.child_by_field_name(field).map(|n| text(n, src))
}

/// One-based line of a node.
pub(crate) fn line_of(node: Node<'_>) -> usize {
    node.start_position().row + 1
}

/// Parse `source` with `language`, or fail with a typed error.
pub(crate) fn parse(
    language: &tree_sitter::Language,
    name: &'static str,
    source: &str,
    path: &str,
) -> crate::Result<tree_sitter::Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(language).map_err(|_| crate::ExtractError::Grammar { language: name })?;
    parser.parse(source, None).ok_or_else(|| crate::ExtractError::Parse { path: path.to_string() })
}

/// Walk every descendant, deepest-last, applying `visit`.
///
/// Iterative rather than recursive: a generated or minified file can nest
/// thousands deep, and blowing the stack inside a hook is indistinguishable
/// from a crash.
pub(crate) fn walk<'t, F: FnMut(Node<'t>)>(root: Node<'t>, mut visit: F) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        visit(node);
        stack.extend(children_of(node));
    }
}

/// Strip one layer of quotes from a string literal as written.
pub(crate) fn unquote(raw: &str) -> String {
    raw.trim_matches(['"', '\'', '`']).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(src: &str) -> tree_sitter::Tree {
        parse(&tree_sitter_ruby::LANGUAGE.into(), "Ruby", src, "t.rb").unwrap()
    }

    #[test]
    fn text_of_a_node_is_its_source_slice() {
        let src = "class Foo\nend\n";
        let t = tree(src);
        let class = child_of_kind(t.root_node(), "class").unwrap();
        assert!(text(class, src).starts_with("class Foo"));
    }

    #[test]
    fn text_against_a_mismatched_source_returns_empty_rather_than_panicking() {
        let t = tree("class Foo\nend\n");
        let class = child_of_kind(t.root_node(), "class").unwrap();
        assert_eq!(text(class, ""), "");
    }

    #[test]
    fn walking_visits_every_node_without_recursion() {
        let t = tree("class A\n  def b; end\nend\n");
        let mut kinds = Vec::new();
        walk(t.root_node(), |n| kinds.push(n.kind().to_string()));
        assert!(kinds.iter().any(|k| k == "class"));
        assert!(kinds.iter().any(|k| k == "method"));
    }

    #[test]
    fn deep_nesting_does_not_blow_the_stack() {
        let deep = format!("{}{}", "[".repeat(2_000), "]".repeat(2_000));
        let t = tree(&deep);
        let mut count = 0usize;
        walk(t.root_node(), |_| count += 1);
        assert!(count > 100);
    }

    #[test]
    fn unquoting_handles_each_literal_style() {
        assert_eq!(unquote("\"json\""), "json");
        assert_eq!(unquote("'json'"), "json");
        assert_eq!(unquote("`json`"), "json");
        assert_eq!(unquote("json"), "json");
    }

    #[test]
    fn line_numbers_are_one_based() {
        let t = tree("\n\nclass A\nend\n");
        let class = child_of_kind(t.root_node(), "class").unwrap();
        assert_eq!(line_of(class), 3);
    }
}
