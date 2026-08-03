# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] — 2026-08-03

### Fixed

- A file that followed every convention exactly was reported for breaking two,
  and once those rules reached total agreement it was refused outright.
  Verifying judged every declared type while deriving resolved one, so a
  namespace module counted as a type with no base and no public methods. Both
  halves now resolve the subject the same way, in one place. (#1)
- The entrypoint-name rule was withheld from any type with more than one public
  method, so the files that broke it hardest were told the least, and fixing
  one file took two round trips. (#2)
- Moving the working directory into a nested repository silently switched to a
  snapshot that was never built, and every hook went quiet for the rest of the
  session. The snapshot is now found by searching upward, and a root with none
  says so once. (#3)
- `reconcile` compared a written file only against tracked files, so several
  near-identical files written in the same turn were invisible to each other —
  which is the case worth catching. The session's own ledger is now part of the
  index. (#4)
- Naming rules were derived for binary assets. Twelve of them on one workspace,
  including 2,912 `.jpg` files, each taking a slot in a 1,500-character budget.
  (#8)
- Five conventionally-named files at a repository root produced "files here are
  named in `SCREAMING_SNAKE_CASE`" for every `.md` in the project. (#8)
- Required agreement now rises with the size of the sample. The widest scopes
  were clearing the floor by the smallest margin and generating the most
  messages: one rule covered every `.rb` file in a Rails API at 0.81 and
  complained about a migration written the way the framework asks. (#8)

### Added

- A rule is stated at the parent when every child directory that has an opinion
  holds the same one. Agreement is counted over directories, not files, because
  a tree where twelve subdirectories each hold a rule without exception can
  still sit at 0.82 across its files. A new subdirectory now inherits the rule
  instead of getting nothing. (#6)
- `enforce`, on by default, with `CANON_ENFORCE` to turn it off. Refusing a
  write is the only channel a model cannot decline; the other three were each
  measured being declined, and a tool that only advises is followed when
  convenient. The safety is in what qualifies rather than in the switch: a rule
  may only refuse when every file in scope already agrees, which makes a false
  positive a contradiction rather than a risk. Replaying 400 tracked Ruby files
  from a production repository through the write path refused none. (#5)
- File-name casing can refuse, when enabled. It is the safest check there is:
  a string comparison on the path, with no parsing and no question about which
  type in the file is the subject. (#5)

### Changed

- The report after a write asks for the change rather than restating the
  policy. (#5)
- `SNAPSHOT_VERSION` is 3. A snapshot from before this release holds the
  over-broad rules, none of the rolled-up ones, and marks every convention
  advisory, so an upgraded install would refuse nothing until its commit
  changed.
- `injection_budget` is 3,000, up from 1,500. Headroom rather than a fix:
  measured on a 13,456-file workspace, real paths spend 262 to 486 bytes and
  raising the ceiling to 4,000 produced byte-identical output. What bounds the
  block is how many conventions exist for a path, not how many fit.



### Added

- canon can refuse a write. `Enforcement::Blocking` existed from the first
  release and nothing ever constructed it; it is now derived for the three
  rules whose check cannot be wrong about a legitimate file — public method
  count, entrypoint name, base type — and only where the repository agrees
  without a single exception.
- Stale state is swept on the cold path. A snapshot is written per repository
  and nothing removed it when that repository was deleted; a touched-file list
  is removed when a turn ends, and a session that is killed never ends.

### Why refusing, and not something gentler

Advisory context is ignorable, which was measured rather than assumed. A hook
demanded a marker line the model had no other reason to write:

- context before the write reaches the model in time and steers it
- a `PostToolUse` block delivered its reason and the turn ended anyway
- a `Stop` block held the turn open three times and the edit still never came
- refusing the write is the only channel that does not need the model to agree

`PreToolUse` also accepts an `updatedInput`, and it works: a hook can replace
the content before it is written. canon does not use it. Silently authoring
someone's code from a rule derived by counting is a worse failure than the one
it would prevent.

## [0.2.0] — 2026-08-03

### Added

- A declarative extraction layer. Call sites, raises and imports are matched by
  tree-sitter queries in `queries/<language>/facts.scm` rather than by another
  hand-rolled walk per language. Adding a fact to a language is now editing an
  `.scm` file.
- Layering conventions, listed as unbuilt until now: who a directory talks to.
  On a real worker directory, `Files here call ``User``. (9/10, 0.90)`. It is
  the kind of rule no linter checks and every team holds.
- The capture vocabulary is canon's and identical across languages. Every
  grammar crate exports a `TAGS_QUERY`, and they disagree: Go's reports
  `@reference.type` where Ruby's reports `@reference.call`, and TypeScript's
  covers only the constructs TypeScript adds, on the assumption the JavaScript
  query is concatenated with it.

### Changed

- `rust-version` is 1.97, the current release. canon ships prebuilt binaries for
  eight targets, so the floor only affects building from source, and the CI job
  that proves it moves with it.
- A file is parsed once and read twice. Extractors previously each parsed their
  own tree, so a second pass would have parsed the file again.
- `SNAPSHOT_VERSION` is 2. A snapshot built before the query layer holds no
  layering rules, so it is discarded rather than kept until the commit changes.
- `toml` moves from 0.9 to 1. Its error for an unknown key now lists the keys
  that are valid, which is the message `canon check` shows a human.

### Removed

- Absence-of-raising as a convention. It was built, measured against a
  9,546-file repository, and produced six rules covering `spec/`, `vendor/`,
  `config/`, `db/` and all of `app/` — every one arithmetic rather than a choice
  anyone made.

## [0.1.2] — 2026-08-03

### Fixed

- Windows on ARM had no binary at all, and neither did any musl system. The
  release built five of the eight triples the shim can compute, so those
  platforms got a 404 and then the fail-open path, which produces no symptom
  beyond a plugin that never speaks.
- The shim now asks the loader whether the C library is musl or glibc, instead
  of assuming glibc and handing Alpine something that cannot run.

### Added

- All eight targets are built natively rather than cross-compiled. The
  tree-sitter grammars are C, and a missing cross toolchain yields something
  unrunnable rather than a build failure. Each build job now runs what it built.
- A release smoke stage: every published asset is downloaded on its own
  platform, checked against `SHA256SUMS`, and executed. Building an artifact
  and publishing a working one are different claims.
- `tests/asset-coverage.sh`, which enumerates the shim's computable triples and
  the release's published assets from source and fails on any mismatch. That
  mismatch is what shipped a Windows plugin that did nothing, and it is
  structural rather than a typo, so it needed a test rather than vigilance.
- CI runs tests and the resolution shim on both architectures of all three
  platforms, the fail-open contract on all three, and the Windows batch wrapper
  through `cmd.exe`, which is the path the host actually takes there.

## [0.1.1] — 2026-08-03

### Fixed

- Windows never got a binary. The release publishes
  `canon-x86_64-pc-windows-msvc.exe`, and the resolution shim asked for the
  same name without the suffix, so the download 404'd. Had it succeeded it
  would have saved the file under a name Windows refuses to execute. The
  Windows wrapper had the mirror-image fault: it looked for `canon.exe` while
  the shim installs a versioned `canon-<version>.exe`, so the two never met.
  Every path failed open, which is why the only symptom was a plugin that
  silently did nothing.
- Installing a binary now removes the one it replaces, so the Windows wrapper's
  `canon-*` match is unambiguous.

### Added

- A `shim` CI job that runs the resolution shell under bash on Linux, macOS and
  Windows and requires it to find a binary. Nothing tested the shim before, and
  it is where every platform difference lives.
- A check that the asset name the shim requests matches the one the release
  workflow publishes, so the two cannot drift apart again.

## [0.1.0] — 2026-08-03

First release. A Claude Code plugin that derives a repository's conventions
from its own code and states them before each file is written.

### Added

- Five crates with boundaries that hold three properties: derivation rules are
  testable without a repository or a grammar, adding a language cannot reach
  those rules, and exactly two functions in the workspace may write to a
  standard stream.
- Tier 0 conventions from paths alone — naming style, test-file naming,
  canonical exemplars. Works on any text repository in any language.
- Tier 1 conventions from tree-sitter — public surface size, entrypoint name,
  base type, module export count. Wired for Ruby, JavaScript, JSX, TypeScript,
  TSX, Python, Go, Rust and PHP, spanning six different visibility models.
- `inject` on `PreToolUse`: the conventions for the file about to be written.
- `verify` on `PostToolUse`: how the written file differs from them.
- `reconcile` on `Stop`: near-duplicate detection across the turn's files.
- `subagent-start`: the same manifest into a worker with an empty context
  window, which is the reason this is a plugin and not a skill.
- `canon check`, which prints the capability table read from the binary, so
  documentation cannot claim a language the build does not link.
- `canon explain`, the audit surface: every rule with the files behind it, and
  suppression by id in `.canon.toml`.
- Layered configuration, file-only logging, and concrete error enums per crate.
- `tests/fail-open.sh`: 75 hostile-payload checks against the shipped binary,
  asserting exit 0, silence on stderr, and stdout empty or valid JSON.
- `tests/injection-reaches-the-model.sh`: verifies against the installed host
  that `PreToolUse` context reaches the model before the tool runs.
- CI across Linux, macOS and Windows, a minimum-supported-Rust job at 1.85, and
  a job asserting the manifests agree with the crate version.

### Notes from building it

Five defects survived a green test suite and were found only by pointing the
tool at production repositories and hostile input. They are recorded in
`STATUS.md` because each one is the reason a piece of the design looks the way
it does:

- The filesystem walk found 498,419 files where git tracks 5,445. The file list
  comes from `git ls-files` now.
- `class Foo::Bar` was invisible to the Ruby extractor, because a compound
  constant parses as `scope_resolution` and the extractor matched on node kind.
- Error classes and prop interfaces outvoted the type a file is actually about.
- Repository-wide shape rules let migrations speak for every Ruby file.
- The Ruby extractor recursed and overflowed the stack on a real file, aborting
  the process. It walks iteratively now.
