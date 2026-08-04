; TypeScript and TSX facts.
;
; The same patterns as JavaScript: TypeScript is a superset, and every node
; kind used here is one it inherits. Upstream ships these separately and
; expects them concatenated, which is why its tags query alone finds nothing
; in an ordinary class.
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

; @Injectable() / @Controller('x') / @Get() / @Body()
;
; Matched by the decorator node itself rather than anchored to what it
; decorates: `method_definition` carries no `decorator` field in this
; grammar, only `class_body`, `public_field_definition` and
; `required_parameter` do, so anchoring the way JavaScript does would compile
; and match nothing on a method.
(decorator
  [
    (identifier) @annotation
    (member_expression) @annotation
    (call_expression function: [(identifier) (member_expression)] @annotation)
  ])
