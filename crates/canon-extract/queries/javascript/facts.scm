; JavaScript and JSX facts.

; Payment.charge(x)
(call_expression
  function: (member_expression
    object: (_) @call.receiver
    property: (property_identifier) @call))

; charge(x)
(call_expression
  function: (identifier) @call)

; throw new TypeError("...")
(throw_statement
  (new_expression
    constructor: (identifier) @raise))

(throw_statement
  (identifier) @raise)

; import a from "b" / export ... from "b"
(import_statement
  source: (string) @import)

(export_statement
  source: (string) @import)

; require("b"), still ubiquitous
(call_expression
  function: (identifier) @_require
  arguments: (arguments . (string) @import)
  (#eq? @_require "require"))

; @Injectable() / @Controller('x') on a class, a method or a field
(decorator
  [
    (identifier) @annotation
    (member_expression) @annotation
    (call_expression function: [(identifier) (member_expression)] @annotation)
  ])
