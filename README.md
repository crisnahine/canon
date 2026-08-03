# canon

**Your repository already documents its own conventions. `canon` reads them.**

A Claude Code plugin, written in Rust, that derives how your codebase actually
writes code — from its AST, not from a style guide — and states the relevant
rules at the moment the agent is about to write a file.

No `CLAUDE.md`. No `.claude/rules/`. No configuration. You install it and it
works.

```
/plugin marketplace add crisnahine/canon
/plugin install canon@canon
```

---

## The problem

You join a team with a nine-thousand-file repository. You ask Claude to add a
feature. It reads five files, infers a pattern that half-exists, and writes
something that would pass review in no other codebase on earth. You correct it.
Next session, same thing.

The standard fix is to write the conventions down in `CLAUDE.md`. That works,
and it has two costs: somebody has to write them, and they start drifting from
the code the moment they're written.

## What it actually does

At session start it reads the files git tracks, parses them with tree-sitter,
and counts structural facts. Then, immediately before Claude writes
`app/services/billing/refund_payment.rb`, it says this:

```
Conventions for app/services/**/*.rb, derived from this repository:

- That public method is named `execute`. (1241/1248, 0.99)
- Types here inherit from `ApplicationService`. (1268/1550, 0.82)
- Types here expose exactly 1 public method. (1248/1550, 0.80)
- Files here call `Ledger`. (9/10, 0.90)
- Files here are named in snake_case. (2399/2399, 1.00)
- Test files are named `*_spec.rb`. (1321/1462, 0.90)

Canonical example, most recently modified: app/services/billing/settle_batch.rb
```

Those counts are real, measured on a 9,545-file Rails repository. The class
and file names are changed, because that repository is not mine to publish; the
numbers, the confidences and the shape of the block are exactly what it
produced. Nothing about the frontend, nothing about migrations: only what
applies to this file, right now.

After the write it re-parses the result and structurally diffs it against those
same conventions. A difference comes back as specific feedback, so it gets
fixed before the turn ends rather than at review time.

That fourth line is a layering rule: who this directory talks to. It is the
kind of thing no linter checks and every team holds, and a service that reaches
past its collaborators is wrong in a way that compiles and passes tests.

## Why the numbers are in the text

`(1241/1248, 0.99)` is not decoration. It is what makes the block read as
evidence rather than policy.

Your prompt is the instruction. If the injected block reads like a competing
instruction it will sometimes win, and the tool will start overriding what you
actually asked for. Stating a count invites the model to follow the house style
and leaves you in charge. There is a test asserting the block never contains
"You must", "Always", or "Never".

## Why hooks, not a skill or a rules file

Three reasons, in increasing order of importance:

1. **Plugins can't ship a `CLAUDE.md`.** A `CLAUDE.md` at a plugin root is not
   loaded as project context. Hooks aren't a workaround; they're the mechanism.

2. **Path-scoped rules fire on read; `PreToolUse` fires on write.** The hook
   receives the target path *and* the content about to be written, which is
   what makes after-the-fact verification possible at all.

3. **Hooks fire inside subagents.** This is the one that matters. Plan-execution
   workflows spawn a fresh subagent per task, each starting with an empty
   context window. Skills and conversation memory don't propagate into them, so
   every worker reinvents the house style independently: seven tasks, seven
   different shapes. `SubagentStart` fires in every one of them.

## What it deliberately does not do

- **It almost never refuses a write, but it can.** Everything advises, with one
  exception: a rule the repository holds *without a single exception*, checked
  by a test that cannot be wrong about a legitimate file. Not 51 of 52 — every
  file. On by default; set `enforce = false` to make everything advisory. See
  "When it refuses" below.
- **It doesn't replace your linter.** RuboCop and ESLint encode conventions
  someone already wrote down. `canon` covers the ones nobody did: architectural
  shape, naming topology, layering, canonical examples.
- **It doesn't write to your repository.** Everything lives in the plugin's own
  data directory, keyed by a hash of the repository root.
- **It doesn't touch the network** after the one-time binary install.
- **It derives shape, never correctness.** It can tell you code resembles your
  other code. It cannot tell you the code is right.

## A folder of repositories

A workspace root is often not a repository itself:

```
workspace/          <- you open the editor here; not a git repo
├── api/                  <- its own checkout, Rails
├── client/               <- its own checkout, TypeScript/React
└── wordpress/            <- its own checkout, PHP
```

canon asks the children instead, and prefixes their files with their directory
name. Rules come out scoped per checkout, so Ruby conventions never reach the
frontend:

```
api/app/services/**/*.rb      that public method is named `execute`
client/src/hooks/**/*.ts      files here export exactly 1 function
```

One level down, up to 32 children. The combined commit string is every child's
HEAD, so a commit in any one of them refreshes the snapshot.

This is not a nicety. On the layout above, walking the filesystem instead did
not finish indexing inside a minute, because one child held a 24 GB tool cache.
Asking git takes 3.4 seconds for 13,456 files.

## When it refuses

Advisory context is ignorable, and this was measured rather than assumed. Four
channels reach the model, and three of them the model may simply decline:

| Channel | What happens |
|---|---|
| context before the write | steers it, and it can be ignored |
| a block after the write | the reason is delivered; the turn ended anyway |
| a block on the turn ending | the turn is genuinely held open, and the edit still may not come |
| **refusing the write** | the file never lands |

Only the last does not depend on the model cooperating, so it is the only one
canon uses for a rule it is certain of. It is on by default: a tool that only
advises is a tool that gets followed when convenient, and three of the four
channels above were measured being declined.

The safety is in what qualifies, not in whether the switch is thrown. Two
conditions both have to hold:

**Total agreement.** Every file, not most. One counterexample already in the
tree means the rule has an exception nobody wrote down, and refusing a write
that matches an existing file is the fastest way to get a tool uninstalled.

**A check that cannot be wrong.** Counting a type's public methods, reading its
base type, reading the name of its single public method. Not naming style,
where a single-word name is compatible with three styles at once. Not "files
here call `X`", where a new file may legitimately not need that collaborator.

A refusal states the counts and points at `canon explain`, so you can see the
files it was derived from and suppress it in `.canon.toml` if it is wrong.
Everything else in this document is advisory and always will be.

Because a rule may only refuse when *every* file already agrees, no file that
exists in the repository can violate one. That is checkable rather than
reassuring: run against 400 tracked Ruby files in a production repository,
enforcement refused none of them.

```toml
# .canon.toml — if you would rather never be interrupted
enforce = false
```

## Supported languages

Two tiers, and the difference matters.

**Tier 0 needs no grammar.** Naming style, test layout, and canonical
exemplars. Works on any text repository in any language, today.

**Tier 1 needs a parser.** Public surface shape, entrypoint naming, base
classes. This is where "types here expose exactly one public method, named
`execute`" comes from, and it cannot be derived without per-language visibility
rules.

| Language | Tier 0 | Tier 1 | How it decides what is public |
|---|---|---|---|
| Ruby | yes | **yes** | a bare `private` is a section keyword |
| JavaScript | yes | **yes** | `export`, and `#` for members |
| JSX | yes | **yes** | as JavaScript |
| TypeScript | yes | **yes** | an `accessibility_modifier`, or `#` |
| TSX | yes | **yes** | as TypeScript |
| Python | yes | **yes** | a leading underscore, by convention |
| Go | yes | **yes** | the case of the first letter |
| Rust | yes | **yes** | the `pub` keyword |
| PHP | yes | **yes** | a modifier, public when absent |
| Vue SFC | yes | no | needs two-pass extraction |

Those six visibility rules do not reduce to each other, which is why each
language resolves its own before the derivation layer ever sees it.

`canon check` prints this table for your install, read from the binary. The
README cannot drift from what is actually linked, because the binary reports
itself.

Vue is deliberately blank: `<script lang="ts">` and `<template>` parse under
different grammars, so it needs two passes rather than one. Left unwired rather
than half-wired.

Adding a language is one module in `crates/canon-extract/src/`, one arm in
`lang::provider`, and one `queries/<language>/facts.scm`. The match is
exhaustive, so it will not compile until the capability table is updated.

## Facts by query

Two kinds of fact, extracted two ways.

**Patterns are queries.** Call sites, raises and imports look the same in every
file of a language, so they are written once in
`crates/canon-extract/queries/<language>/facts.scm` and matched by tree-sitter.
Adding a fact to a language is editing an `.scm` file.

```scheme
; Payment.charge(x) — a call with an explicit receiver.
(call
  receiver: (_) @call.receiver
  method: (identifier) @call)
```

**State is Rust.** Ruby's `private` is a section keyword that flips everything
after it, and Go's methods are declared outside the type they belong to.
Neither is a pattern, and a query cannot express either. Those stay in a cursor
walk, which is also why the first attempt at Ruby visibility as a query
produced a confident and wrong convention.

The capture vocabulary is canon's rather than upstream's. Every grammar crate
exports a `TAGS_QUERY`, and they disagree: Go's reports `@reference.type` where
Ruby's reports `@reference.call`, and TypeScript's covers only the constructs
TypeScript adds, on the assumption the JavaScript query is concatenated with it.
Pointed at an ordinary class, the TypeScript tags query returns nothing.

## Commands

Hooks call these; you generally won't.

```
canon session-start     # refresh the snapshot, state what the repository is
canon subagent-start    # state the same thing to a worker with empty context
canon inject            # PreToolUse: conventions for the target path
canon verify            # PostToolUse: structural diff against them
canon reconcile         # Stop: cross-file duplication over what was touched
```

For people:

```
canon check                     # live config, language table, snapshot state
canon index --rebuild           # force a rebuild
canon explain app/services/     # every convention for a path, with evidence
canon explain --id shape.base.app.services.rb
```

`canon explain` is the audit surface. If the engine derived something wrong,
this is where you see the files it counted and disagree with it. Then:

```toml
# .canon.toml
suppress = ["shape.base.app.services.rb"]
```

## Configuration

Optional, and most repositories should never need it. Layered: defaults, then
`.canon.toml` at the repository root, then `CANON_*` environment variables.

| Key | Default | Environment | What it does |
|---|---|---|---|
| `enforce` | `true` | `CANON_ENFORCE` | whether a rule may refuse a write, or only advise |
| `injection_budget` | `3000` | `CANON_INJECTION_BUDGET` | bytes of convention text per write |
| `confidence_floor` | `0.8` | `CANON_CONFIDENCE_FLOOR` | agreement below this is not a convention |
| `min_files` | `5` | `CANON_MIN_FILES` | sample below this is coincidence |
| `recency_half_life_days` | `365` | `CANON_RECENCY_HALF_LIFE_DAYS` | how fast an old file loses its vote |
| `suppress` | `[]` | `CANON_SUPPRESS` | convention ids to silence, `*` allowed |
| `exclude_dirs` | 29 entries | — | directory names never scanned |
| `log_level` | `off` | `CANON_LOG` | `off`, `error`, `warn`, `info`, `debug`, `trace` |

`canon check` prints all of these as they are actually resolved, so what is in
effect is never inferred from a document.

Two paths come from the environment only, because they are about where canon
keeps its own state rather than about how it derives: `CANON_DATA_DIR`
overrides everything, and `CLAUDE_PLUGIN_DATA` is what the host supplies to a
plugin hook.

The `confidence_floor` is a floor, not the bar. The bar rises with the size of
the sample: a rule over thirty files may hold four times in five, but the same
ratio over four thousand is eight hundred counterexamples, and that is what a
rule looks like when it has been derived from one kind of file and applied to
every kind.

An unknown key is a hard error, because a typo means a setting you think is
active never parsed. On the hook path an invalid config degrades to defaults
and is logged; `canon check` is where it fails loudly, because that is a
command a human ran and is waiting on.

Logging is off by default and never goes to a stream. stdout is the hook
protocol channel and stderr on `PostToolUse` is fed to the model, so a debug
line there would appear inside your conversation looking like considered
feedback about your code.

## Development

```
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets
./tests/fail-open.sh ./target/release/canon
./tests/injection-reaches-the-model.sh
```

Installing your own build:

```
claude plugin marketplace add /path/to/canon
claude plugin install canon@canon
./bin/install-local          # after install, not before
```

The order matters. Uninstalling a plugin clears its data directory, so a
binary placed there first is deleted. Two other things are worth knowing:
installing from a local directory copies the tree without honouring
`.gitignore`, so build there and `target/` follows you into the plugin cache
(1.6 GB, measured); and the released install path downloads the binary from
the matching GitHub release, which is why a local build needs somewhere else
to live.

The last two are the ones that matter.

`fail-open.sh` feeds every hook subcommand malformed, truncated, hostile and
oversized payloads and asserts one contract: **exit 0, nothing on stderr, and
stdout empty or valid JSON.** It has already caught a stack overflow that
aborted the process, which is the one failure mode a fail-open harness inside
the program cannot disguise.

`injection-reaches-the-model.sh` checks the assumption the whole design rests
on, against the installed host, by injecting a convention the request never
mentions and asserting the first `Write` carries it.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the crate boundaries and
the reasoning behind them, and [`STATUS.md`](STATUS.md) for measured results
including where it currently falls short.

## License

MIT OR Apache-2.0
