//! Python: visibility is a naming convention the language does not enforce.
//!
//! A single leading underscore means private by agreement. Dunder methods are
//! neither: `__init__` is not part of the deliberate public surface, but it is
//! not a hidden helper either, so counting it in either list distorts arity.
//! They are excluded from both.

use crate::util::{child_of_any, child_of_kind, children_of, field_text, line_of, text};
use crate::{FileFacts, TypeFacts};

pub(crate) fn extract(tree: &tree_sitter::Tree, source: &str) -> FileFacts {
    let mut facts = FileFacts::default();
    for child in children_of(tree.root_node()) {
        // A decorated definition wraps the class or function it decorates.
        // Matching only the bare kinds made `@dataclass`, `@register`,
        // `@app.route` and `@pytest.fixture` invisible, which is most of the
        // interesting surface in a real Python codebase: identical repositories
        // derived five conventions plain and two decorated.
        let child = if child.kind() == "decorated_definition" {
            match child_of_any(child, &["class_definition", "function_definition"]) {
                Some(inner) => inner,
                None => continue,
            }
        } else {
            child
        };
        match child.kind() {
            "class_definition" => {
                if let Some(t) = type_facts(child, source) {
                    facts.types.push(t);
                }
            }
            "function_definition" => {
                if let Some(n) = field_text(child, "name", source)
                    && visibility(&n) == Vis::Public
                {
                    facts.free_functions.push(n);
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
    // Positional bases only, in order. A keyword argument is configuration
    // (`metaclass=`, `total=`), never a parent. Matching only a bare
    // `identifier` read `class X(base.BaseService)` as an `attribute` and
    // `class X(BaseService[Order])` as a `subscript`, so both came out with no
    // base at all and were then refused for having none.
    let bases: Vec<String> = class_node
        .child_by_field_name("superclasses")
        .map(|a| {
            children_of(a)
                .into_iter()
                .filter(|c| {
                    !matches!(
                        c.kind(),
                        "(" | ")"
                            | ","
                            | "comment"
                            | "keyword_argument"
                            | "list_splat"
                            | "dictionary_splat"
                    )
                })
                .map(|n| bare_base(&text(n, src)))
                .collect()
        })
        .unwrap_or_default();
    // The last positional base is the type the class is; anything before it is
    // a mixin, and a mixin is composition the type opts into rather than its
    // parent.
    let superclass = bases.last().cloned();
    let mixins: Vec<String> = bases.iter().take(bases.len().saturating_sub(1)).cloned().collect();
    let interfaces = Vec::new();

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

    Some(TypeFacts {
        name,
        line: line_of(class_node),
        public_methods,
        private_methods,
        superclass,
        bases,
        interfaces,
        mixins,
    })
}

/// A base as written, minus its type parameters.
///
/// `BaseService[Order]` and `BaseService` name the same class; the type
/// parameter is how the file reached it, not what it is. The module a base is
/// qualified with stays: the statement tells the author what to write, and an
/// author writes `models.Model`, not `Model`.
fn bare_base(raw: &str) -> String {
    crate::util::bare_type(raw)
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
    fn a_decorated_class_is_still_the_subject_of_the_file() {
        let f = f("@dataclass\nclass Create(BaseService):\n    def execute(self): pass\n");
        assert_eq!(f.types.len(), 1);
        assert_eq!(f.types[0].name, "Create");
        assert_eq!(f.types[0].superclass.as_deref(), Some("BaseService"));
        assert_eq!(f.types[0].public_methods, vec!["execute"]);
    }

    #[test]
    fn a_decorated_module_function_is_public_surface() {
        let f = f("@app.route('/x')\ndef handler(): pass\n");
        assert_eq!(f.free_functions, vec!["handler"]);
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

    #[test]
    fn the_last_positional_base_is_the_base_type() {
        // Django puts mixins first and the concrete view last, so the first base
        // names the behaviour bolted on and the last names what the class is.
        let f = f("class V(LoginRequiredMixin, ListView):\n    def get(self): pass\n");
        assert_eq!(f.types[0].superclass.as_deref(), Some("ListView"));
        assert_eq!(f.types[0].bases, vec!["LoginRequiredMixin", "ListView"]);
        assert_eq!(f.types[0].mixins, vec!["LoginRequiredMixin"]);
        assert!(f.types[0].interfaces.is_empty());
    }

    #[test]
    fn a_keyword_argument_in_the_base_list_is_not_a_base() {
        let f = f("class M(Base, metaclass=ABCMeta):\n    def go(self): pass\n");
        assert_eq!(f.types[0].superclass.as_deref(), Some("Base"));
        assert_eq!(f.types[0].bases, vec!["Base"]);
    }

    #[test]
    fn a_single_base_is_unchanged_by_ordering() {
        // `bare_base` already strips a trailing `[...]`; adding the ordering
        // logic for a multi-base class must not disturb that.
        let f = f("class P(BaseService[Order]):\n    def save(self): pass\n");
        assert_eq!(f.types[0].superclass.as_deref(), Some("BaseService"));
        assert!(f.types[0].interfaces.is_empty());
        assert!(f.types[0].mixins.is_empty());
    }

    #[test]
    fn a_qualified_base_keeps_its_module_path() {
        // The statement is an instruction: an author writes `models.Model`,
        // not `Model`. Stripping the module made the statement name a class
        // nobody types.
        let f = f("class P(models.Model):\n    def save(self): pass\n");
        assert_eq!(f.types[0].superclass.as_deref(), Some("models.Model"));
    }
}
