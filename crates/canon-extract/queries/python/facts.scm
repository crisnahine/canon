; Python facts.

; Payment.charge(x)
(call
  function: (attribute
    object: (_) @call.receiver
    attribute: (identifier) @call))

; charge(x)
(call
  function: (identifier) @call)

; raise ValueError("...")
(raise_statement
  (call function: (identifier) @raise))

(raise_statement
  (identifier) @raise)

; import json
(import_statement
  name: (dotted_name) @import)

; from a.b import c
(import_from_statement
  module_name: (dotted_name) @import)
