# Architecture

Five crates. The boundaries exist to keep three properties true, and each one
has been load-bearing at least once.

```
canon-core      types + confidence arithmetic       no I/O at all
canon-extract   tree-sitter, one module per language depends on: core
canon-derive    walk, rules, snapshot, verify, dup   depends on: core, extract
canon-hook      the host protocol + fail-open        depends on: core
canon-cli       the binary: config, logging, git     depends on: all
```

## The three properties

**1. Derivation rules are testable without a repository, a grammar, or a
process.** `canon-core` performs no I/O, so every confidence and scope decision
is a unit test over plain values. `canon-derive` never spawns a subprocess, so
its rules run in CI without git on `PATH`.

**2. Adding a language cannot reach the derivation rules.** A new language is
one module in `canon-extract` plus one arm in `lang::provider`. If a language
ever required changing how conventions are derived, the abstraction would be
wrong, and the compiler would say so: `provider` matches exhaustively, so a new
variant fails to build until the capability table is updated.

**3. Only one function in the workspace writes to stdout.** `print_stdout` and
`print_stderr` are denied workspace-wide with two audited exceptions:
`canon_hook::write_line` for the protocol, and `canon_cli::emit` for the
commands humans run. A stray `println!` on a hook path does not compile.

## Why visibility lives in the extraction layer

"Types here expose exactly one public method" is the convention teams actually
hold. *Public* means something different in every language:

| Language | Rule |
|---|---|
| Ruby | a bare `private` is a section keyword that flips everything after it |
| Go | the case of the first letter |
| TypeScript | an `accessibility_modifier`, or a `#` prefix |
| Python | a leading underscore, enforced by nobody |
| Rust | the `pub` keyword |
| PHP | a modifier, public when absent |

None reduce to another. Pushing them up into the derivation layer would put six
special cases in the one place that has to stay language-agnostic. Each
extractor resolves visibility itself and reports two flat lists, and everything
above works on those.

Ruby's is the interesting one. Against the real AST a bare `private` is an
`identifier` inside `body_statement`, not a `call` as it reads, which is why
that extractor is a positional cursor walk rather than a tree-sitter query.
Guessing at the query form fails quietly: it finds zero private methods, reports
every helper as public, and inflates the arity of every class in the repository.

## The cost model

Deriving is expensive. Selecting is cheap. They are separate functions over a
persisted snapshot for exactly that reason.

```
SessionStart  →  git ls-files → parse → derive → write snapshot     seconds
PreToolUse    →  read snapshot → filter → render                    milliseconds
```

On a 9,545-file repository that is 2.4 s once and 2.6 ms before every write. The
hot path never walks the tree, never parses a file, and never spawns a process.
The snapshot is 52 KB.

The snapshot is keyed on the commit SHA plus a fingerprint of the settings, and
expires after a day regardless. The SHA is the primary key because conventions
barely move between commits; the expiry exists because a long-lived branch
accumulates uncommitted work until the snapshot stops describing the tree.

## Why git decides what a file is

`canon-derive` can walk the filesystem, and that is the fallback. The primary
source is `git ls-files`, because a working tree contains build output, caches,
vendored dependencies and someone's scratch directory, and none of those are
anybody's convention.

This is not a tuning detail. Pointed at a real Rails repository the walk found
498,419 files where git tracks 5,445, because one tool's cache directory held
909,661 of them. Indexing took 1 minute 47 seconds and derived conventions from
files nobody wrote.

A repository with no commits yet answers `git ls-files` successfully and says
nothing. That is treated as "git has no opinion" and falls back to the walk,
rather than as "this repository is empty", which would silently produce no
conventions and no explanation.

## The fail-open contract

Every hook exits 0 and writes either nothing or one valid JSON document to
stdout. There is no input and no internal failure that produces any other
outcome.

Fail-open happens at four separate points, because each has a different cause
and all four have been observed: stdin unreadable, payload not JSON, payload
valid JSON of an unexpected shape, handler panicking. The last is why the
release profile unwinds rather than aborts — a panic is caught and still emits
`{}`.

There is one failure this cannot cover: a stack overflow aborts the process
before any handler runs. That is why `tests/fail-open.sh` exists as an external
harness rather than as a unit test, and it is exactly what it caught.

Oversized output is replaced with `{}` rather than truncated. The host truncates
what it cannot fit, and a truncated JSON document reads as a crash on the far
end. Saying nothing is better than saying half a thing.

## Which channel actually reaches the model

Measured against the running host, not assumed:

| Event | What canon sends | When it lands |
|---|---|---|
| `SessionStart` | manifest | once, before any work |
| `SubagentStart` | manifest | every subagent, own context window |
| `PreToolUse` | conventions for the target | before the tool executes |
| `PostToolUse` | differences from those conventions | after the write |
| `Stop` | duplication across what was touched | end of turn |

`PreToolUse` carrying `additionalContext` is load-bearing and is the one the
host's documented field list does not promise. See `STATUS.md` for how it was
confirmed and `tests/injection-reaches-the-model.sh` for the check that keeps
it honest.

`SubagentStart` is the reason this is a plugin with hooks rather than a skill. A
subagent begins with an empty context window, so nothing said in the
conversation reaches it. Seven parallel workers otherwise invent seven house
styles.

## Dependency policy

Four dependency trees, all load-bearing: `serde`, `serde_json`, `toml`, and
tree-sitter with its grammars. Nothing else.

Argument parsing is eighty hand-written lines because the surface is eight
subcommands and three flags, fully specified by the hook configuration this
binary ships with. Error types are hand-written enums because the project wants
concrete errors per crate rather than a derive macro's. Logging is eighty lines
because the requirement — off by default, file only, never a stream — is
smaller than any logging framework's configuration surface.

This is a bet that a binary with four dependencies still builds in a decade.

## Where the rules live

| File | Rule |
|---|---|
| `canon-core/confidence.rs` | when agreement becomes a convention |
| `canon-derive/naming.rs` | why a single-word file name proves nothing |
| `canon-derive/tier0.rs` | naming and test layout, no grammar needed |
| `canon-derive/semantic.rs` | shape rules, and which type a file is *about* |
| `canon-derive/dup.rs` | near-duplicate detection by line shingling |
| `canon-derive/select.rs` | what to say inside a fixed budget |
| `canon-derive/render.rs` | why the block states evidence, not policy |
| `canon-derive/verify.rs` | comparing what was written against the rules |
