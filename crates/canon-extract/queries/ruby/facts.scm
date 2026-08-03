; Ruby facts.
;
; Visibility is deliberately absent: a bare `private` is a section keyword,
; which is accumulated state rather than a pattern, and lives in ruby.rs.

; Payment.charge(x) — a call with an explicit receiver.
(call
  receiver: (_) @call.receiver
  method: (identifier) @call)

; helper(1) — a call with no receiver.
;
; Only where the source makes it a call. A bare `notify_customer` with no
; parentheses and no arguments parses as a plain `identifier`, because Ruby
; cannot tell a method call from a local variable read without resolving
; scope. Capturing those would report every variable as a call and inflate
; every coupling count in the repository.
(call
  method: (identifier) @call
  arguments: (argument_list))

; raise ArgumentError, "..."
(call
  method: (identifier) @_raise
  arguments: (argument_list . [(constant) (scope_resolution)] @raise)
  (#eq? @_raise "raise"))

; require "json" / require_relative "foo"
(call
  method: (identifier) @_require
  arguments: (argument_list . (string) @import)
  (#any-of? @_require "require" "require_relative" "load"))
