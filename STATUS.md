# Measured status — v0.1.0

Every number below was executed. Reproduce with the commands shown.

## Gates

```
cargo build --release              0 errors
cargo test --workspace             257 passing
cargo clippy --workspace           0 warnings
cargo fmt --all --check            clean
./tests/fail-open.sh               75/75
./tests/injection-reaches-the-model.sh   PASS
```

7,353 lines of Rust across five crates, of which roughly half are tests.

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

What it derived for the Rails repository, unedited:

```
app/services       inherit from `ActiveInteraction::Base`  (1268/1550, 0.82)
app/services       expose exactly 1 public method          (1248/1550, 0.80)
app/services       that method is named `execute`          (1241/1248, 0.99)
app/workers        that method is named `perform`          (481/481,   1.00)
app/controllers/api/v1  inherit from `BaseController`
db/                that method is named `change`
repository-wide    test files are named `*_spec.rb`        (1321/1462, 0.90)
```

Those are correct, and none of them are written down anywhere in that
repository.

## Four defects the real repositories found that fixtures did not

Each of these passed a green test suite and was wrong in production.

**1. The walk found 498,419 files where git tracks 5,445.** A single tool's
cache directory held 909,661 of them. Indexing took 1 minute 47 seconds. No
hand-maintained exclude list keeps up with a working tree; the ignore rules the
team already wrote do. Now the file list comes from `git ls-files`, falling
back to the walk when there is no git. Indexing went to 2.4 s.

**2. `class Hubspot::EnrollSequence` was invisible to the Ruby extractor.** A
compound constant parses as `scope_resolution`, not `constant`, and the
extractor matched on node kind. So it skipped the real class in every namespaced
service and captured only the small `class SenderConfigurationError <
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

## What is not built

- **Vue SFC.** `<template>` and `<script lang="ts">` are separate grammars, so
  it needs two-pass extraction rather than one. Declared unwired in the
  capability table rather than half-wired.
- **CSS and Tailwind.** Value-frequency analysis, a different engine entirely.
- **Import-layering rules.** Imports are extracted and stored; nothing derives
  from them yet. "Files in `app/services` never import from `app/controllers`"
  is the obvious next rule.
- **Blocking enforcement.** The type exists and nothing constructs it. No rule
  derived by counting should ever refuse a write, and every rule here is
  derived by counting.

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
