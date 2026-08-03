//! Go: visibility is the case of the first letter, and methods live outside
//! the type they belong to.
//!
//! Both facts force a two-pass walk. Types are collected first, then method
//! declarations are attached by receiver name. A single pass would either miss
//! every method declared above its type or require the file to be ordered,
//! and real Go files are not.

use crate::util::{child_of_any, child_of_kind, children_of, field_text, line_of, text, unquote};
use crate::{FileFacts, TypeFacts};

pub(crate) fn extract(tree: &tree_sitter::Tree, source: &str) -> FileFacts {
    let root = tree.root_node();
    let mut facts = FileFacts::default();

    // Pass one: type declarations, in source order.
    for child in children_of(root) {
        match child.kind() {
            "type_declaration" => {
                for spec in children_of(child).into_iter().filter(|c| c.kind() == "type_spec") {
                    if let Some(t) = type_facts(spec, source) {
                        facts.types.push(t);
                    }
                }
            }
            "function_declaration" => {
                if let Some(n) = field_text(child, "name", source) {
                    if is_exported(&n) {
                        facts.free_functions.push(n);
                    }
                }
            }
            "import_declaration" => collect_imports(child, source, &mut facts),
            _ => {}
        }
    }

    // Pass two: attach methods to the type named by their receiver.
    for child in children_of(root) {
        if child.kind() != "method_declaration" {
            continue;
        }
        let (Some(recv), Some(name)) =
            (receiver_type(child, source), field_text(child, "name", source))
        else {
            continue;
        };
        let Some(t) = facts.types.iter_mut().find(|t| t.name == recv) else { continue };
        if is_exported(&name) {
            t.public_methods.push(name);
        } else {
            t.private_methods.push(name);
        }
    }

    facts
}

fn is_exported(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

fn type_facts(spec: tree_sitter::Node<'_>, src: &str) -> Option<TypeFacts> {
    let name = field_text(spec, "name", src)?;
    Some(TypeFacts {
        name,
        line: line_of(spec),
        public_methods: Vec::new(),
        private_methods: Vec::new(),
        superclass: embedded_type(spec, src),
    })
}

/// Go's nearest analogue to a base class is an embedded field: a field
/// declaration carrying a type and no name.
fn embedded_type(spec: tree_sitter::Node<'_>, src: &str) -> Option<String> {
    let body = child_of_kind(spec, "struct_type")
        .and_then(|s| child_of_kind(s, "field_declaration_list"))?;
    children_of(body)
        .into_iter()
        .filter(|f| f.kind() == "field_declaration")
        .find(|f| f.child_by_field_name("name").is_none())
        .and_then(|f| f.child_by_field_name("type"))
        .map(|t| text(t, src).trim_start_matches('*').to_string())
}

fn receiver_type(method: tree_sitter::Node<'_>, src: &str) -> Option<String> {
    let recv = method.child_by_field_name("receiver")?;
    let param = children_of(recv).into_iter().find(|c| c.kind() == "parameter_declaration")?;
    let ty = param.child_by_field_name("type")?;
    // A pointer receiver wraps the identifier; both bind to the same type.
    let ident = if ty.kind() == "pointer_type" {
        child_of_any(ty, &["type_identifier", "generic_type"])?
    } else {
        ty
    };
    Some(text(ident, src).trim_start_matches('*').to_string())
}

fn collect_imports(node: tree_sitter::Node<'_>, src: &str, facts: &mut FileFacts) {
    crate::util::walk(node, |n| {
        if n.kind() == "interpreted_string_literal" {
            facts.imports.push(unquote(&text(n, src)));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(src: &str) -> FileFacts {
        crate::tests::facts_of(crate::Language::Go, src)
    }

    #[test]
    fn capitalisation_decides_visibility() {
        let f = f(
            "package a\ntype Service struct{}\nfunc (s *Service) Call() {}\nfunc (s *Service) helper() {}\n",
        );
        let t = &f.types[0];
        assert_eq!(t.name, "Service");
        assert_eq!(t.public_methods, vec!["Call"]);
        assert_eq!(t.private_methods, vec!["helper"]);
        assert_eq!(t.public_arity(), 1);
    }

    #[test]
    fn a_method_declared_above_its_type_still_attaches() {
        // The reason this is a two-pass walk.
        let f = f("package a\nfunc (s Service) Call() {}\ntype Service struct{}\n");
        assert_eq!(f.types[0].public_methods, vec!["Call"]);
    }

    #[test]
    fn value_and_pointer_receivers_bind_to_the_same_type() {
        let f = f("package a\ntype S struct{}\nfunc (s S) A() {}\nfunc (s *S) B() {}\n");
        assert_eq!(f.types[0].public_arity(), 2);
    }

    #[test]
    fn an_embedded_field_is_the_closest_thing_to_a_base_type() {
        let f = f("package a\ntype S struct {\n  BaseService\n  name string\n}\n");
        assert_eq!(f.types[0].superclass.as_deref(), Some("BaseService"));
    }

    #[test]
    fn a_named_field_is_not_mistaken_for_an_embedded_one() {
        let f = f("package a\ntype S struct {\n  name string\n}\n");
        assert_eq!(f.types[0].superclass, None);
    }

    #[test]
    fn only_exported_package_functions_are_public_surface() {
        let f = f("package a\nfunc Public() {}\nfunc private() {}\n");
        assert_eq!(f.free_functions, vec!["Public"]);
    }

    #[test]
    fn imports_are_captured_from_a_grouped_block() {
        let f = f("package a\nimport (\n  \"fmt\"\n  \"net/http\"\n)\n");
        assert!(f.imports.contains(&"fmt".to_string()));
        assert!(f.imports.contains(&"net/http".to_string()));
    }

    #[test]
    fn malformed_source_yields_partial_facts_rather_than_failing() {
        for bad in ["package", "func func", "\u{0}", "type S struct {", ""] {
            let _ = f(bad);
        }
    }
}
