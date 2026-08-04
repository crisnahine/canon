; Rust facts.

; Payment::charge(x)
(call_expression
  function: (scoped_identifier
    path: (_) @call.receiver
    name: (identifier) @call))

; payment.charge(x) — a field_expression, not a method_call_expression. Rust
; has no separate node for a method call; the receiver and the method are a
; field access in the function position.
(call_expression
  function: (field_expression
    value: (_) @call.receiver
    field: (field_identifier) @call))

; charge(x)
(call_expression
  function: (identifier) @call)

; panic! and friends are Rust's raise.
(macro_invocation
  macro: (identifier) @raise
  (#any-of? @raise "panic" "unreachable" "todo" "unimplemented"))

(use_declaration
  argument: (_) @import)

; #[tokio::main] / #[cfg(test)] / #[serde(rename_all = "camelCase")]
;
; Excludes the bare word `derive`: it names almost every Rust file and says
; nothing on its own, so it is only ever surfaced through the trait list below.
(attribute [(identifier) (scoped_identifier)] @annotation
  (#not-eq? @annotation "derive"))

; #[derive(Debug, thiserror::Error)] — the list whole, split into one
; annotation per trait in Rust.
;
; A `token_tree` is a flat token list, not a parse of what it holds, so a
; capture per `identifier` inside it read `thiserror::Error` as the two traits
; `thiserror` and `Error`. Neither is one.
(attribute_item
  (attribute
    (identifier) @_derive
    arguments: (token_tree) @annotation.derive)
  (#eq? @_derive "derive"))
