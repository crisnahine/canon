# Measured status — v0.3.0

Every number below was executed. Reproduce with the commands shown.

## Gates

```
cargo build --release              0 errors
cargo test --workspace             274 passing
cargo clippy --workspace           0 warnings
cargo fmt --all --check            clean
./tests/fail-open.sh               75/75
./tests/asset-coverage.sh          8 of 8 platforms
./tests/injection-reaches-the-model.sh   PASS
```

8,035 lines of Rust across five crates, of which roughly half are tests,
plus 105 lines of tree-sitter query across seven languages.

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

It is a single file read and a filter over a 52 KB snapshot. No tree walk, no
parsing, no subprocess.

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
- **Vue and ERB.** Both are two grammars in one file. `Parser::set_included_ranges`
  is the mechanism and it is not wired yet, which leaves 339 `.erb` files
  invisible in the Rails repository measured above.
- **Rewriting a write rather than refusing it.** `PreToolUse` accepts an
  `updatedInput`, and it works: a hook can replace the content before it is
  written. Measured, and deliberately not used. canon silently authoring
  someone's code on a derived rule is a worse failure than anything it would
  prevent.

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

canon uses `deny`, and only for a rule with total agreement and an exact check.
It does not use `updatedInput`, though it works: rewriting someone's code from
a rule derived by counting is a worse failure than the one it would prevent.

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

**A single-word file name proves nothing.** `create` is valid `snake_case`,
`kebab-case` and `camelCase` simultaneously. A directory of single-word names is
compatible with every style and evidence for none, so no naming rule is derived
unless the sample contains a name that actually distinguishes them.

**Old code still votes, at a discount.** Weight halves every 365 days by
default, floored at 5%. A directory nobody has touched in three years still
produces conventions; it just loses to current code where they disagree.

## What this cannot do, by construction

Derives *shape*, never *correctness*. It can tell you code resembles your other
code. It cannot tell you the code is right. It is not a substitute for review.
