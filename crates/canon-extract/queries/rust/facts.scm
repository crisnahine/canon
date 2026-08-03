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
