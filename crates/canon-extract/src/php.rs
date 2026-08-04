//! PHP: an explicit modifier, defaulting to public when absent.
//!
//! The default is what makes this different from TypeScript's superficially
//! similar `private` keyword. A method with no modifier at all is public, so
//! absence of evidence is evidence of publicity here and nowhere else.

use crate::util::{child_of_kind, children_of, field_text, line_of, text};
use crate::{FileFacts, TypeFacts};

pub(crate) fn extract(tree: &tree_sitter::Tree, source: &str) -> FileFacts {
    let mut facts = FileFacts::default();
    crate::util::walk(tree.root_node(), |node| match node.kind() {
        "class_declaration" | "interface_declaration" | "trait_declaration" => {
            if let Some(t) = type_facts(node, source) {
                facts.types.push(t);
            }
        }
        "function_definition" => {
            if let Some(n) = field_text(node, "name", source) {
                facts.free_functions.push(n);
            }
        }
        // The first wins. A file may reopen a namespace further down, but the
        // one at the top is the one PSR-4 pairs with the directory.
        "namespace_definition" if facts.namespace.is_none() => {
            if let Some(n) = field_text(node, "name", source) {
                facts.namespace = Some(n);
            }
        }
        "namespace_use_declaration" => {
            for clause in
                children_of(node).into_iter().filter(|c| c.kind() == "namespace_use_clause")
            {
                facts.imports.push(text(clause, source).trim().to_string());
            }
        }
        _ => {}
    });
    facts
}

fn type_facts(class_node: tree_sitter::Node<'_>, src: &str) -> Option<TypeFacts> {
    let name = field_text(class_node, "name", src)?;
    let superclass = child_of_kind(class_node, "base_clause")
        .and_then(|b| children_of(b).into_iter().find(|c| c.kind() == "name"))
        .map(|n| text(n, src));
    // `implements` names contracts the class holds beside its base.
    let interfaces = child_of_kind(class_node, "class_interface_clause")
        .map(|c| {
            children_of(c)
                .into_iter()
                .filter(|n| matches!(n.kind(), "name" | "qualified_name"))
                .map(|n| text(n, src))
                .collect()
        })
        .unwrap_or_default();

    let mut public_methods = Vec::new();
    let mut private_methods = Vec::new();
    if let Some(body) = child_of_kind(class_node, "declaration_list") {
        for member in children_of(body).into_iter().filter(|c| c.kind() == "method_declaration") {
            let Some(n) = field_text(member, "name", src) else { continue };
            // Constructors and the magic methods are not deliberate surface.
            if n == "__construct" || n.starts_with("__") {
                continue;
            }
            if is_hidden(member, src) {
                private_methods.push(n);
            } else {
                public_methods.push(n);
            }
        }
    }

    Some(TypeFacts {
        name,
        line: line_of(class_node),
        public_methods,
        private_methods,
        superclass,
        bases: Vec::new(),
        interfaces,
        mixins: Vec::new(),
    })
}

fn is_hidden(member: tree_sitter::Node<'_>, src: &str) -> bool {
    children_of(member)
        .into_iter()
        .filter(|c| c.kind() == "visibility_modifier")
        .any(|m| matches!(text(m, src).trim(), "private" | "protected"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(src: &str) -> FileFacts {
        crate::tests::facts_of(crate::Language::Php, src)
    }

    #[test]
    fn an_absent_modifier_means_public() {
        // The rule that separates PHP from TypeScript.
        let f = f("<?php class A { function call() {} private function helper() {} }");
        assert_eq!(f.types[0].public_methods, vec!["call"]);
        assert_eq!(f.types[0].private_methods, vec!["helper"]);
    }

    #[test]
    fn an_explicit_public_modifier_agrees_with_the_default() {
        let f = f("<?php class A { public function call() {} }");
        assert_eq!(f.types[0].public_arity(), 1);
    }

    #[test]
    fn protected_is_not_surface() {
        let f = f("<?php class A { public function call() {} protected function h() {} }");
        assert_eq!(f.types[0].public_arity(), 1);
    }

    #[test]
    fn the_constructor_and_magic_methods_are_excluded() {
        let f = f(
            "<?php class A { public function __construct() {} public function __toString() {} public function call() {} }",
        );
        assert_eq!(f.types[0].public_arity(), 1);
    }

    #[test]
    fn extends_is_captured() {
        let f = f("<?php class A extends BaseService {}");
        assert_eq!(f.types[0].superclass.as_deref(), Some("BaseService"));
    }

    #[test]
    fn interfaces_and_traits_are_types() {
        let f = f("<?php interface I { public function a(); } trait T { public function b() {} }");
        assert_eq!(f.types.len(), 2);
    }

    #[test]
    fn the_declared_namespace_is_a_fact() {
        // Issue #16. PSR-4 agreement between namespace and path is a real,
        // checkable PHP convention, and 134 tracked PHP files derived nothing
        // at all because the whole vocabulary asked about base classes.
        let f = f("<?php\nnamespace App\\Services\\Billing;\nclass ChargeCard {}\n");
        assert_eq!(f.namespace.as_deref(), Some("App\\Services\\Billing"));
    }

    #[test]
    fn a_file_with_no_namespace_declares_none() {
        // Most of a WordPress plugin, and the absence has to be reportable
        // rather than indistinguishable from a file nobody parsed.
        let f = f("<?php\nclass ChargeCard {}\n");
        assert_eq!(f.namespace, None);
    }

    #[test]
    fn malformed_source_yields_partial_facts_rather_than_failing() {
        for bad in ["<?php class", "<?php function function", "\u{0}", "<?php class A {", ""] {
            let _ = f(bad);
        }
    }
}
