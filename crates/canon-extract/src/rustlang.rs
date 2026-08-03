//! Rust: private by default, and methods live in `impl` blocks away from the
//! type.
//!
//! Like Go this needs two passes. Unlike Go, a type can have many `impl`
//! blocks, and an `impl Trait for Type` block describes a contract rather than
//! the type's own surface. Both are collected; the trait name is recorded the
//! way a base class is in other languages, because it is the closest analogue.

use crate::util::{child_of_kind, children_of, field_text, line_of, text};
use crate::{FileFacts, TypeFacts};

pub(crate) fn extract(tree: &tree_sitter::Tree, source: &str) -> FileFacts {
    let root = tree.root_node();
    let mut facts = FileFacts::default();

    for child in children_of(root) {
        match child.kind() {
            "struct_item" | "enum_item" | "trait_item" | "union_item" => {
                if let Some(name) = field_text(child, "name", source) {
                    facts.types.push(TypeFacts {
                        name,
                        line: line_of(child),
                        public_methods: Vec::new(),
                        private_methods: Vec::new(),
                        superclass: None,
                    });
                }
            }
            "function_item" => {
                if is_pub(child)
                    && let Some(n) = field_text(child, "name", source)
                {
                    facts.free_functions.push(n);
                }
            }
            "use_declaration" => {
                if let Some(arg) = child.child_by_field_name("argument") {
                    facts.imports.push(text(arg, source));
                }
            }
            _ => {}
        }
    }

    for child in children_of(root).into_iter().filter(|c| c.kind() == "impl_item") {
        let Some(ty) = field_text(child, "type", source) else { continue };
        let trait_name = field_text(child, "trait", source);
        let Some(target) = facts.types.iter_mut().find(|t| t.name == ty) else { continue };
        if let Some(t) = trait_name {
            target.superclass.get_or_insert(t);
        }
        let Some(body) = child_of_kind(child, "declaration_list") else { continue };
        for item in children_of(body).into_iter().filter(|c| c.kind() == "function_item") {
            let Some(name) = field_text(item, "name", source) else { continue };
            if is_pub(item) {
                target.public_methods.push(name);
            } else {
                target.private_methods.push(name);
            }
        }
    }

    facts
}

/// Any `pub`, including `pub(crate)`, counts as surface. The distinction
/// between crate-public and fully public is a packaging concern, not a shape
/// one, and conventions here are about shape.
fn is_pub(item: tree_sitter::Node<'_>) -> bool {
    child_of_kind(item, "visibility_modifier").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(src: &str) -> FileFacts {
        crate::tests::facts_of(crate::Language::Rust, src)
    }

    #[test]
    fn pub_decides_visibility_and_private_is_the_default() {
        let f = f("pub struct S;\nimpl S {\n  pub fn call(&self) {}\n  fn helper(&self) {}\n}\n");
        let t = &f.types[0];
        assert_eq!(t.public_methods, vec!["call"]);
        assert_eq!(t.private_methods, vec!["helper"]);
        assert_eq!(t.public_arity(), 1);
    }

    #[test]
    fn pub_crate_still_counts_as_surface() {
        let f = f("pub struct S;\nimpl S { pub(crate) fn call(&self) {} }\n");
        assert_eq!(f.types[0].public_arity(), 1);
    }

    #[test]
    fn methods_from_several_impl_blocks_accumulate() {
        let f = f("pub struct S;\nimpl S { pub fn a(&self) {} }\nimpl S { pub fn b(&self) {} }\n");
        assert_eq!(f.types[0].public_arity(), 2);
    }

    #[test]
    fn an_implemented_trait_is_recorded_as_the_base() {
        let f = f("pub struct S;\nimpl Display for S { fn fmt(&self) {} }\n");
        assert_eq!(f.types[0].superclass.as_deref(), Some("Display"));
    }

    #[test]
    fn enums_and_traits_are_types_too() {
        let f = f("pub enum E { A }\npub trait T {}\n");
        assert_eq!(f.types.len(), 2);
    }

    #[test]
    fn only_pub_free_functions_are_public_surface() {
        let f = f("pub fn a() {}\nfn b() {}\n");
        assert_eq!(f.free_functions, vec!["a"]);
    }

    #[test]
    fn use_declarations_become_imports() {
        let f = f("use std::collections::HashMap;\n");
        assert_eq!(f.imports, vec!["std::collections::HashMap"]);
    }

    #[test]
    fn malformed_source_yields_partial_facts_rather_than_failing() {
        for bad in ["pub struct", "fn fn fn", "\u{0}", "impl S {", ""] {
            let _ = f(bad);
        }
    }
}
