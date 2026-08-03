; Go facts.

; payment.Charge(x)
(call_expression
  function: (selector_expression
    operand: (_) @call.receiver
    field: (field_identifier) @call))

; Charge(x)
(call_expression
  function: (identifier) @call)

; Go has no exceptions; panic is the nearest thing and is worth knowing about.
(call_expression
  function: (identifier) @raise
  (#eq? @raise "panic"))

(import_spec
  path: (interpreted_string_literal) @import)
