//! Python: visibility is a naming convention the language does not enforce.
//!
//! A single leading underscore means private by agreement. Dunder methods are
//! neither: `__init__` is not part of the deliberate public surface, but it is
//! not a hidden helper either, so counting it in either list distorts arity.
//! They are excluded from both.

use crate::util::{child_of_kind, children_of, field_text, line_of, text};
use crate::{FileFacts, TypeFacts};

pub(crate) fn extract(tree: &tree_sitter::Tree, source: &str) -> FileFacts {
    let mut facts = FileFacts::default();
    for child in children_of(tree.root_node()) {
        match child.kind() {
            "class_definition" => {
                if let Some(t) = type_facts(child, source) {
                    facts.types.push(t);
                }
            }
            "function_definition" => {
                if let Some(n) = field_text(child, "name", source) {
                    if visibility(&n) == Vis::Public {
                        facts.free_functions.push(n);
                    }
                }
            }
            "import_statement" | "import_from_statement" => {
                if let Some(m) = child
                    .child_by_field_name("module_name")
                    .or_else(|| child_of_kind(child, "dotted_name"))
                {
                    facts.imports.push(text(m, source));
                }
            }
            _ => {}
        }
    }
    facts
}

#[derive(PartialEq, Eq)]
enum Vis {
    Public,
    Private,
    Dunder,
}

fn visibility(name: &str) -> Vis {
    if name.starts_with("__") && name.ends_with("__") {
        Vis::Dunder
    } else if name.starts_with('_') {
        Vis::Private
    } else {
        Vis::Public
    }
}

fn type_facts(class_node: tree_sitter::Node<'_>, src: &str) -> Option<TypeFacts> {
    let name = field_text(class_node, "name", src)?;
    let superclass = class_node
        .child_by_field_name("superclasses")
        .and_then(|a| children_of(a).into_iter().find(|c| c.kind() == "identifier"))
        .map(|n| text(n, src));

    let mut public_methods = Vec::new();
    let mut private_methods = Vec::new();
    if let Some(body) = class_node.child_by_field_name("body") {
        for member in children_of(body) {
            // A decorated method is wrapped, so unwrap before reading the name.
            let def = if member.kind() == "decorated_definition" {
                child_of_kind(member, "function_definition")
            } else if member.kind() == "function_definition" {
                Some(member)
            } else {
                None
            };
            let Some(def) = def else { continue };
            let Some(n) = field_text(def, "name", src) else { continue };
            match visibility(&n) {
                Vis::Public => public_methods.push(n),
                Vis::Private => private_methods.push(n),
                Vis::Dunder => {}
            }
        }
    }

    Some(TypeFacts { name, line: line_of(class_node), public_methods, private_methods, superclass })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(src: &str) -> FileFacts {
        crate::tests::facts_of(crate::Language::Python, src)
    }

    #[test]
    fn a_leading_underscore_is_private() {
        let f = f("class Create:\n    def call(self): pass\n    def _helper(self): pass\n");
        assert_eq!(f.types[0].public_methods, vec!["call"]);
        assert_eq!(f.types[0].private_methods, vec!["_helper"]);
    }

    #[test]
    fn dunders_count_as_neither_public_nor_private() {
        // Counting __init__ as public would floor every class at arity 1.
        let f = f("class A:\n    def __init__(self): pass\n    def call(self): pass\n");
        assert_eq!(f.types[0].public_arity(), 1);
        assert!(f.types[0].private_methods.is_empty());
    }

    #[test]
    fn a_decorated_method_is_still_seen() {
        let f = f("class A:\n    @property\n    def value(self): pass\n");
        assert_eq!(f.types[0].public_methods, vec!["value"]);
    }

    #[test]
    fn the_first_base_class_is_captured() {
        let f = f("class Create(ApplicationService):\n    pass\n");
        assert_eq!(f.types[0].superclass.as_deref(), Some("ApplicationService"));
    }

    #[test]
    fn a_private_module_function_is_not_public_surface() {
        let f = f("def public(): pass\ndef _private(): pass\n");
        assert_eq!(f.free_functions, vec!["public"]);
    }

    #[test]
    fn imports_are_captured_from_both_forms() {
        let f = f("import json\nfrom a.b import c\n");
        assert!(f.imports.contains(&"json".to_string()));
        assert!(f.imports.contains(&"a.b".to_string()));
    }

    #[test]
    fn malformed_source_yields_partial_facts_rather_than_failing() {
        for bad in ["class", "def def", "\u{0}", "class A:\n  def", ""] {
            let _ = f(bad);
        }
    }
}
