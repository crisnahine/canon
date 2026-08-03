//! Ruby: visibility is a section keyword.
//!
//! A bare `private` inside a class body flips everything declared after it.
//! Against the real AST that bare `private` is an `identifier` node sitting
//! directly inside `body_statement`, not a `call` as it reads. That is why this
//! is a positional cursor walk rather than a tree-sitter query: a query matches
//! nodes, and visibility here is state that accumulates as you move through
//! siblings.
//!
//! Guessing the query form would have failed quietly. It would have found zero
//! private methods and reported every helper as public, inflating the arity of
//! every class in the repository and producing a confident, wrong convention.

use crate::util::{child_of_kind, children_of, line_of, parse, text, unquote};
use crate::{FileFacts, Result, TypeFacts};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Public,
    Private,
}

pub(crate) fn extract(source: &str, path: &str) -> Result<FileFacts> {
    let tree = parse(&tree_sitter_ruby::LANGUAGE.into(), "Ruby", source, path)?;
    let mut facts = FileFacts::default();
    collect(tree.root_node(), source, &mut facts);
    Ok(facts)
}

/// Walk the whole file iteratively, carrying whether we are inside a type.
///
/// Iterative rather than recursive, and this is not a style preference: a
/// recursive version overflowed the stack on a real file and aborted the
/// process with SIGABRT. That is the one failure the fail-open harness cannot
/// disguise, because the process dies before it can print anything, and the
/// host sees a crash rather than a hook that chose to stay quiet.
///
/// The `inside_type` flag is what stops a method being counted twice, once as
/// a member of its class and once as a module-level function.
fn collect(root: tree_sitter::Node<'_>, src: &str, facts: &mut FileFacts) {
    let mut stack = vec![(root, false)];
    while let Some((node, inside_type)) = stack.pop() {
        for child in children_of(node) {
            match child.kind() {
                "class" | "module" => {
                    if let Some(t) = type_facts(child, src) {
                        facts.types.push(t);
                    }
                    // Descend into the body for *nested* types only; the
                    // methods were already resolved by `type_facts`.
                    if let Some(body) = child_of_kind(child, "body_statement") {
                        stack.push((body, true));
                    }
                }
                "method" => {
                    if inside_type {
                        continue;
                    }
                    if let Some(n) = child_of_kind(child, "identifier").map(|n| text(n, src)) {
                        facts.free_functions.push(n);
                    }
                }
                "call" => {
                    if let Some(t) = require_target(child, src) {
                        facts.imports.push(t);
                    }
                    stack.push((child, inside_type));
                }
                _ => stack.push((child, inside_type)),
            }
        }
    }
}

fn type_facts(class_node: tree_sitter::Node<'_>, src: &str) -> Option<TypeFacts> {
    // Read the named fields rather than looking for a `constant` child. A
    // compound name parses as `scope_resolution`, not `constant`, so matching
    // on kind silently skipped every `class Hubspot::EnrollSequence` in a real
    // Rails codebase and captured only the small error class nested inside it.
    // The result was a confident, wrong claim that services inherit from
    // `StandardError`.
    let name = class_node.child_by_field_name("name").map(|n| text(n, src))?;
    let superclass = class_node
        .child_by_field_name("superclass")
        .and_then(|s| children_of(s).into_iter().find(tree_sitter::Node::is_named))
        .map(|n| text(n, src));

    let mut public_methods = Vec::new();
    let mut private_methods = Vec::new();
    let mut section = Section::Public;

    if let Some(body) = child_of_kind(class_node, "body_statement") {
        for child in children_of(body) {
            match child.kind() {
                // A bare `private` or `protected`: flips every later method.
                "identifier" => {
                    if matches!(text(child, src).as_str(), "private" | "protected") {
                        section = Section::Private;
                    }
                }
                // `private :foo` names one method and leaves the section alone.
                "call" => {
                    if let Some((kw, target)) = visibility_call(child, src) {
                        if matches!(kw.as_str(), "private" | "protected") {
                            private_methods.push(target);
                        }
                    }
                }
                "method" => {
                    if let Some(n) = child_of_kind(child, "identifier").map(|n| text(n, src)) {
                        match section {
                            Section::Public => public_methods.push(n),
                            Section::Private => private_methods.push(n),
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // `private :foo` written below the definition demotes it after the fact.
    public_methods.retain(|m| !private_methods.contains(m));

    Some(TypeFacts { name, line: line_of(class_node), public_methods, private_methods, superclass })
}

fn visibility_call(node: tree_sitter::Node<'_>, src: &str) -> Option<(String, String)> {
    let kw = child_of_kind(node, "identifier").map(|n| text(n, src))?;
    let args = child_of_kind(node, "argument_list")?;
    let target = children_of(args).into_iter().find_map(|c| match c.kind() {
        "simple_symbol" => Some(text(c, src).trim_start_matches(':').to_string()),
        "string" => Some(unquote(&text(c, src))),
        _ => None,
    })?;
    Some((kw, target))
}

fn require_target(node: tree_sitter::Node<'_>, src: &str) -> Option<String> {
    let m = child_of_kind(node, "identifier").map(|n| text(n, src))?;
    if !m.starts_with("require") {
        return None;
    }
    let args = child_of_kind(node, "argument_list")?;
    children_of(args).into_iter().find(|c| c.kind() == "string").map(|c| unquote(&text(c, src)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(src: &str) -> FileFacts {
        extract(src, "t.rb").expect("parses")
    }

    #[test]
    fn the_section_keyword_splits_public_from_private() {
        let facts = f("class Create\n  def call; end\n  private\n  def helper; end\nend\n");
        let t = &facts.types[0];
        assert_eq!(t.name, "Create");
        assert_eq!(t.public_methods, vec!["call"]);
        assert_eq!(t.private_methods, vec!["helper"]);
        assert_eq!(t.public_arity(), 1);
    }

    #[test]
    fn everything_after_the_keyword_is_private_not_just_the_next_method() {
        let facts = f(
            "class A\n  def call; end\n  private\n  def x; end\n  def y; end\n  def z; end\nend\n",
        );
        assert_eq!(facts.types[0].public_arity(), 1);
        assert_eq!(facts.types[0].private_methods.len(), 3);
    }

    #[test]
    fn protected_behaves_as_a_section_keyword_too() {
        let facts = f("class A\n  def call; end\n  protected\n  def helper; end\nend\n");
        assert_eq!(facts.types[0].public_arity(), 1);
    }

    #[test]
    fn an_inline_private_symbol_demotes_an_earlier_method() {
        let facts = f("class A\n  def call; end\n  def hidden; end\n  private :hidden\nend\n");
        let t = &facts.types[0];
        assert_eq!(t.public_methods, vec!["call"]);
        assert!(t.private_methods.contains(&"hidden".to_string()));
    }

    #[test]
    fn a_superclass_is_captured() {
        let facts = f("class Create < ApplicationService\nend\n");
        assert_eq!(facts.types[0].superclass.as_deref(), Some("ApplicationService"));
    }

    #[test]
    fn a_compound_class_name_is_captured() {
        // The dominant Rails service-object shape. Matching on node kind found
        // `constant` and missed `scope_resolution`, so these files reported no
        // main class at all.
        let facts =
            f("class Hubspot::EnrollSequence < ActiveInteraction::Base\n  def execute; end\nend\n");
        assert_eq!(facts.types[0].name, "Hubspot::EnrollSequence");
        assert_eq!(facts.types[0].superclass.as_deref(), Some("ActiveInteraction::Base"));
        assert_eq!(facts.types[0].public_methods, vec!["execute"]);
    }

    #[test]
    fn a_nested_error_class_does_not_replace_the_subject() {
        // Verbatim shape of app/services/hubspot/enroll_sequence.rb: the real
        // class plus a small error type declared inside it.
        let facts = f(
            "class Hubspot::EnrollSequence < ActiveInteraction::Base\n  def execute; end\n\n  class SenderConfigurationError < StandardError; end\nend\n",
        );
        assert_eq!(facts.types.len(), 2);
        assert_eq!(facts.types[0].name, "Hubspot::EnrollSequence");
        assert_eq!(facts.types[1].name, "SenderConfigurationError");
    }

    #[test]
    fn nested_classes_are_found_and_counted_once() {
        let facts = f("module Enrolments\n  class Create\n    def call; end\n  end\nend\n");
        assert!(facts.types.iter().any(|t| t.name == "Create"));
        assert_eq!(facts.types.iter().filter(|t| t.name == "Create").count(), 1);
        assert!(facts.free_functions.is_empty(), "a method in a class is not a free function");
    }

    #[test]
    fn requires_become_imports() {
        let facts = f("require 'json'\nrequire_relative 'foo'\nclass A; end\n");
        assert!(facts.imports.contains(&"json".to_string()));
        assert!(facts.imports.contains(&"foo".to_string()));
    }

    #[test]
    fn malformed_source_yields_partial_facts_rather_than_failing() {
        for bad in ["class", "def def def", "\u{0}\u{1}", "class A\n  def", ""] {
            assert!(extract(bad, "t.rb").is_ok(), "errored on {bad:?}");
        }
    }

    #[test]
    fn a_deeply_nested_file_does_not_overflow_the_stack() {
        // A recursive walk aborted the process here with SIGABRT, which the
        // host cannot tell apart from a crash. Depth well past anything a
        // human writes, and well within what a generator emits.
        let deep = format!("{}{}", "if x\n".repeat(5_000), "end\n".repeat(5_000));
        assert!(extract(&deep, "deep.rb").is_ok());

        let nested_arrays = format!("x = {}{}", "[".repeat(5_000), "]".repeat(5_000));
        assert!(extract(&nested_arrays, "arrays.rb").is_ok());

        let mut nested_classes = String::new();
        for i in 0..500 {
            nested_classes.push_str(&format!("class C{i}\n"));
        }
        nested_classes.push_str(&"end\n".repeat(500));
        assert!(extract(&nested_classes, "classes.rb").is_ok());
    }

    #[test]
    fn a_method_inside_a_class_is_not_also_a_module_function() {
        let facts = f("class A\n  def call; end\nend\ndef top_level; end\n");
        assert_eq!(facts.free_functions, vec!["top_level"]);
        assert_eq!(facts.types[0].public_methods, vec!["call"]);
    }
}
