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
base type, reading the name of its single public method, comparing a file name
against a style. Not "files here call `X`", where a new file may legitimately
not need that collaborator. Not a rule assembled from the rules of child
directories, which generalises to siblings that never voted.

Naming qualifies only because a style rule is withheld unless the sample has
actually witnessed the style: one name repeated is one observation, however
many files carry it. Ten files called `Cargo.toml` used to derive "files here
are named in `PascalCase`" at total agreement, and refuse an ordinary
`deny.toml`.

A refusal states the counts against the scope they were counted over, names
each rule by id, and prints the `suppress` line ready to paste, because
`.canon.toml` is keyed by id and a refusal that names the rule only in prose
leaves you guessing at the key. Suppressing takes effect on the next write, not
the next session. Everything else in this document is advisory and always will
be.

Which tool you reach for does not change the answer. `Edit` used to reach file
states `Write` refused, because the check read the field only `Write` carries;
the resulting file is now reconstructed from what is on disk, so the same bytes
get the same verdict either way.

Because a rule may only refuse when *every* file already agrees, no file that
exists in the repository can violate one. That is checkable rather than
reassuring: 20,718 tracked files from nine production repositories — a Rails
API, a TypeScript/React client, wagtail, pixelfed, hugo, NestJS, nuxt-ui,
starship and the FastAPI template — replayed through the write path, refused
none of them. 18,625 of those writes got a rule to check against, which is what
separates that result from a snapshot nobody loaded.

That is the weaker half of the check, because a file already in the index has
already voted. The half that finds things writes 12,324 *new* files into those
same directories — the content of one file at the name of its neighbour, a test
in each naming idiom into every directory holding an enforceable rule, and a
class rewritten to keep only the base its neighbours put last. None of those
are refused either, and three false positives were found that way and no other.

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
| Vue SFC | yes | **yes** | the `<script>` block, as TypeScript or JavaScript |
| ERB | yes | **yes** | the Ruby between the tags |

Those six visibility rules do not reduce to each other, which is why each
language resolves its own before the derivation layer ever sees it.

`canon check` prints this table for your install, read from the binary. The
README cannot drift from what is actually linked, because the binary reports
itself.

Vue and ERB are two languages in one file, handled by
`Parser::set_included_ranges`: the same buffer is parsed by the grammar of the
language canon has conventions about, restricted to the byte ranges that
language occupies. The markup outside those ranges is treated as absent, and
the tree keeps the original offsets so a reported line still points at the
right line of the file.

Templates yield fewer conventions than source files, because they declare
expressions rather than types and methods. The point is that they are no longer
invisible: 340 `.erb` files in the repository measured above now contribute to
the index and have their Ruby read.

### Rules that are not about classes

The first vocabulary was Rails-shaped. "Types here inherit from `X`" and "types
here expose exactly one public method" describe a service-object codebase
exactly, and describe a React component, a view template and a WordPress plugin
file not at all — so those trees derived almost nothing however many files they
held. Three rules cover what they actually agree on:

| Rule | Reads | Example |
|---|---|---|
| `format` | the segment between the name and the extension | Files here are named `*.html.erb` |
| `shape.export` | whether the module exports a default | Files here export a default |
| `shape.namespace` | the namespace the file declares | Files here declare namespace `App\Services\Billing` |

Measured on the same three checkouts: a 9,557-file Rails repository went from
one ERB rule to nine; a 3,189-file React repository gained 39 export-style
rules; a 698-file WordPress theme derived its first structural rule at all —
though that one is a vendored library's namespace rather than a convention the
team chose, which says as much about the theme as about the rule.

All three are advisory and none can refuse a write. A view tree that is
entirely `.html.erb` can still legitimately gain the one `.json.erb` an
endpoint needs, and a component directory can gain the one barrel that exports
no default.

### Rules about what a framework asks for

A base class is not how most frameworks say what a file is. A Sidekiq worker
declares no superclass and writes `include Sidekiq::Worker`; a Laravel job
writes `implements ShouldQueue`; a NestJS controller is a plain class with
`@Controller` on it; a Vue component declares nothing at all and calls
`defineProps`. Every one of those was invisible, so the directories holding
them derived nothing about the thing that makes them what they are.

| Rule | Reads | Example |
|---|---|---|
| `shape.annotation` | the decorator or attribute a file carries | Files here carry `@Controller` |
| `shape.macros` | a call with no receiver | Files here use `defineProps` |
| `shape.mixin` | a module or trait a type composes in | Types here include `Sidekiq::Worker` |
| `shape.contract` | an `implements` clause or a Rust trait impl | Types here implement `ShouldQueue` |
| `shape.family` | the suffix several namespaced bases share | Types here inherit from a `*BaseController` |

`shape.family` is the fallback for a directory that agrees on a kind of base
but not on one base: a Rails API namespaces its controllers, so 95 of 102 files
inherit something ending `BaseController` while the largest single spelling is
53, and the exact rule finds no winner at all. It is only stated where the
exact rule found nothing.

Measured, in conventions derived: the Rails API 146 to 262, `pixelfed` 122 to
142, `nest` 107 to 136, `wagtail` 74 to 101, `nuxt-ui` 34 to 46, `starship` 7
to 10. The individual answers matter more than the counts. `db/migrate` derives
`ActiveRecord::Migration` at 1519/1519 where six version-parameterised
spellings had agreed on nothing; `app/workers` derives `Sidekiq::Worker` at
485/490 where 485 files declared no base at all; pixelfed's `app/Jobs` derives
`ShouldQueue` at 119/119 from an `implements` clause the extractor had been
recording, and nothing had ever read.

All five are advisory. Each is a fact about what a directory mostly does, and
each has a legitimate exception: the one plain helper class beside the
decorated ones, the one worker that composes something else.

The same work fixed a live false refusal. Python's base list is positional and
its frameworks put mixins first, so reading the first entry made
`class OrderView(LoginRequiredMixin, ListView)` a subclass of
`LoginRequiredMixin`. Measured on a Django codebase, the first base ends in
`Mixin` in 337 of 754 declarations. The last positional base is the type the
class is; everything before it is composition, and lands in `shape.mixin`
instead. A base read from a language that allows several is advisory in any
case, for Rust and Python both.

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

An id is `<family>.<directory>.<extension>`, and `*` is allowed, so a whole
family goes quiet with one line. The families are `naming`, `format`,
`tests.suffix`, `tests.colocation`, `shape.public-arity`, `shape.entrypoint`,
`shape.base`, `shape.family`, `shape.mixin`, `shape.contract`,
`shape.annotation`, `shape.macros`, `shape.collaborator`, `shape.import`,
`shape.export`, `shape.module-arity` and `shape.namespace`.

```toml
suppress = ["shape.macros.*", "shape.annotation.src.orders.ts"]
```

## Configuration

Optional, and most repositories should never need it. Layered: defaults, then
`.canon.toml` at the repository root, then `CANON_*` environment variables.

| Key | Default | Environment | What it does |
|---|---|---|---|
| `enforce` | `true` | `CANON_ENFORCE` | whether a rule may refuse a write, or only advise |
| `injection_budget` | `3000` | `CANON_INJECTION_BUDGET` | bytes of convention text per write |
| `confidence_floor` | `0.8` | `CANON_CONFIDENCE_FLOOR` | agreement below this is not a convention; `0.8` to `1.0` |
| `min_files` | `5` | `CANON_MIN_FILES` | sample below this is coincidence |
| `recency_half_life_days` | `365` | `CANON_RECENCY_HALF_LIFE_DAYS` | how fast an old file loses its vote |
| `suppress` | `[]` | `CANON_SUPPRESS` | convention ids to silence, `*` allowed |
| `exclude_dirs` | 30 entries | — | directory names never scanned |
| `log_level` | `off` | `CANON_LOG` | `off`, `error`, `warn`, `info`, `debug`, `trace` |

`canon check` prints all of these as they are actually resolved, so what is in
effect is never inferred from a document.

Every one of these is read per invocation. Turning `enforce` off, or adding an
id to `suppress`, changes the next write — not the next session. It used to
change the next session, which made the escape hatch a refusal points at
useless at the moment you reach for it.

Two paths come from the environment only, because they are about where canon
keeps its own state rather than about how it derives: `CANON_DATA_DIR`
overrides everything, and `CLAUDE_PLUGIN_DATA` is what the host supplies to a
plugin hook. When neither is set — you, in a terminal — canon looks for the
installed plugin's data directory before falling back to the XDG one, so
`canon explain` audits the same snapshot the hook that refused you was reading.
`canon check` prints which directory it resolved.

The `confidence_floor` is a floor, not the bar. The bar rises with the size of
the sample: a rule over thirty files may hold four times in five, but the same
ratio over four thousand is eight hundred counterexamples, and that is what a
rule looks like when it has been derived from one kind of file and applied to
every kind.

`0.8` is the lowest value it accepts, because the sample-size bar starts there
and a lower setting could admit nothing. Raising it states strictly less: it is
read over the finished set, so the 9,557-file repository measured above derives
146 conventions at the default, 72 at `0.95` and 49 at `1.0`, and the rules that
may refuse a write never outnumber the default's.

An unknown key is a hard error, because a typo means a setting you think is
active never parsed. On the hook path a config that will not load runs on the
defaults *except* for `enforce`, which is forced off: the setting that can
block a write has to fail toward permissive, or a typo made while turning a
refusal off answers by refusing again. It is logged, and `canon check` is where
it fails loudly, because that is a command a human ran and is waiting on. A
file that exists and cannot be read at all — saved as UTF-16, or unreadable —
is reported the same way rather than treated as absent.

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
