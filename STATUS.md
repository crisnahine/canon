# Measured status — v0.5.0

Every number below was executed. Reproduce with the commands shown.

## Gates

```
cargo build --release                      0 errors
cargo test --workspace                     514 passing
cargo clippy --workspace --all-targets     0 warnings
cargo fmt --all --check                    clean
./tests/fail-open.sh                       75/75
./tests/asset-coverage.sh                  8 of 8 platforms
./tests/injection-reaches-the-model.sh     PASS
tests/refusal-regressions.py               78/78
tests/issue-regressions.py                 35/35
tests/replay-tracked-files.py              20,718 replayed, 0 refused
tests/replay-new-files.py                  12,324 written,  0 refused
tests/naming-already-used.py               0 names refused that the sample used
```

The last five need fixtures and the corpus:

```
tests/fixtures-for-issue-regressions.sh /tmp/canon-fixtures
python3 tests/issue-regressions.py    ./target/release/canon /tmp/canon-fixtures
python3 tests/refusal-regressions.py  ./target/release/canon

tests/clone-corpus.sh /tmp/canon-corpus/realrepos
python3 tests/replay-tracked-files.py ./target/release/canon /tmp/canon-corpus
python3 tests/replay-new-files.py     ./target/release/canon /tmp/canon-corpus
python3 tests/naming-already-used.py  ./target/release/canon /tmp/canon-corpus
```

18,325 lines of Rust across five crates, of which roughly half are tests,
plus 258 lines of tree-sitter query across seven languages.

## What this branch moved

Two binaries, one built at the branch point and one at its tip, run against the
same five checkouts at the same commit with a clean tree:

| Repository | Tracked | Conventions | Rules that may refuse |
|---|---|---|---|
| Rails API | 9,588 | 146 to 264 | 26 to 39 |
| TypeScript/React client | 3,193 | 93 to 166 | 12 to 8 |
| wagtail | 5,210 | 74 to 111 | 34 to 25 |
| FastAPI template | 238 | 10 to 17 | 6 to 6 |
| starship | 814 | 7 to 9 | 4 to 4 |

Most of the gain is vocabulary that did not exist before and refuses nothing:
54 macro rules and 23 mixin rules on the Rails API, 23 macro rules on the React
client, 14 on wagtail, the first `shape.contract` rule on starship. Grouping
eight directories deep rather than four is the rest of it, and that one does
reach enforcement: a narrower directory reaches total agreement where its
parent did not, which is where the Rails API's extra thirteen refusing rules
come from.

On two of the five the columns move in opposite directions — more said, less
refused — and each fall is a rule this branch decided not to enforce.

- wagtail's base rules go from 10 refusing to 1. A Python class lists several
  bases and which one is "the" base is a reading, not a fact, so a Python base
  rule is graded Advisory.
- The React client's naming rules go from 12 refusing to 8 while the family
  triples in size, because a rule may now only refuse inside the directories
  its sample actually covered. Mid-branch that number was 54: deeper grouping
  produced the rules first and the sample gate arrived after it.

starship and the FastAPI template refuse on the same count as before, and the
Rails API is the only one that refuses on more.

## The numbers this document used to carry were measured against nothing

Every replay figure in the version of this file before this one is void, and
the reason is worth stating rather than quietly correcting.

Both replay harnesses ran `canon inject` with no data directory of their own
and no `sys.exit`. With no snapshot, `inject` answers `{}` for every case, so
each harness loaded zero rules, saw zero refusals, and reported a clean sweep
it could not have failed. The counts they printed were counts of files opened.

That is why both now count how many replayed cases actually received a context
block, and fail when none did. A harness that cannot tell "no rule fired" from
"no rule exists" is not evidence, and this one reported success from the second
state for as long as it existed.

## What a pre-push review found, and why the harnesses missed it

Six reviewers over the outgoing diff, every finding then handed to a separate
agent whose job was to refute it. Fifteen survived, eight of them critical, and
one root cause accounts for three: a guard written for `_form.html.erb` was
written for a leading underscore rather than for the class of name it belongs
to. `[id].tsx`, `[...slug].tsx`, `+page.server.ts` — every file-based router's
own file names were refused, and the author cannot rename them.

Both replay harnesses are structurally blind to that. The existing-file replay
cannot see it, because a repository that already contains a route file has
already had the rule silenced by it. The new-file harness writes a neighbour's
name and a test-idiom name, so it never generates a `[` or a `+`. A harness
finds the shapes it was built to generate, and nothing else — which is the
argument for reading the diff as well as running it.

The other criticals were of a piece: a rule refusing a directory or a file
qualifier its sample never covered, a file too large for the index refused by
rules it never voted on, and two ways a Rust `impl` block was attributed to the
wrong type.

Every one has a check in a harness of 50 assertions, and each was run against
the binary from before its fix to confirm it actually fails there. A regression
test that passes on the code it was written to catch is worth nothing, and
three of them were exactly that when first written — one saved by a stem
name-match that short-circuits the path it tested, one by a fixture that
derived no rule of the relevant kind, one by a fixture using the single
directory layout its guard happened to cover.

Three rounds, and the shape of them is the finding worth recording. Round one
reviewed the original work: eight criticals. Round two was told round one's
ground was covered and found seven more, **four of them inside round one's own
fixes**. Round three found twelve more, **four of them inside round two's**.

The recurring failure is not carelessness, it is that a fix written against one
reproduction covers that reproduction. Round one's guard used the top-level
directory of a rule's sample, which fixed `.github/PULL_REQUEST_TEMPLATE.md`
and left `src/pages/` refused by a rule counted in `src/components/`. Round
two's exemption for framework-named files was applied when checking and not
when deriving, so one `docs/FAQ.md` deleted every markdown naming rule instead.
Both had passing tests.

Round three also reached past the fixes into the extractors and found six
defects that predate all of this: Ruby's `def self.call` invisible, so a
class-method service was refused for exposing nothing; Ruby's `::Base` refused
by a rule naming the same constant; Python's `base.BaseService` and
`BaseService[Order]` read as no base at all; Rust recording whichever trait
`impl` came first as the base, so a refusal depended on block order; the
subject of every `PascalCase`-named file resolved by surface rather than by
name, because the stem was compared unnormalised; and `shape.public-arity`
refusing the `up`/`down` pair Rails requires for an irreversible migration.

## Every open issue, reproduced rather than read

Eight issues were open against this repository. Each was triaged by running the
build, not by reading it: two had already been closed by the work above, six
had not, and adversarial re-checking of the two "fixed" verdicts found a hole
in one of them that the issue had not reported — an unreadable `.canon.toml`
took the absent-file path and silently kept enforcement on.

All eight are now covered by a harness of 23 assertions that runs against any
build. Every one of them fails on the build before this and passes on it.

The most valuable was #9. Enforcement read the `Write`-only content field, so
`Edit` reached the same file states `Write` refused — the same path, the same
resulting bytes, opposite outcomes. Verified end to end against the running
host: asked to rename a method in a way that breaks a rule holding in 6 of 6
files, the `Edit` was refused, and the model then took the suppression path the
refusal printed and said what it cost.

## Fourteen public repositories, and what they found

The gates above were green before any of this. Every defect in the 0.4.1
changelog was found by running the release binary against real code: Mastodon,
Laravel, RuboCop, Nuxt, Vue, Redux Toolkit, ripgrep, Flask, requests, gin,
cobra, Slim, Sinatra and axios, plus twenty-four purpose-shaped repositories
covering the idioms each supported language actually uses.

Languages are the ones the conventions actually came from, which is why the
column is shorter than the list of languages present: Laravel tracks
JavaScript and RuboCop tracks ERB, and neither contributed a rule.

| | files | languages | conventions | index |
|---|---|---|---|---|
| Mastodon | 9,870 | ERB, JSX, JS, Ruby, TS | 104 | 1.6 s |
| Laravel | 3,338 | PHP | 127 | 2.4 s |
| RuboCop | 2,205 | Ruby | 39 | 1.2 s |
| Nuxt | 1,873 | TS, Vue | 32 | 0.5 s |
| Redux Toolkit | 1,149 | JS, TSX, TS | 19 | 0.4 s |
| Vue | 704 | JS, TSX, TS, Vue | 29 | 0.6 s |
| axios | 460 | JS, TS | 11 | 0.2 s |
| ripgrep | 236 | Rust | 5 | 0.2 s |

Three classes of defect came out that a fixture cannot produce. A naming rule
derived from one name repeated ten times, enforced, refusing an ordinary file.
Extractors blind to the idioms real code is written in — a decorated Python
class, a generic Go or Rust type, a type inside an inline `mod`. And a hot path
paying 25 ms per write to compile a tree-sitter query whose results the check
then discarded.

The one that mattered most was none of those. `enforce = false` did nothing
until the next session, because the decision was baked into the snapshot. The
escape hatch a refusal points at was inert at the only moment anyone reaches
for it.

## Nothing legitimate is refused, at 20,718 files and 12,324 new ones

The safety claim behind enforcement is that a rule may only refuse when every
file in scope already agrees, so nothing already in the tree can break one.

The corpus is nine checkouts, 28,388 tracked files, and it derives 1,000
conventions of which 188 may refuse:

| | tracked | conventions | may refuse |
|---|---|---|---|
| rails-api | 9,567 | 264 | 39 |
| py-wagtail | 5,203 | 111 | 25 |
| client | 3,189 | 166 | 8 |
| pixelfed | 2,603 | 150 | 50 |
| hugo | 2,539 | 67 | 31 |
| nest | 2,128 | 146 | 12 |
| nuxt-ui | 2,117 | 70 | 13 |
| starship | 808 | 9 | 4 |
| fastapi-app | 234 | 17 | 6 |

Measured rather than argued. Every tracked file of a supported extension was
replayed through the write path, as though the model had just written it. The
middle column is the one that separates evidence from an empty snapshot: how
many of those writes canon actually had something to say about.

```
                files   with a block   refused
rails-api       6,384          5,722         0
py-wagtail      2,796          2,481         0
client          2,688          2,536         0
pixelfed        2,231          1,845         0
hugo            2,119          1,786         0
nest            2,031          1,964         0
nuxt-ui         1,541          1,454         0
starship          737            710         0
fastapi-app       191            127         0
TOTAL          20,718         18,625         0
```

That result is weaker than it sounds, and it is worth being precise about why.
Every file in the index has already voted, so a rule that refused one of them
could not have reached total agreement in the first place. The replay confirms
the invariant holds; it cannot find the case where a refusal actually happens,
which is a file that does not exist yet.

So the second harness writes *new* files into real directories. The content of
one tracked file at the path of another in the same directory — both the name
and the shape idiomatic for that directory — a test file in each of the common
naming idioms, into every directory that has an enforceable shape rule, and a
mutant: a tracked file's type header rewritten to keep only its last base, so a
directory where every class lists a mixin first yields a file that inherits the
same real base without the mixin.

```
12,324 new files   11,673 with a block   0 refused   0 errors
```

29 of those are mutants, all in the two languages where a declaration may name
several bases at once: 20 in wagtail's Python and 9 in hugo's Go. None refused.

A third harness asks the narrowest form of the same question: for every naming
rule that may refuse, does it refuse a name the repository already uses? Across
the corpus, no rule refuses a name used inside its own scope.

Three false positives were found by the new-file harness and by nothing else.
Two Rust files that satisfy the arity rule they were refused by — two modules
declaring the same type name, and a type implemented from two `#[cfg]`-gated
modules — and the first test file written into any directory of service
objects, which was told it must inherit `BaseService` and expose one public
method.

Deliberate violations are still caught, through the same code path: a Laravel
console command whose entrypoint is `run` rather than `handle`, a RuboCop cop
exposing `check` rather than `on_send`, a `newBinding.go` in a repository of 58
unanimous `snake_case` names.

## The whole loop, against the running host

Not the hook in isolation — the plugin installed, in a real session, in a
six-file Python repository canon had never seen.

Asked for "a service in `app/services/` that voids a subscription", with
nothing said about house style, the first write was:

```python
from app.services.base import BaseService


class VoidSubscription(BaseService):
    def execute(self, payload):
        return self._run(payload)

    def _run(self, payload):
        return payload
```

All five derived conventions, on the first attempt.

Asked for one that broke three of them on purpose, the file never landed. The
refusal named each rule with its counts and its id and printed the `suppress`
line. Told to use whatever escape hatch the tooling offered, the model wrote
that line into `.canon.toml` and the retry succeeded — in the same session,
which is the fix that release is mostly about.

## The assumption everything rests on, and how it was checked

canon delivers conventions from a `PreToolUse` hook. That is only useful if
`hookSpecificOutput.additionalContext` on `PreToolUse` reaches the model *and*
arrives before the tool runs.

The host's own field list does not promise it. `permissionDecision` and
`updatedInput` are documented as `PreToolUse`-only; `additionalContext` is not
documented as belonging to `PreToolUse` at all. One reference implementation
emits it anyway.

So it was measured rather than assumed. A hook injected a convention the
request never mentioned, with `Edit` withheld so the model could not write the
file and fix it afterwards. The first `Write` carried the injected header.

`tests/injection-reaches-the-model.sh` re-runs that against the installed host,
because if it ever stops being true the failure is completely silent.

## Against a real repository

Not a fixture. Two production repositories, 9,545 and 3,189 tracked files.

| | files | conventions | index time |
|---|---|---|---|
| Rails API | 9,545 | 19 | 2.4 s |
| TypeScript/React client | 3,189 | 53 | 0.6 s |

Hot path, the one that runs before every write, on the 9,545-file repository:

```
inject   median 2.6 ms   p95 3.6 ms   max 6.1 ms      (budget 50 ms)
```

One file read, a filter over the snapshot, and a parse of the content about to
be written. No tree walk and no subprocess.

That parse is the only expensive thing on the path, and it was five times the
cost of everything else put together until 0.4.1. Measured again on a
9,870-file Rails repository:

```
inject   median 2.6 ms   p95 4.2 ms       (was 27.3 ms)
```

A tree-sitter query is compiled once per process and a hook is one process per
write, so 25 ms of Ruby query compilation landed on every single write — to
produce call and raise facts that the check does not read. The write path
extracts structure only. Nothing else changed.

What it derived for the Rails repository. Counts and confidences are as
measured; class and file names are changed, because that repository is private:

```
app/services       inherit from `ApplicationService`  (1268/1550, 0.82)
app/services       expose exactly 1 public method          (1248/1550, 0.80)
app/services       that method is named `execute`          (1241/1248, 0.99)
app/workers        that method is named `perform`          (481/481,   1.00)
app/controllers/api/v1  inherit from `BaseController`
db/                that method is named `change`
repository-wide    test files are named `*_spec.rb`        (1321/1462, 0.90)
```

Those are correct, and none of them are written down anywhere in that
repository. That is the point: they were derived, not read.

## Four defects the real repositories found that fixtures did not

Each of these passed a green test suite and was wrong in production.

**1. The walk found 498,419 files where git tracks 5,445.** A single tool's
cache directory held 909,661 of them. Indexing took 1 minute 47 seconds. No
hand-maintained exclude list keeps up with a working tree; the ignore rules the
team already wrote do. Now the file list comes from `git ls-files`, falling
back to the walk when there is no git. Indexing went to 2.4 s.

**2. `class Billing::ChargeCard` was invisible to the Ruby extractor.** A
compound constant parses as `scope_resolution`, not `constant`, and the
extractor matched on node kind. So it skipped the real class in every namespaced
service and captured only the small `class ChargeDeclinedError <
StandardError` nested inside, then reported with 0.95 confidence that services
inherit from `StandardError`. It now reads the `name` and `superclass` fields
instead of guessing at node kinds.

**3. Auxiliary types outvoted the subject.** Even with the name fixed, counting
every declared type lets error classes and prop interfaces outnumber the class
the file is about. Shape rules now resolve the file's primary type: the one
whose name matches the file name, else the one with the largest surface.

**4. Repository-wide shape rules were nonsense.** Derived across a whole Rails
codebase, migrations outnumber everything else and produce "the public method
here is named `change`" for every Ruby file in the project. Shape is now a
property of a directory, never of a repository.

A fifth was found by `tests/fail-open.sh` rather than by the repository: the
Ruby extractor walked the tree recursively and overflowed the stack on a real
file, aborting the process with SIGABRT. That is the one failure an in-process
fail-open harness cannot disguise, because the process dies before it can print
anything. The walk is now iterative, with a regression test at 5,000 levels.

## A folder of repositories

Measured on a workspace root holding four separate checkouts, itself not a
repository:

| | before | after |
|---|---|---|
| index | did not finish in 60 s | 3.4 s |
| files | walking a 24 GB tool cache | 13,456 tracked |
| languages | — | JSX, JavaScript, PHP, Python, Ruby, TSX, TypeScript |

Conventions come out scoped per checkout, so nothing leaks sideways:

```
api/app/services/**/*.rb   that public method is named `execute`  (1241/1248, 0.99)
api/app/workers/**/*.rb    that public method is named `perform`
client/src/hooks/**/*.ts   files here export exactly 1 function   (18/21, 0.86)
```

The PHP checkout is silent, correctly. It is a WordPress theme: 133 tracked
PHP files and no classes at all, so there is no structural shape to state.

Two guards were added alongside it, because asking git is not always possible.
The fallback walk now abandons a tree past 150,000 files rather than returning
an arbitrary prefix, and the default exclusions cover agent and build caches.
Neither is sufficient alone: no hand-maintained list stays ahead of whatever
tool a team installs next, which is why the cap exists as well.

## What is not built

- **CSS and Tailwind.** Value-frequency analysis, a different engine entirely.
- **Rewriting a write rather than refusing it.** `PreToolUse` accepts an
  `updatedInput`, and it works: a hook can replace the content before it is
  written. Measured, and deliberately not used. canon silently authoring
  someone's code on a derived rule is a worse failure than anything it would
  prevent.

## Two languages in one file

Vue and ERB are wired through `Parser::set_included_ranges`: the buffer is
parsed by the grammar of the language canon has conventions about, restricted
to the ranges that language occupies. Everything else is treated as absent, and
the tree keeps the original byte offsets.

```
<div>
  <% Payment.charge(1) %>      ->  parsed as Ruby
</div>                             the markup is not there as far as the parser
                                   is concerned
```

340 `.erb` files that were invisible now contribute. They yield few conventions
of their own, because a template declares expressions rather than types and
methods, and that is the honest outcome rather than a shortfall: canon measures
shape, and templates have little.

Every language canon knows is now wired. `canon check` still prints the tier
per language, because the next language added without a grammar has to say so.

## Facts by query rather than by hand

Call sites, raises and imports are matched by tree-sitter queries in
`crates/canon-extract/queries/<language>/facts.scm`. Adding a fact to a language
is editing an `.scm` file; the structural extractors stay in Rust because what
they resolve is *stateful* and a pattern cannot express it. Ruby's `private` is
a section keyword; Go's methods live outside the type they belong to.

The capture vocabulary is canon's, not upstream's. Every grammar crate exports a
`TAGS_QUERY` and they disagree with each other: Go's reports `@reference.type`
where Ruby's reports `@reference.call`, and TypeScript's covers only the
constructs TypeScript adds, on the assumption that the JavaScript query is
concatenated with it. Pointed at `class Foo { call() { charge(1) } }` the
TypeScript tags query returns nothing at all.

A file is parsed once and read twice. The extractors previously each parsed
their own tree, so a second pass would have parsed the file again.

What it buys is layering, which this document listed as unbuilt until now:

```
app/workers/workers/users/**/*.rb   Files here call `User`  (9/10, 0.90)
```

Who a directory talks to is a rule no linter checks and every team holds.

## Two more rules the real repository killed

**A collaborator has to be written like a type.** Unfiltered, the layering rule
produced "files here call `listing`", "call `response`", "call `user`". Those
are local variables. True of the code, and describing nothing anyone chose. The
filter took ten rules down to the two that are real.

**Absence of raising was built, measured, and deleted.** It produced six rules,
covering `spec/`, `vendor/`, `config/`, `db/` and the whole of `app/`, every one
arithmetic rather than a choice. A rule whose entire output on a 9,546-file
repository is noise does not ship, so it was removed rather than tuned.

## What the model can decline, measured

Advisory context is ignorable. The escalation above it was tested against the
running host rather than assumed, with a hook demanding a marker line the model
had no other reason to write:

| Channel | Result |
|---|---|
| `PreToolUse` `additionalContext` | reaches the model in time and steers the write |
| `PostToolUse` `decision: block` | reason delivered, hook fired once, turn ended anyway |
| `Stop` `decision: block` | turn genuinely held open three times, edit still not made |
| `PreToolUse` `updatedInput` | content rewritten before the write; the model has no say |
| `PreToolUse` `permissionDecision: deny` | the write does not happen |

Three of the five are persuasion and can be declined. Two are not.

canon uses `deny`, on by default, and only for a rule with total agreement and
an exact check. It does not use `updatedInput`, though it works: rewriting
someone's code from a rule derived by counting is a worse failure than the one
it would prevent.

The default is on because the alternative is a tool followed when convenient,
and because the condition for refusing makes a false positive a contradiction:
a rule only qualifies when every file in scope already agrees, so no existing
file can break one. Checked rather than argued — 20,718 tracked files from nine
production repositories, replayed through the write path, refused none, and
18,625 of them given a rule to check against.

The injection budget is not what bounds the block. Measured on the same
workspace, real paths spend 262 to 486 bytes of it, and raising it from 1,500
to 4,000 produced byte-identical output. What limits the block is how many
conventions exist for a path, which is what issue #7 is about.

Verified end to end. Asked directly for a violation — "two public methods named
`perform` and `also`, do not use a base class" — against a directory where six
of six files disagree, no file was written. The model stopped and offered the
suppression path the refusal names.

## Known limits

**Duplication detection is narrow on purpose.** It compares a written file
against siblings in the same directory with the same extension, not the whole
repository. The realistic failure is copying the file next door and editing
three lines. Being wrong about duplication is more insulting than being silent
about it, so the floor is set high: a ten-line service object shares its whole
shape with every sibling by design and is never flagged.

**A single-word file name proves nothing, and a repeated one proves less.**
`create` is valid `snake_case`, `kebab-case` and `camelCase` simultaneously, so
no naming rule is derived unless the sample contains a name that actually
distinguishes them. Ten copies of one name are one observation, so the sample
is counted in distinct names — the gate that stops `crates/*/Cargo.toml` from
deriving `PascalCase` and refusing a `deny.toml`.

**Style is read from the name root.** `charge_card.html.erb` is named
`charge_card`, `globals.d.ts` is named `globals`, `payments.service.ts` is
named `payments`. Reading up to the last dot instead made all three compatible
with no style at all, and one of them was enough to silence a whole directory.

**Old code still votes, at a discount.** Weight halves every 365 days by
default, floored at 5%. A directory nobody has touched in three years still
produces conventions; it just loses to current code where they disagree.

## What this cannot do, by construction

Derives *shape*, never *correctness*. It can tell you code resembles your other
code. It cannot tell you the code is right. It is not a substitute for review.
