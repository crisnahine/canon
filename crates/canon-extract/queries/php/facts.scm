; PHP facts.

; Payment::charge(x)
(scoped_call_expression
  scope: (_) @call.receiver
  name: (name) @call)

; $payment->charge(x)
(member_call_expression
  object: (_) @call.receiver
  name: (name) @call)

; charge(x)
(function_call_expression
  function: (name) @call)

; throw new RuntimeException("...")
(throw_expression
  (object_creation_expression
    (name) @raise))

(namespace_use_clause
  (qualified_name) @import)
