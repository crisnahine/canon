# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
