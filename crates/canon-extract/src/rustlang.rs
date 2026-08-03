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
    let mut facts = FileFacts::default();
    let items = items_of(tree.root_node(), source);
    // The module each declared type lives in, parallel to `facts.types`. Two
    // modules in one file may declare the same name, and an `impl` belongs to
    // the one it can see.
    let mut modules: Vec<String> = Vec::new();

    for (module, child) in &items {
        match child.kind() {
            "struct_item" | "enum_item" | "trait_item" | "union_item" => {
                if let Some(name) = field_text(*child, "name", source) {
                    facts.types.push(TypeFacts {
                        name,
                        line: line_of(*child),
                        public_methods: Vec::new(),
                        private_methods: Vec::new(),
                        superclass: None,
                    });
                    modules.push(module.clone());
                }
            }
            "function_item" => {
                if is_pub(*child)
                    && let Some(n) = field_text(*child, "name", source)
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

    for (module, child) in items.iter().filter(|(_, c)| c.kind() == "impl_item") {
        let Some(ty) = field_text(*child, "type", source).map(|t| bare_name(&t)) else { continue };
        let trait_name = field_text(*child, "trait", source).map(|t| bare_name(&t));
        let Some(index) = target_of(&facts.types, &modules, &ty, module) else { continue };
        let Some(target) = facts.types.get_mut(index) else { continue };
        if let Some(t) = trait_name {
            target.superclass.get_or_insert(t);
        }
        let Some(body) = child_of_kind(*child, "declaration_list") else { continue };
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

/// Which declared type an `impl` block belongs to.
///
/// Matching on the bare name alone attached every block to whichever type was
/// declared first, and once `impl` blocks inside modules became visible that
/// was wrong in two shapes real Rust uses:
///
/// ```ignore
/// pub mod read  { pub struct Cursor; impl Cursor { pub fn get(&self) {} } }
/// pub mod write { pub struct Cursor; impl Cursor { pub fn get(&self) {} } }
///
/// pub struct Handle;
/// #[cfg(unix)]    mod imp { impl super::Handle { pub fn open(&self) {} } }
/// #[cfg(windows)] mod imp { impl super::Handle { pub fn open(&self) {} } }
/// ```
///
/// Both files declare types with one public method each. Both came out as one
/// type with two, and were refused by an arity rule they satisfy.
///
/// So the nearest declaration the block can see wins: the same module first,
/// then the closest enclosing one. A name declared once in the file is
/// unambiguous wherever it sits. A name declared in two sibling modules and
/// implemented from neither resolves to nothing, because guessing between them
/// is what produced the false positive.
fn target_of(types: &[TypeFacts], modules: &[String], name: &str, from: &str) -> Option<usize> {
    let mut visible: Option<(usize, usize)> = None;
    let mut all = Vec::new();
    for (index, declared) in types.iter().enumerate() {
        if declared.name != name {
            continue;
        }
        all.push(index);
        let Some(module) = modules.get(index) else { continue };
        if !in_scope(module, from) {
            continue;
        }
        // The most deeply nested visible declaration shadows the rest.
        if visible.is_none_or(|(_, depth)| module.len() > depth) {
            visible = Some((index, module.len()));
        }
    }
    if let Some((index, _)) = visible {
        return Some(index);
    }
    match all.as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

/// Whether a type declared in `module` is visible to an item in `from`: the
/// same module, or one enclosing it.
fn in_scope(module: &str, from: &str) -> bool {
    module.is_empty()
        || module == from
        || from.strip_prefix(module).is_some_and(|rest| rest.starts_with("::"))
}

/// Any `pub`, including `pub(crate)`, counts as surface. The distinction
/// between crate-public and fully public is a packaging concern, not a shape
/// one, and conventions here are about shape.
fn is_pub(item: tree_sitter::Node<'_>) -> bool {
    child_of_kind(item, "visibility_modifier").is_some()
}

/// A type name without its generic arguments or its path prefix.
///
/// `impl<W: Write> Printer<W>` names the type `Printer<W>`, which never equals
/// the `Printer` the struct declared, so every generic type came out with no
/// methods and no trait. Eighty-one of ripgrep's two hundred and sixty
/// top-level impls are generic.
fn bare_name(text: &str) -> String {
    let head = text.split_once('<').map_or(text, |(name, _)| name);
    head.rsplit("::").next().unwrap_or(head).trim().to_string()
}

/// Every item declared in the file, including the ones nested in a `mod`.
///
/// Rust files routinely wrap their contents in an inline module, and reading
/// only the root's direct children reported those files as declaring nothing
/// at all.
///
/// A module named `test` or `tests` is skipped, the same way a directory
/// called `tests` is skipped one layer up: `#[cfg(test)] mod tests` holds
/// fixtures, and their shape is not the file's.
///
/// Iterative rather than recursive, for the same reason every other walk here
/// is: blowing the stack inside a hook is indistinguishable from a crash.
fn items_of<'t>(root: tree_sitter::Node<'t>, src: &str) -> Vec<(String, tree_sitter::Node<'t>)> {
    let mut out = Vec::new();
    let mut stack = vec![(String::new(), root)];
    while let Some((path, node)) = stack.pop() {
        for child in children_of(node) {
            if child.kind() == "mod_item" {
                let name = field_text(child, "name", src).unwrap_or_default();
                if matches!(name.as_str(), "test" | "tests") {
                    continue;
                }
                if let Some(body) = child_of_kind(child, "declaration_list") {
                    let inner = if path.is_empty() { name } else { format!("{path}::{name}") };
                    stack.push((inner, body));
                }
                continue;
            }
            out.push((path.clone(), child));
        }
    }
    out
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
    fn a_generic_type_keeps_the_methods_declared_against_it() {
        let f = f(
            "pub struct Printer<W> { w: W }\nimpl<W: Write> Printer<W> {\n  pub fn print(&self) {}\n  fn helper(&self) {}\n}\n",
        );
        assert_eq!(f.types[0].name, "Printer");
        assert_eq!(f.types[0].public_methods, vec!["print"]);
        assert_eq!(f.types[0].public_arity(), 1);
    }

    #[test]
    fn a_generic_trait_implementation_records_the_bare_trait() {
        let f = f("pub struct S;\nimpl From<u8> for S { fn from(_: u8) -> Self { S } }\n");
        assert_eq!(f.types[0].superclass.as_deref(), Some("From"));
    }

    #[test]
    fn a_type_inside_an_inline_module_is_still_declared() {
        let f = f("pub mod inner {\n  pub struct S;\n  impl S { pub fn call(&self) {} }\n}\n");
        assert_eq!(f.types.len(), 1);
        assert_eq!(f.types[0].public_methods, vec!["call"]);
    }

    #[test]
    fn two_modules_may_declare_the_same_name_without_merging() {
        let f = f(
            "pub mod read { pub struct Cursor; impl Cursor { pub fn get(&self) {} } }\npub mod write { pub struct Cursor; impl Cursor { pub fn put(&self) {} } }\n",
        );
        assert_eq!(f.types.len(), 2);
        assert_eq!(f.types[0].public_arity(), 1, "impls merged onto one type");
        assert_eq!(f.types[1].public_arity(), 1, "impls merged onto one type");
    }

    #[test]
    fn a_child_module_may_implement_its_parents_type() {
        let f = f("pub struct Handle;\nmod imp { impl super::Handle { pub fn open(&self) {} } }\n");
        assert_eq!(f.types.len(), 1);
        assert_eq!(f.types[0].public_methods, vec!["open"]);
    }

    #[test]
    fn conditionally_compiled_alternatives_are_one_method_not_two() {
        // The cross-platform idiom. Only one branch ever compiles, but a parser
        // sees both, and the file was refused by an arity rule it satisfies.
        let f = f(
            "pub struct Handle;\n#[cfg(unix)]\nmod imp { impl super::Handle { pub fn open(&self) {} } }\n#[cfg(windows)]\nmod imp2 { impl super::Handle { pub fn open(&self) {} } }\n",
        );
        assert_eq!(f.types[0].public_arity(), 1);
        assert_eq!(f.types[0].public_methods, vec!["open"]);
    }

    #[test]
    fn two_genuinely_different_methods_still_count_as_two() {
        let f = f("pub struct S;\nimpl S {\n  pub fn a(&self) {}\n  pub fn b(&self) {}\n}\n");
        assert_eq!(f.types[0].public_arity(), 2);
    }

    #[test]
    fn a_cfg_test_module_is_not_part_of_the_file_shape() {
        let f = f("pub struct S;\n#[cfg(test)]\nmod tests {\n  struct Fixture;\n}\n");
        assert_eq!(f.types.len(), 1);
        assert_eq!(f.types[0].name, "S");
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
