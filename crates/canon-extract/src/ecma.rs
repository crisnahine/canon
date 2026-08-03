//! JavaScript, JSX, TypeScript and TSX: one extractor, four grammars.
//!
//! The four share a class and module shape, so they share an implementation.
//! What differs is only which grammar is loaded and whether
//! `accessibility_modifier` nodes can appear, and both differences are data.
//!
//! Two decisions worth stating, because they change every count downstream:
//!
//! - `constructor` is not part of the public surface. Every class has one, so
//!   counting it would floor every arity at one and quietly destroy the
//!   "exactly one public method" convention this tool exists to find.
//! - A module's public surface is what it exports. A top-level helper that is
//!   never exported is private to the file, so it is not a free function for
//!   convention purposes. This is what makes "files here export exactly one
//!   component" derivable in a React codebase.

use crate::util::{
    child_of_any, child_of_kind, children_of, field_text, line_of, parse, text, unquote,
};
use crate::{FileFacts, Language, Result, TypeFacts};

pub(crate) fn extract(language: Language, source: &str, path: &str) -> Result<FileFacts> {
    let (grammar, name) = match language {
        Language::TypeScript => (tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), "TypeScript"),
        Language::Tsx => (tree_sitter_typescript::LANGUAGE_TSX.into(), "TSX"),
        Language::Jsx => (tree_sitter_javascript::LANGUAGE.into(), "JSX"),
        _ => (tree_sitter_javascript::LANGUAGE.into(), "JavaScript"),
    };
    let tree = parse(&grammar, name, source, path)?;
    let mut facts = FileFacts::default();
    for child in children_of(tree.root_node()) {
        top_level(child, source, &mut facts);
    }
    Ok(facts)
}

fn top_level(node: tree_sitter::Node<'_>, src: &str, facts: &mut FileFacts) {
    match node.kind() {
        "import_statement" => {
            if let Some(s) = field_text(node, "source", src) {
                facts.imports.push(unquote(&s));
            }
        }
        "export_statement" => export(node, src, facts),
        "class_declaration" | "abstract_class_declaration" => {
            if let Some(t) = type_facts(node, src) {
                facts.types.push(t);
            }
        }
        _ => {}
    }
}

/// An `export` is the boundary between a module's surface and its internals.
fn export(node: tree_sitter::Node<'_>, src: &str, facts: &mut FileFacts) {
    // `export { Foo, Bar }` re-exports names declared elsewhere in the file.
    if let Some(clause) = child_of_kind(node, "export_clause") {
        for spec in children_of(clause).into_iter().filter(|c| c.kind() == "export_specifier") {
            if let Some(n) = field_text(spec, "name", src) {
                facts.free_functions.push(n);
            }
        }
    }

    let Some(decl) = node.child_by_field_name("declaration") else { return };
    match decl.kind() {
        "class_declaration" | "abstract_class_declaration" => {
            if let Some(t) = type_facts(decl, src) {
                facts.types.push(t);
            }
        }
        "function_declaration" | "generator_function_declaration" => {
            if let Some(n) = field_text(decl, "name", src) {
                facts.free_functions.push(n);
            }
        }
        // `export const Foo = () => {}` — the dominant React component shape,
        // and invisible to anything that only looks at function_declaration.
        "lexical_declaration" | "variable_declaration" => {
            for d in children_of(decl).into_iter().filter(|c| c.kind() == "variable_declarator") {
                let is_callable = d
                    .child_by_field_name("value")
                    .is_some_and(|v| matches!(v.kind(), "arrow_function" | "function_expression"));
                if is_callable {
                    if let Some(n) = field_text(d, "name", src) {
                        facts.free_functions.push(n);
                    }
                }
            }
        }
        _ => {}
    }
}

fn type_facts(class_node: tree_sitter::Node<'_>, src: &str) -> Option<TypeFacts> {
    let name = field_text(class_node, "name", src)?;
    let superclass =
        child_of_kind(class_node, "class_heritage").and_then(|h| heritage_name(h, src));

    let mut public_methods = Vec::new();
    let mut private_methods = Vec::new();

    if let Some(body) = child_of_kind(class_node, "class_body") {
        for member in children_of(body) {
            if !matches!(
                member.kind(),
                "method_definition" | "method_signature" | "abstract_method_signature"
            ) {
                continue;
            }
            let Some(member_name) = field_text(member, "name", src) else { continue };
            // Every class has a constructor. Counting it floors every arity at
            // one and erases the convention being looked for.
            if member_name == "constructor" {
                continue;
            }
            if is_private(member, &member_name, src) {
                private_methods.push(member_name);
            } else {
                public_methods.push(member_name);
            }
        }
    }

    Some(TypeFacts { name, line: line_of(class_node), public_methods, private_methods, superclass })
}

/// Two independent private markers, and both must be honoured.
///
/// `private x()` is TypeScript's, erased at compile time. `#x()` is the
/// language's own, enforced at runtime, and is the only one available in plain
/// JavaScript.
fn is_private(member: tree_sitter::Node<'_>, member_name: &str, src: &str) -> bool {
    if member_name.starts_with('#') {
        return true;
    }
    child_of_kind(member, "accessibility_modifier")
        .map(|m| text(m, src))
        .is_some_and(|m| matches!(m.trim(), "private" | "protected"))
}

fn heritage_name(heritage: tree_sitter::Node<'_>, src: &str) -> Option<String> {
    // JavaScript puts the expression directly under class_heritage; TypeScript
    // wraps it in an extends_clause alongside any implements_clause.
    let scope = child_of_kind(heritage, "extends_clause").unwrap_or(heritage);
    child_of_any(scope, &["identifier", "type_identifier", "member_expression", "generic_type"])
        .map(|n| text(n, src))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(src: &str) -> FileFacts {
        extract(Language::TypeScript, src, "t.ts").expect("parses")
    }
    fn tsx(src: &str) -> FileFacts {
        extract(Language::Tsx, src, "t.tsx").expect("parses")
    }
    fn js(src: &str) -> FileFacts {
        extract(Language::JavaScript, src, "t.js").expect("parses")
    }

    #[test]
    fn a_typescript_accessibility_modifier_makes_a_method_private() {
        let f = ts("export class Create { call(): void {} private helper(): void {} }");
        let t = &f.types[0];
        assert_eq!(t.public_methods, vec!["call"]);
        assert_eq!(t.private_methods, vec!["helper"]);
        assert_eq!(t.public_arity(), 1);
    }

    #[test]
    fn a_hash_prefix_is_private_in_plain_javascript_too() {
        let f = js("export class A { call() {} #helper() {} }");
        assert_eq!(f.types[0].public_arity(), 1);
        assert_eq!(f.types[0].private_methods.len(), 1);
    }

    #[test]
    fn the_constructor_is_not_part_of_the_public_surface() {
        // Counting it would floor every class at arity 1 and erase the
        // single-public-method convention entirely.
        let f = ts("export class A { constructor(private x: number) {} call() {} }");
        assert_eq!(f.types[0].public_arity(), 1);
        assert!(!f.types[0].public_methods.contains(&"constructor".to_string()));
    }

    #[test]
    fn protected_counts_as_private_for_surface_purposes() {
        let f = ts("class A { call() {} protected helper() {} }");
        assert_eq!(f.types[0].public_arity(), 1);
    }

    #[test]
    fn an_exported_arrow_const_is_a_free_function() {
        // The dominant React component shape. Anything that only looks at
        // function_declaration reports a components directory as empty.
        let f = tsx("export const Button = (props: P) => <button/>;");
        assert_eq!(f.free_functions, vec!["Button"]);
    }

    #[test]
    fn a_top_level_helper_that_is_never_exported_is_not_public_surface() {
        let f = ts("const helper = () => 1;\nexport const Main = () => helper();");
        assert_eq!(f.free_functions, vec!["Main"]);
    }

    #[test]
    fn a_named_export_clause_contributes_to_the_surface() {
        let f = ts("const A = () => 1;\nexport { A };");
        assert_eq!(f.free_functions, vec!["A"]);
    }

    #[test]
    fn extends_is_captured_in_both_grammars() {
        assert_eq!(
            ts("export class A extends ApplicationService {}").types[0].superclass.as_deref(),
            Some("ApplicationService")
        );
        assert_eq!(
            js("export class A extends Base {}").types[0].superclass.as_deref(),
            Some("Base")
        );
    }

    #[test]
    fn implements_is_not_mistaken_for_extends() {
        let f = ts("export class A extends Base implements Iface {}");
        assert_eq!(f.types[0].superclass.as_deref(), Some("Base"));
    }

    #[test]
    fn imports_are_captured_as_written() {
        let f = ts("import { a } from './lib/a';\nimport b from \"pkg\";");
        assert_eq!(f.imports, vec!["./lib/a", "pkg"]);
    }

    #[test]
    fn jsx_in_a_tsx_file_does_not_break_the_parse() {
        let f = tsx(
            "export const Card = ({title}: {title: string}) => (\n  <div className=\"c\">{title}</div>\n);\n",
        );
        assert_eq!(f.free_functions, vec!["Card"]);
    }

    #[test]
    fn malformed_source_yields_partial_facts_rather_than_failing() {
        for bad in ["export class", "const = = =", "\u{0}", "class A { { {", ""] {
            assert!(extract(Language::TypeScript, bad, "t.ts").is_ok(), "errored on {bad:?}");
            assert!(extract(Language::Tsx, bad, "t.tsx").is_ok(), "errored on {bad:?}");
        }
    }
}
